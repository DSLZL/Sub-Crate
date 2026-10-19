"""Loopback-only subscription, Gist and node-info fixtures with observable requests."""
import copy
import base64
import gzip
from artifact_fixtures import response as artifact_response
from dns_fixtures import response_bytes
from gist_fixtures import GistFixture
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
import json
import threading
import time
import zlib
from urllib.parse import parse_qs, quote, unquote, urlsplit


class ControlledUpstream:
    def __init__(self, port=0):
        self.requests = []
        self.gists = {}
        self.script_gist = GistFixture()
        self.gist_lock = threading.Lock()
        self.flow_lock = threading.Lock()
        self.flow_active = 0
        self.background_started = threading.Event()
        self.background_release = threading.Event()
        self.dns_condition = threading.Condition()
        self.dns_active = 0
        self.dns_pair_started = False
        self.dns_release = threading.Event()
        owner = self

        class Handler(BaseHTTPRequestHandler):
            def handle_request(self):
                length = int(self.headers.get("Content-Length", "0"))
                raw_body = self.rfile.read(length)
                body = raw_body.decode("utf-8", errors="replace" if self.path.startswith("/host/") else "strict")
                request = {"method": self.command, "path": self.path,
                           "headers": {key.lower(): value for key, value in self.headers.items()}, "body": body}
                owner.requests.append(request)
                target = urlsplit(self.path)
                if target.netloc and (target.scheme != "http" or target.netloc not in ("ip-api.com", "ip-api.com:80") or self.command != "GET"):
                    status, value = 502, {"message": "Fixture proxy forbids this authority"}
                    content = json.dumps(value).encode("utf-8")
                    content_type = "application/json"
                elif target.path.startswith("/host/"):
                    request["body_bytes"] = list(raw_body)
                    if target.path.startswith('/host/gist/'):
                        status, content, headers = owner.script_gist.response(self.command, self.path, body, self.headers, owner.url)
                    elif target.path.startswith('/host/dns/'):
                        status, content, headers = owner.dns_response(target, request)
                    else:
                        status, content, headers = owner.host_response(self.command, target.path, raw_body, self.headers)
                    self.send_response(status)
                    for key, value in headers:
                        self.send_header(key, value)
                    self.send_header("Content-Length", str(len(content)))
                    self.send_header("X-Fixture", "host-contract")
                    self.end_headers()
                    if self.command != "HEAD":
                        try:
                            self.wfile.write(content)
                        except (BrokenPipeError, ConnectionResetError):
                            pass  # A timed-out probe may close the owned connection.
                    return
                elif target.path.startswith("/json/"):
                    status, value = owner.node_info_response(self.path)
                    content = json.dumps(value, ensure_ascii=False).encode("utf-8")
                    content_type = "application/json"
                elif self.path.startswith(("/gists", "/gist-raw/")):
                    with owner.gist_lock:
                        status, value, raw = owner.gist_response(self.command, self.path, body, self.headers.get("Authorization"))
                    content = value.encode("utf-8") if raw else json.dumps(value, ensure_ascii=False).encode("utf-8")
                    content_type = "text/plain; charset=utf-8" if raw else "application/json"
                else:
                    status = 200 if self.path == "/subscription" else 404
                    content = b"ss://YWVzLTEyOC1nY206cGFzcw==@127.0.0.1:8388#fixture"
                    content_type = "text/plain; charset=utf-8"
                self.send_response(status)
                self.send_header("Content-Type", content_type)
                self.send_header("Content-Length", str(len(content)))
                self.send_header("subscription-userinfo", "upload=10; download=20; total=1000")
                self.end_headers()
                if self.command != "HEAD":
                    self.wfile.write(content)

            do_HEAD = handle_request
            do_GET = handle_request
            do_POST = handle_request
            do_PATCH = handle_request
            do_PUT = handle_request
            do_DELETE = handle_request
            do_OPTIONS = handle_request

            def log_message(self, *args):
                pass

        self.server = ThreadingHTTPServer(("127.0.0.1", port), Handler)
        self.url = f"http://127.0.0.1:{self.server.server_port}"
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)

    def dns_response(self, target, request):
        mode = target.path.removeprefix('/host/dns/')
        encoded = parse_qs(target.query)['dns'][0]
        packet = base64.urlsafe_b64decode(encoded + '=' * (-len(encoded) % 4))
        if mode == 'delay':
            time.sleep(0.2)
        if mode == 'release':
            self.dns_release.set()
        if mode in ('race-fast', 'race-slow'):
            with self.dns_condition:
                if self.dns_active == 0:
                    self.dns_pair_started = False
                    self.dns_release.clear()
                self.dns_active += 1
                if self.dns_active == 2:
                    self.dns_pair_started = True
                self.dns_condition.notify_all()
                # Both real HTTP requests must overlap before either can answer.
                request['dns_overlap'] = self.dns_condition.wait_for(lambda: self.dns_pair_started, timeout=2)
            if mode == 'race-slow':
                # The caller releases the loser only after observing the fast winner.
                self.dns_release.wait(timeout=3)
            with self.dns_condition:
                self.dns_active -= 1
        return (503 if mode == 'status' else 200), response_bytes(packet, mode), [('Content-Type', 'application/dns-message')]

    def host_response(self, method, path, body, headers):
        if path.startswith('/host/notification'):
            return (503 if path.startswith('/host/notification-error') else 200), b'{"ok":true}', [('Content-Type', 'application/json')]
        if path.startswith('/host/pipeline/'):
            name = path.removeprefix('/host/pipeline/')
            if name == 'failed':
                return 503, b'pipeline rejected', []
            if name == 'slow':
                time.sleep(0.04)
            content = f'ss://YWVzLTEyOC1nY206cGFzcw==@{name}.invalid:8388#remote-{name}'
            return 200, content.encode('utf-8'), []
        if path.startswith('/host/artifact/'):
            return artifact_response(path)
        text = "订阅🙂\r\nsecond\n".encode("utf-8")
        if path.startswith('/host/script/'):
            mode = path.removeprefix('/host/script/')
            if mode == 'missing':
                return 404, b'no script', []
            if mode == 'response':
                script = 'function transformFunction(res) { res.body += $arguments.text; return res; }'
            else:
                script = """async function operator(proxies, target, context) {
  await $substore.wait(1);
  for(const proxy of proxies) proxy.linked={arguments:$arguments,target,options:$options??null,raw:context.raw??null,source:context.source?.marker??null};
  return proxies;
}"""
            flow = 'upload=3; download=0; total=3' if mode == 'flow' else 'upload=1; download=2; total=100'
            return 200, script.encode('utf-8'), [('Subscription-Userinfo', flow)]
        if path.startswith('/host/download/'):
            mode = path.removeprefix('/host/download/')
            content = "下载🙂\r\n\0尾\n".encode('utf-8')
            if mode.startswith('age/'):
                vectors = json.loads((Path(__file__).parent / 'fixtures/age-native-vectors.json').read_text(encoding='utf-8'))
                pair = next((item for item in vectors if item['type'] == mode[4:]), vectors[0])
                return 200, pair['items'][1]['armor'].encode('ascii'), []
            if mode == 'text/background':
                self.background_started.set()
                self.background_release.wait(timeout=5)
            if mode.startswith('status/'):
                return int(mode.rsplit('/', 1)[-1]), b'provider rejected', []
            if mode == 'text/white':
                return 200, b' \t\r\n', []
            if mode == 'text/slow':
                time.sleep(0.2)
            if mode == 'text/preprocess-good':
                return 200, 'proxies:\n  - {name: 预处理, type: ss, server: example.invalid, port: 8388, cipher: aes-128-gcm, password: pass}\n'.encode('utf-8'), []
            if mode == 'text/preprocess-invalid':
                return 200, b'<!DOCTYPE html>no nodes', []
            flow = 'upload=3; download=0; total=3' if mode == 'text/flow-exhausted' else 'upload=1; download=2; total=100; expire=4102444800'
            return 200, content, [('Subscription-Userinfo', flow)]
        if path.startswith('/host/compressed/'):
            mode = path.removeprefix('/host/compressed/')
            if mode == 'too-many':
                return 200, b'', [('Content-Encoding', ', '.join(['gzip'] * 6))]
            encoded = gzip.compress(text, mtime=0)
            if mode == 'unknown':
                return 200, encoded, [('Content-Encoding', 'gzip, unknown')]
            if mode in ('deflate', 'compress', 'x-compress'):
                encoded = zlib.compress(text)
            elif mode == 'nested':
                encoded = zlib.compress(encoded)
            coding = 'gzip, deflate' if mode == 'nested' else ('gzip' if mode == 'status' else mode)
            return (503 if mode == 'status' else 200), encoded, [('Content-Encoding', coding)]
        if path.startswith("/host/flow/"):
            mode = path.removeprefix("/host/flow/")
            flow = [("Subscription-Userinfo", "upload=10; download=20; total=1000"),
                    ("Profile-Web-Page-Url", "https://example.invalid/flow"), ("Plan-Name", "Flow plan")]
            if mode.startswith("queued/"):
                with self.flow_lock:
                    self.flow_active += 1
                    active = self.flow_active
                try:
                    time.sleep(0.03)
                    return 200, b"", [("Subscription-Userinfo", "download=0; total=1"),
                                      ("Plan-Name", f"{mode.rsplit('/', 1)[-1]}-active-{active}")]
                finally:
                    with self.flow_lock:
                        self.flow_active -= 1
            if mode == "delay":
                time.sleep(0.2)
                return 200, b"download=1; total=2", []
            if mode in ("head", "get", "fallback", "badstatus"):
                return (503 if mode == "badstatus" else 200), b"invalid flow body", ([] if mode == "get" and method == "HEAD" else flow)
            if mode == "body":
                return 200, b" upload=30; download=40; total=2000 ", []
            if mode in ("rejected", "empty"):
                return (503 if mode == "rejected" else 200), b"no flow", []
            return 404, b"unexpected flow fixture", []
        if path == "/host/delay":
            time.sleep(0.2)
            return 200, text, []
        if path == "/host/text":
            return 200, text, [("Content-Type", "text/plain; charset=utf-8")]
        if path == "/host/echo":
            value = {"method": method, "body": list(body), "accept": headers.get("Accept"),
                     "probe": headers.get("X-Probe"), "authorization": headers.get("Authorization")}
            return 200, json.dumps(value, ensure_ascii=False).encode("utf-8"), [("Content-Type", "application/json")]
        if path == "/host/binary":
            return 200, bytes([0, 255, 195, 40, 13, 10]), [("Set-Cookie", "first=1"), ("Set-Cookie", "second=2")]
        if path == "/host/status":
            return 503, b"provider rejected", []
        if path.startswith("/host/redirect/"):
            remaining = int(path.rsplit("/", 1)[-1])
            return 302, b"", [("Location", f"/host/redirect/{remaining - 1}" if remaining > 1 else "/host/text")]
        return 404, b"unexpected host request", []

    def node_info_response(self, path):
        address = unquote(urlsplit(path).path.removeprefix("/json/"))
        if address == "rejected.invalid":
            return 200, {"status": "fail", "message": "fixture lookup rejected"}
        if address == "http-error.invalid":
            return 503, {"status": "success", "query": address, "countryCode": "ZZ"}
        if address not in ("1.2.3.4", "2001:db8::1"):
            return 404, {"status": "fail", "message": "fixture address missing"}
        return 200, {"status": "success", "query": address, "countryCode": "US", "city": "fixture-city", "lat": 0, "mobile": False}

    def gist_response(self, method, path, body, authorization):
        if authorization == "token rejected-fixture-token":
            return 403, {"message": "fixture credentials rejected"}, False
        parts = [unquote(part) for part in urlsplit(path).path.strip("/").split("/")]
        if parts[0] == "gist-raw" and method == "GET":
            content = self.gists.get(parts[1], {}).get("files", {}).get(parts[-1], {}).get("content")
            return (200, content, True) if content is not None else (404, {"message": "fixture file missing"}, False)
        if parts == ["gists"] and method == "GET":
            return 200, [self.gist_metadata(key) for key in self.gists], False
        if parts == ["gists"] and method == "POST":
            key = f"gist-{len(self.gists) + 1}"
            self.gists[key] = json.loads(body)
            return 201, self.gist_metadata(key), False
        if len(parts) == 2 and parts[0] == "gists" and parts[1] in self.gists:
            key = parts[1]
            if method == "PATCH":
                for filename, value in json.loads(body)["files"].items():
                    if value is None:
                        self.gists[key]["files"].pop(filename, None)
                    else:
                        self.gists[key]["files"][filename] = value
            if method in ("GET", "PATCH"):
                return 200, self.gist_metadata(key), False
        return 404, {"message": "fixture gist missing"}, False

    def gist_metadata(self, key):
        return {**copy.deepcopy(self.gists[key]), "id": key, "html_url": self.url + "/gists/" + key,
                "files": {name: {**value, "filename": name, "raw_url": self.url + "/gist-raw/" + key + "/raw/revision/" + quote(name, safe="")}
                          for name, value in self.gists[key]["files"].items()}}

    def __enter__(self):
        self.thread.start()
        return self

    def __exit__(self, *args):
        self.server.shutdown()
        self.server.server_close()
        self.thread.join(timeout=5)


def substitute(value, upstream_url):
    if isinstance(value, str):
        return value.replace("$UPSTREAM", upstream_url)
    if isinstance(value, list):
        return [substitute(item, upstream_url) for item in value]
    if isinstance(value, dict):
        return {key: substitute(item, upstream_url) for key, item in value.items()}
    return value
