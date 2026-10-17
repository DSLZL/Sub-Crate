"""Disposable Gist/Snippet providers; never contact a public service."""
import copy
import json
import threading
import time
from urllib.parse import quote, unquote, urlsplit

USER_AGENT = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_4) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/81.0.4044.141 Safari/537.36"


class GistFixture:
    def __init__(self):
        self.lock = threading.Lock()
        self.github = {}
        self.gitlab = {}
        self.active = 0

    def snapshot(self):
        with self.lock:
            return copy.deepcopy({"github": self.github, "gitlab": self.gitlab})

    def response(self, method, path, body, headers, upstream):
        parts = [unquote(part) for part in urlsplit(path).path.removeprefix('/host/gist/').split('/')]
        if parts == ['reset'] and method == 'POST':
            with self.lock:
                self.github.clear()
                self.gitlab.clear()
            return 200, b'{}', []
        provider = parts[0]
        expected = 'helper-token'
        if provider == 'gitlab':
            valid = headers.get('PRIVATE-TOKEN') == expected and headers.get('Authorization') is None
        else:
            valid = headers.get('Authorization') == 'token ' + expected and headers.get('PRIVATE-TOKEN') is None
        if not valid:
            return 403, b'{"message":"fixture credentials rejected"}', []
        if provider == 'queue':
            with self.lock:
                self.active += 1
                active = self.active
            try:
                time.sleep(0.03)
                value = {'message': 'queue rejected'} if parts[1] == 'rejected' else {'name': parts[1], 'active': active}
                return (503 if parts[1] == 'rejected' else 200), json.dumps(value).encode(), []
            finally:
                with self.lock:
                    self.active -= 1
        if provider == 'errors':
            errors = {
                'nested': (400, b'{"message":{"error":"nested"},"error":"ignored"}'),
                'error': (401, b'{"message":"ignored","error":"second"}'),
                'message': (429, b'{"message":"third"}'),
                'text': (503, b'raw fixture error'),
                'empty': (404, b''),
                'object': (500, b'{"message":{"code":1}}'),
                'invalid': (200, b'not json'),
                'delay': (200, b'[]'),
            }
            if parts[1] == 'delay':
                time.sleep(0.2)
            status, content = errors[parts[1]]
            return status, content, []
        with self.lock:
            status, value, raw = self.provider_response(provider, method, parts[1:], body, upstream)
        content = value.encode('utf-8') if raw else json.dumps(value, ensure_ascii=False).encode('utf-8')
        return status, content, [('Content-Type', 'text/plain; charset=utf-8' if raw else 'application/json')]

    def provider_response(self, provider, method, parts, body, upstream):
        if provider not in ('github', 'gitlab'):
            return 404, {'message': 'unknown fixture provider'}, False
        store = self.github if provider == 'github' else self.gitlab
        resource = 'gists' if provider == 'github' else 'snippets'

        def metadata(key):
            item = copy.deepcopy(store[key])
            item['id'] = key
            if provider == 'github':
                item['files'] = {name: {**file, 'filename': name, 'raw_url': upstream + '/host/gist/github/raw/' + key + '/' + quote(name, safe='')}
                                 for name, file in item['files'].items()}
            else:
                item['files'] = [{'path': name} for name in item['files']]
            return item

        if parts == [resource] and method == 'GET':
            return 200, [metadata(key) for key in store], False
        if parts == [resource] and method == 'POST':
            data = json.loads(body)
            key = ('g' if provider == 'github' else 's') + str(len(store) + 1)
            if provider == 'gitlab':
                data['files'] = {item['file_path']: {'content': item['content']} for item in data['files']}
            store[key] = data
            return 201, metadata(key), False
        if len(parts) == 2 and parts[0] == resource and parts[1] in store:
            key = parts[1]
            if method == ('PATCH' if provider == 'github' else 'PUT'):
                files = json.loads(body)['files']
                if provider == 'github':
                    for name, value in files.items():
                        if value is None:
                            store[key]['files'].pop(name, None)
                        else:
                            store[key]['files'][name] = value
                else:
                    for action in files:
                        if action['action'] == 'delete':
                            store[key]['files'].pop(action['file_path'], None)
                        else:
                            store[key]['files'][action['file_path']] = {'content': action['content']}
            if method in ('GET', 'PATCH', 'PUT'):
                return 200, metadata(key), False
        if len(parts) == 3 and parts[0] == 'raw' and method == 'GET':
            value = store.get(parts[1], {}).get('files', {}).get(parts[2], {}).get('content')
            if value is not None:
                return 200, value, True
        return 404, {'message': 'fixture resource missing'}, False


def check_gist_requests(requests):
    from runner import check_json
    keys = ('authorization', 'private-token', 'user-agent', 'accept', 'content-type', 'accept-encoding', 'x-gist', 'x-gist-event')
    actual = [{"method": item['method'], "path": item['path'], "body_bytes": item['body_bytes'],
               "headers": {key: item['headers'][key] for key in keys if key in item['headers']}} for item in requests]
    check_json(actual, expected_gist_requests())


def expected_gist_requests():
    expected = []

    def request(path, method='GET', body=None, patch=False, gitlab=False, override=False):
        headers = {'accept': '*/*', 'user-agent': USER_AGENT,
                   **({'private-token': 'helper-token'} if gitlab else {'authorization': 'token helper-token'})}
        if path == 'reset':
            headers = {'accept': '*/*'}
        if patch:
            headers['accept-encoding'] = 'gzip, deflate, br'
        if gitlab and method in ('POST', 'PUT'):
            headers['content-type'] = 'application/json'
        if override:
            headers.update({'user-agent': 'Gist override/1', 'x-gist': 'yes', 'x-gist-event': 'mutated'})
        expected.append({'method': method, 'path': '/host/gist/' + path,
                         'body_bytes': [] if body is None else list(json.dumps(body, ensure_ascii=False, separators=(',', ':')).encode()),
                         'headers': headers})

    def locate(gitlab=False):
        request('gitlab/snippets' if gitlab else 'github/gists?per_page=100&page=1', gitlab=gitlab)

    def update(files, gitlab=False):
        locate(gitlab)
        request('gitlab/snippets/s1' if gitlab else 'github/gists/g2', 'PUT' if gitlab else 'PATCH',
                {'files': files}, patch=not gitlab, gitlab=gitlab)

    request('reset', 'POST')
    locate()
    locate()
    locate()
    request('github/gists', 'POST', {'description': 'other', 'public': False, 'files': {'other': {'content': 'other'}}})
    locate()
    request('github/gists', 'POST', {'description': 'helper', 'public': False,
                                   'files': {'订阅🙂.txt': {'content': '订阅🙂\r\nsecond\n'}, 'remove.txt': {'content': 'remove'}}})
    locate()
    request('github/gists/g2')
    request('github/raw/g2/' + quote('订阅🙂.txt', safe=''))
    update({'订阅🙂.txt': {'content': 'changed'}, 'remove.txt': None, 'zero': {'content': '0'}})
    update({'订阅🙂.txt': None, 'zero': None, 'keep': {'content': 'placeholder'}})
    update({})
    update({'real': {'content': 'restored'}, 'keep': None})
    for _ in range(4):
        locate()
    for mode in ('nested', 'error', 'message', 'text', 'empty', 'object'):
        request('errors/' + mode)
    request('errors/invalid/gists?per_page=100&page=1')
    request('errors/delay')
    locate()
    for method in ('get', 'post', 'put', 'patch', 'delete'):
        request('queue/' + method, method.upper())
    request('queue/rejected')
    request('queue/recovery')
    locate(True)
    locate(True)
    request('gitlab/snippets', 'POST', {'title': 'snippet', 'visibility': 'private', 'files': [
        {'file_path': 'alpha', 'content': '一🙂'}, {'file_path': 'gone', 'content': 'delete'}]}, gitlab=True)
    update([{'action': 'update', 'file_path': 'alpha', 'content': 'changed'},
            {'action': 'create', 'file_path': 'beta', 'content': 'second'}, {'action': 'delete', 'file_path': 'gone'}], True)
    for _ in range(3):
        locate(True)
    update([{'action': 'delete', 'file_path': 'alpha'}, {'action': 'delete', 'file_path': 'beta'},
            {'action': 'create', 'file_path': 'keep', 'content': 'placeholder'}], True)
    update([], True)
    update([{'action': 'create', 'file_path': 'real', 'content': 'restored'}, {'action': 'delete', 'file_path': 'keep'}], True)
    locate(True)
    request('gitlab/gists/s1', gitlab=True)
    locate(True)
    locate()
    request('github/gists/g2')
    request('github/raw/g2/real')
    request('github/gists?per_page=100&page=1', override=True)
    locate()
    return expected


def check_gist_state(state):
    from runner import check_json
    check_json(state, {
        'github': {'g1': {'description': 'other', 'public': False, 'files': {'other': {'content': 'other'}}},
                   'g2': {'description': 'helper', 'public': False, 'files': {'real': {'content': 'restored'}}}},
        'gitlab': {'s1': {'title': 'snippet', 'visibility': 'private', 'files': {'real': {'content': 'restored'}}}},
    })
