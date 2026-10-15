"""Owned HG10 server, fresh Node/Rust processes and independently checked files."""
import hashlib
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import socket
import threading
import time
from urllib.parse import urlsplit

from reference import HERE, ROOT, execute
from runner import check_json

TEXT = '下载🙂\r\n\0尾\n'
SMALL = bytes([0, 255, 128, 13, 10, 65])
PATTERN = bytes(range(256)) * 1024 + bytes([0, 1, 2])
DOWNLOAD_CASES = [
    'DL01-defaults-unicode-raw-and-regular-cache',
    'DL02-settings-headers-and-fragment-arguments',
    'DL03-github-proxy-pattern-and-invalid-regex',
    'DL04-native-local-utf8-empty-and-read-errors',
    'DL05-internal-modules-typed-content-and-missing-sources',
    'DL06-cache-ttl-inclusive-expiry-and-bypass',
    'DL07-cache-threshold-and-custom-cache-write-eligibility',
    'DL08-optimistic-cache-awaited-refresh-and-error-fallback',
    'DL09-flow-header-cache-ttl-and-validation-bypass',
    'DL10-status-empty-timeout-and-queue-recovery',
    'DL11-both-age-types-raw-cache-key-precedence-and-redaction',
    'DL12-public-helpers-in-async-script-and-persisted-values',
    'DL13-binary-status-redirect-limits-and-existing-file-retention',
    'DL14-binary-stream-visible-before-finish-and-partial-on-error',
    'DL15-binary-filesystem-errors-and-recovery',
]
EXPECTED_FILES = {'local🙂.txt': TEXT.encode(), 'empty.txt': b'', 'invalid-utf8.txt': b'a\xf0(\x8c(z',
                  'script🙂.bin': SMALL, 'binary🙂.bin': SMALL, 'complete.bin': PATTERN,
                  'broken.bin': PATTERN[:16384], 'zero.bin': b'', 'recovery.bin': SMALL}


class DownloadUpstream:
    def __init__(self, keys):
        self.keys = keys
        self.reset()
        owner = self

        class Handler(BaseHTTPRequestHandler):
            def log_message(self, *_):
                pass

            def do_GET(self):
                self.respond()

            def do_POST(self):
                self.respond()

            def respond(self):
                raw = self.rfile.read(int(self.headers.get('Content-Length', '0')))
                owner.requests.append({'method': self.command, 'path': self.path,
                    'headers': {key.lower(): value for key, value in self.headers.items()}, 'body_bytes': list(raw)})
                path = urlsplit(self.path).path
                if path.startswith('/stream/'):
                    mode = path.removeprefix('/stream/')
                    self.send_response(200)
                    self.send_header('Content-Length', str(len(PATTERN)))
                    self.end_headers()
                    self.wfile.write(PATTERN[:16384])
                    self.wfile.flush()
                    owner.events.append({'mode': mode, 'event': 'prefix-sent'})
                    if not owner.release[mode].wait(10):
                        owner.errors.append('stream release timed out: ' + mode)
                    elif mode == 'complete':
                        self.wfile.write(PATTERN[16384:])
                        self.wfile.flush()
                    owner.events.append({'mode': mode, 'event': 'closed'})
                    self.close_connection = True
                    self.connection.shutdown(socket.SHUT_WR)
                    return
                status, body, headers = 200, TEXT.encode(), []
                if path.startswith('/release/'):
                    mode = path.removeprefix('/release/')
                    owner.events.append({'mode': mode, 'event': 'released'})
                    owner.release[mode].set()
                    body = b'ok'
                elif path.startswith('/status/'):
                    status = int(path.removeprefix('/status/'))
                    body = b'status fixture'
                elif path.startswith('/redirect/'):
                    count = int(path.removeprefix('/redirect/'))
                    if count:
                        status, body, headers = 302, b'', [('Location', '/redirect/' + str(count - 1))]
                    else:
                        body = SMALL
                elif path == '/loop':
                    status, body, headers = 302, b'', [('Location', '/loop')]
                elif path == '/file/small':
                    body = SMALL
                elif path == '/file/empty':
                    body = b''
                elif path.startswith('/age/'):
                    body = next(item['armor'].encode() for item in owner.keys if path == '/age/' + item['type'])
                elif path == '/text/white':
                    body = b' \t\r\n '
                elif path == '/text/slow':
                    time.sleep(0.12)
                elif path in ('/text/flow', '/text/flow-options', '/text/flow-valid', '/text/flow-exhausted'):
                    total = 2 if path.endswith('exhausted') else 100
                    headers = [('subscription-userinfo', f'upload=1; download=2; total={total}; expire=4102444800')]
                elif not (path.startswith('/text/') or path == '/proxy/' + owner.url + '/text/proxied'):
                    status, body = 404, b'unknown download fixture'
                self.send_response(status)
                for key, value in headers:
                    self.send_header(key, value)
                self.send_header('Content-Length', str(len(body)))
                self.end_headers()
                try:
                    self.wfile.write(body)
                except (BrokenPipeError, ConnectionResetError):
                    pass  # The timeout case intentionally abandons its owned response.

        self.server = ThreadingHTTPServer(('127.0.0.1', 0), Handler)
        self.url = 'http://127.0.0.1:' + str(self.server.server_port)

    def reset(self):
        self.requests, self.events, self.errors = [], [], []
        self.release = {mode: threading.Event() for mode in ('complete', 'broken')}

    def __enter__(self):
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)
        self.thread.start()
        return self

    def __exit__(self, *_):
        for event in self.release.values():
            event.set()
        self.server.shutdown()
        self.server.server_close()
        self.thread.join()


def expected_wire(base):
    default = {'accept': '*/*', 'user-agent': 'clash.meta/v1.19.23'}
    requests = []

    def add(path, headers=default, method='GET'):
        requests.append({'method': method, 'path': path, 'headers': headers, 'body_bytes': []})

    add('/text/default')
    add('/text/settings', {**default, 'user-agent': 'download-default'})
    for path in ('/text/headers', '/text/pairs'):
        add(path, {**default, 'user-agent': 'download-custom', 'x-download-token': 'token-2'})
    add('/text/invalid-headers', {**default, 'user-agent': 'download-default'})
    add('/proxy/' + base + '/text/proxied')
    add('/text/invalid-regex')
    for _ in range(4):
        add('/text/ttl')
    for _ in range(2):
        add('/text/threshold')
    for path in ('/text/optimistic', '/status/503', '/text/flow', '/text/flow-options', '/text/flow-valid', '/text/flow-exhausted',
                 '/status/404', '/text/white', '/text/slow', '/text/recovered',
                 '/age/x25519', '/status/500', '/age/mlkem768-x25519', '/status/500', '/text/reload'):
        add(path)
    for path in ('/file/small', '/file/small', '/redirect/2', '/redirect/1', '/redirect/0',
                 '/status/201', '/status/404', '/redirect/3', '/redirect/2', '/redirect/1', '/redirect/0', '/loop'):
        add(path, {})
    for mode in ('complete', 'broken'):
        add('/stream/' + mode, {})
        add('/release/' + mode, {'accept': '*/*'}, 'POST')
    for path in ('/file/small', '/file/small', '/file/empty', '/file/small'):
        add(path, {})
    return requests


def check_download_wire(requests, base):
    exact = []
    for item in requests:
        # Only transport-managed Host/Connection/zero Content-Length may differ.
        headers = item['headers']
        check_json(headers['host'], base.removeprefix('http://'))
        if 'content-length' in headers:
            check_json(headers['content-length'], '0')
        if 'connection' in headers:
            if headers['connection'].lower() not in ('keep-alive', 'close'):
                raise AssertionError('Unexpected Connection header')
        exact.append({**item, 'headers': {key: value for key, value in headers.items()
                                         if key not in ('host', 'connection', 'content-length')}})
    check_json(exact, expected_wire(base))


def check_download_run(result, progress, resume=False):
    if resume:
        check_json(sorted(result), ['headersCache', 'resourceCache', 'storage'])
    else:
        check_json(result['passed'], DOWNLOAD_CASES)
        check_json(list(result['observations']), DOWNLOAD_CASES)
    check_json(progress, [{'id': case, 'event': event} for case in ([] if resume else DOWNLOAD_CASES)
                          for event in ('start', 'end')])


def file_snapshot(state):
    expected_names = sorted([*EXPECTED_FILES, 'sub-store.json', 'sub-store-probe.json', 'root.json', 'download-input.json'])
    check_json(sorted(path.name for path in state.iterdir()), expected_names)
    snapshot = {}
    for name, expected in EXPECTED_FILES.items():
        actual = (state / name).read_bytes()
        if actual != expected:
            raise AssertionError('Downloaded file bytes differ: ' + name)
        snapshot[name] = {'length': len(actual), 'sha256': hashlib.sha256(actual).hexdigest()}
    return snapshot


def check_download_state(result, persisted):
    check_json(result['storage'], persisted)
    check_json(result['resourceCache'], json.loads(persisted['root']['sub-store-cached-resource']))
    check_json(result['headersCache'], json.loads(persisted['root']['sub-store-cached-headers-resource']))
    check_json(persisted['cache']['downloadHelper'], {'text': TEXT, 'zero': 0, 'flag': False})


def run_download_probe(node, executable, environment, output, report, container=None):
    report['download_runs'] = []
    prepare = output / 'download-prepare-state'
    prepare.mkdir()
    child_env = {**environment, 'SUB_STORE_COMPAT_STATE_DIR': str(prepare), 'SUB_STORE_DATA_BASE_PATH': str(prepare)}
    process = execute([node, '--require', HERE / 'register.cjs', HERE / 'download-reference.cjs', '--prepare'],
                      ROOT / 'backend', child_env, output, 'download-prepare', timeout=30,
                      direct_reason='Direct oracle ownership allows watchdog cleanup on Windows')
    if process.returncode:
        raise RuntimeError('Download age fixture preparation failed')
    input_bytes = (prepare / 'download-input.json').read_bytes()
    with DownloadUpstream(json.loads(input_bytes)['keys']) as upstream:
        report['download_upstream'] = upstream.url
        for kind in ('node', 'rust'):
            upstream.reset()
            state = output / (kind + '-download-state')
            state.mkdir()
            for name in ('sub-store.json', 'sub-store-probe.json', 'root.json'):
                (state / name).write_text('{}\n', encoding='utf-8')
            (state / 'download-input.json').write_bytes(input_bytes)
            for name in ('local🙂.txt', 'empty.txt', 'invalid-utf8.txt'):
                (state / name).write_bytes(EXPECTED_FILES[name])
            for resume in (False, True):
                name = kind + '-download' + ('-resume' if resume else '')
                child_env = {**environment, 'SUB_STORE_COMPAT_STATE_DIR': str(state), 'SUB_STORE_DATA_BASE_PATH': str(state),
                             'SUB_STORE_COMPAT_UPSTREAM': upstream.url}
                before = len(upstream.requests)
                if kind == 'rust' and container:
                    process = container(state, upstream.url, ['--download-resume' if resume else '--download'], child_env, name)
                else:
                    argv = ([node, '--require', HERE / 'register.cjs', HERE / 'download-reference.cjs', *(['--resume'] if resume else [])]
                            if kind == 'node' else [executable, '--download-resume' if resume else '--download'])
                    process = execute(argv, ROOT / 'backend' if kind == 'node' else ROOT, child_env, output, name, timeout=60,
                                      direct_reason='Direct download process ownership allows watchdog cleanup on Windows')
                item = {'name': name, 'exit_code': process.returncode, 'requests': upstream.requests[before:],
                        'progress': [json.loads(line.removeprefix('COMPAT_DOWNLOAD ')) for line in process.stderr.splitlines()
                                     if line.startswith('COMPAT_DOWNLOAD ')]}
                report['download_runs'].append(item)
                (output / (name + '-wire.json')).write_text(json.dumps(item['requests'], indent=2) + '\n', encoding='utf-8')
                if process.returncode:
                    raise RuntimeError(name + ' failed; inspect preserved logs')
                item['result'] = json.loads(next(line.removeprefix('COMPAT_DOWNLOAD_RESULT ') for line in process.stdout.splitlines()
                                           if line.startswith('COMPAT_DOWNLOAD_RESULT ')) if kind == 'node' else process.stdout.splitlines()[-1])
                check_download_run(item['result'], item['progress'], resume)
                if resume:
                    check_json(item['requests'], [])
                    first = report['download_runs'][-2]['result']
                    check_json(item['result'], {key: first[key] for key in ('storage', 'resourceCache', 'headersCache')})
                else:
                    check_download_wire(item['requests'], upstream.url)
                item['files'] = file_snapshot(state)
                item['stream_events'] = upstream.events[:]
                check_json(upstream.errors, [])
                item['input_sha256'] = hashlib.sha256(input_bytes).hexdigest()
    report['download_artifact_check'] = check_download_artifacts(output, report)


def check_download_artifacts(directory, report):
    runs = report['download_runs']
    names = ['node-download', 'node-download-resume', 'rust-download', 'rust-download-resume']
    check_json([item['name'] for item in runs], names)
    prepare = directory / 'download-prepare-state/download-input.json'
    original_input = prepare.read_bytes()
    expected_events = [{'mode': mode, 'event': event} for mode in ('complete', 'broken')
                       for event in ('prefix-sent', 'released', 'closed')]
    for index, item in enumerate(runs):
        name = item['name']
        state = directory / (name.removesuffix('-resume') + '-state')
        command = json.loads((directory / (name + '.command.json')).read_text(encoding='utf-8'))
        check_json([command['exit_code'], command['timed_out'], item['exit_code']], [0, False, 0])
        stderr = (directory / (name + '.stderr.log')).read_text(encoding='utf-8')
        stdout = (directory / (name + '.stdout.log')).read_text(encoding='utf-8')
        progress = [json.loads(line.removeprefix('COMPAT_DOWNLOAD ')) for line in stderr.splitlines() if line.startswith('COMPAT_DOWNLOAD ')]
        check_json(item['progress'], progress)
        result = json.loads(next(line.removeprefix('COMPAT_DOWNLOAD_RESULT ') for line in stdout.splitlines()
                                 if line.startswith('COMPAT_DOWNLOAD_RESULT ')) if name.startswith('node') else stdout.splitlines()[-1])
        check_json(item['result'], result)
        check_download_run(result, progress, index % 2 == 1)
        wire = json.loads((directory / (name + '-wire.json')).read_text(encoding='utf-8'))
        check_json(item['requests'], wire)
        check_download_wire(wire, report['download_upstream']) if index % 2 == 0 else check_json(wire, [])
        check_json(item['files'], file_snapshot(state))
        check_json(item['stream_events'], expected_events)
        check_json(item['input_sha256'], hashlib.sha256(original_input).hexdigest())
        if (state / 'download-input.json').read_bytes() != original_input:
            raise AssertionError('Age input changed between download processes')
        persisted = {key: json.loads((state / filename).read_text(encoding='utf-8'))
                     for key, filename in [('cache', 'sub-store.json'), ('root', 'root.json')]}
        check_download_state(result, persisted)
        for key in json.loads(original_input)['keys']:
            if key['age-secret-key'] in stdout + stderr:
                raise AssertionError('Fixture age secret leaked into download logs')
    check_json(runs[0]['result'], runs[2]['result'])
    check_json(runs[1]['result'], runs[3]['result'])
    prepare_command = json.loads((directory / 'download-prepare.command.json').read_text(encoding='utf-8'))
    check_json([prepare_command['exit_code'], prepare_command['timed_out']], [0, False])
    return {'groups': len(DOWNLOAD_CASES), 'fresh_processes': 4, 'http_requests_per_runtime': len(expected_wire(report['download_upstream'])),
            'files_per_runtime': len(EXPECTED_FILES), 'progress_events': len(DOWNLOAD_CASES) * 4,
            'stream_prefix_bytes': 16384, 'complete_bytes': len(PATTERN), 'partial_error_bytes': 16384,
            'age_types': ['x25519', 'mlkem768-x25519']}
