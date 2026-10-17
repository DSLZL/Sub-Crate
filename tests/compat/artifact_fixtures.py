"""Controlled artifact sources and independently checked request contracts."""
import json
import time

URI = 'ss://YWVzLTEyOC1nY206c2VjcmV0@ss.example.com:8388#remote'
CLASH = 'proxies:\n  - name: remote🙂\n    type: ss\n    server: ss.example.com\n    port: 8388\n    cipher: aes-128-gcm\n    password: secret\n'
TEXT = '远程文件🙂\r\nline\n'
SCRIPT = 'function operator(proxies) { return proxies.map(p => ({...p, name: p.name + "-" + $arguments.suffix})); }'


def response(path):
    mode = path.removeprefix('/host/artifact/')
    if mode == 'slow':
        time.sleep(0.05)
    bodies = {'sub': URI, 'slow': URI.replace('#remote', '#slow'), 'clash': CLASH,
              'invalid': 'not a subscription', 'file': TEXT, 'script': SCRIPT, 'bad': 'artifact upstream rejected'}
    return (503 if mode == 'bad' else 200), bodies[mode].encode(), [('Content-Type', 'text/plain; charset=utf-8')]


def expected_requests():
    default = 'clash.meta/v1.19.23'
    # Collection requests and file sources are deliberately concurrent pairs.
    paths = [('script', default), ('clash', default), ('invalid', default), ('invalid', default),
             ('sub', 'Artifact Override/1'), ('sub', 'Artifact Override/1'),
             ('slow', 'Artifact Pass/1'), ('sub', 'Artifact Fixed/1'),
             ('file', default), ('bad', default),
             ('slow', default), ('sub', 'Artifact Fixed/1'),
             ('file', default), ('sub', default)]
    return [{'method': 'GET', 'path': '/host/artifact/' + path, 'body_bytes': [],
             'headers': {'user-agent': ua, 'accept': '*/*'}} for path, ua in paths]


def check_requests(requests):
    from runner import check_json
    actual = [{'method': item['method'], 'path': item['path'], 'body_bytes': item['body_bytes'],
               'headers': {key: value for key, value in item['headers'].items() if key not in ('host', 'connection', 'accept-encoding', 'accept-language', 'sec-fetch-mode')}} for item in requests]
    expected = expected_requests()
    for start, end in ((6, 8), (8, 10), (10, 12)):
        actual[start:end] = sorted(actual[start:end], key=lambda item: item['path'])
        expected[start:end] = sorted(expected[start:end], key=lambda item: item['path'])
    check_json(actual, expected)


def check_state(result, persisted):
    from runner import check_json
    check_json(result['storage'], persisted)
    check_json(persisted['cache']['artifactHelper'], {'text': TEXT + '|remote', 'zero': 0, 'flag': False})
    for field, key in (('resourceCache', 'sub-store-cached-script-resource'), ('headersCache', 'sub-store-cached-headers-resource'), ('dnsCache', 'sub-store-cached-resource')):
        check_json(result[field], json.loads(persisted['root'][key]))
