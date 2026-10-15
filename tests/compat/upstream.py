"""Loopback-only subscription, Gist and node-info fixtures with observable requests."""
import copy
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import threading
from urllib.parse import quote, unquote, urlsplit


class ControlledUpstream:
    def __init__(self, port=0):
        self.requests = []
        self.gists = {}
        self.gist_lock = threading.Lock()
        owner = self

        class Handler(BaseHTTPRequestHandler):
            def handle_request(self):
                length = int(self.headers.get("Content-Length", "0"))
                body = self.rfile.read(length).decode("utf-8")
                owner.requests.append({"method": self.command, "path": self.path,
                                       "headers": {key.lower(): value for key, value in self.headers.items()}, "body": body})
                target = urlsplit(self.path)
                if target.netloc and (target.scheme != "http" or target.netloc not in ("ip-api.com", "ip-api.com:80") or self.command != "GET"):
                    status, value = 502, {"message": "Fixture proxy forbids this authority"}
                    content = json.dumps(value).encode("utf-8")
                    content_type = "application/json"
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

            def log_message(self, *args):
                pass

        self.server = ThreadingHTTPServer(("127.0.0.1", port), Handler)
        self.url = f"http://127.0.0.1:{self.server.server_port}"
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)

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
