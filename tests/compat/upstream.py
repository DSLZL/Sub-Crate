"""Loopback-only subscription fixture with observable requests."""
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import threading


class ControlledUpstream:
    def __init__(self):
        self.requests = []
        owner = self

        class Handler(BaseHTTPRequestHandler):
            def handle_request(self):
                length = int(self.headers.get("Content-Length", "0"))
                body = self.rfile.read(length).decode("utf-8")
                owner.requests.append({"method": self.command, "path": self.path,
                                       "headers": {key.lower(): value for key, value in self.headers.items()}, "body": body})
                content = b"ss://YWVzLTEyOC1nY206cGFzcw==@127.0.0.1:8388#fixture"
                self.send_response(200 if self.path == "/subscription" else 404)
                self.send_header("Content-Type", "text/plain; charset=utf-8")
                self.send_header("Content-Length", str(len(content)))
                self.send_header("subscription-userinfo", "upload=10; download=20; total=1000")
                self.end_headers()
                if self.command != "HEAD":
                    self.wfile.write(content)

            do_HEAD = handle_request
            do_GET = handle_request

            def log_message(self, *args):
                pass

        self.server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        self.url = f"http://127.0.0.1:{self.server.server_port}"
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)

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
