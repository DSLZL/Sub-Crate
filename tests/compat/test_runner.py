import copy
from contextlib import redirect_stdout
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import io
import json
from pathlib import Path
import tempfile
import threading
import unittest
from unittest.mock import patch

import runner


class RunnerTests(unittest.TestCase):
    def test_command_evidence_with_and_without_rtk(self):
        import reference
        import subprocess
        for rtk in (None, "rtk.exe"):
            with self.subTest(rtk=rtk), tempfile.TemporaryDirectory() as directory:
                with patch("reference.shutil.which", return_value=rtk), patch("reference.subprocess.run", return_value=subprocess.CompletedProcess([], 0, "ok", "")) as run:
                    reference.execute(["node", "--version"], directory, {"PATH": "fixture"}, Path(directory), "version")
                command = json.loads((Path(directory) / "version.command.json").read_text(encoding="utf-8"))
                self.assertEqual(run.call_args.args[0], ([rtk, "proxy"] if rtk else []) + ["node", "--version"])
                self.assertEqual(command["rtk_exception"], None if rtk else "RTK unavailable on this runner")
                self.assertEqual((Path(directory) / "version.stdout.log").read_text(encoding="utf-8"), "ok")

    def test_status_content_required_field_and_state_mismatch(self):
        valid = {"status": 200, "headers": {}, "body": '{"status":"success","data":{"flag":false}}'}
        for expected in ({"status": 201}, {"status": 200, "body": "wrong"},
                         {"status": 200, "json_contains": {"missing": 1}},
                         {"status": 200, "json_contains": {"data": {"flag": 0}}}):
            with self.subTest(expected=expected), self.assertRaises(AssertionError):
                runner.check_response(valid, expected)
        with self.assertRaises(AssertionError):
            runner.check_json({"subs": []}, {"subs": [{"name": "must-persist"}]}, subset=True)

    def run_cli(self, fixtures, manifest, url, state_dir):
        fixtures_path = state_dir / "cases.json"
        manifest_path = state_dir / "manifest.json"
        evidence = state_dir / "evidence.json"
        fixtures_path.write_text(json.dumps(fixtures), encoding="utf-8")
        manifest_path.write_text(json.dumps(manifest), encoding="utf-8")
        with redirect_stdout(io.StringIO()):
            result = runner.main(["--url", url, "--kind", "node-baseline", "--manifest", str(manifest_path),
                                  "--fixtures", str(fixtures_path), "--state-dir", str(state_dir), "--evidence", str(evidence)])
        return result, json.loads(evidence.read_text(encoding="utf-8"))

    def test_omitted_required_scenario_fails_cli(self):
        with tempfile.TemporaryDirectory() as directory:
            code, report = self.run_cli([], {"suites": {"smoke": ["missing"]}}, "http://127.0.0.1:1", Path(directory))
        self.assertEqual(code, 1)
        self.assertIn("no executable fixture", report["error"])

    def test_unavailable_target_fails_cli(self):
        # Bind an unused port without listening, keeping another process from claiming it.
        import socket
        with socket.socket() as unavailable, tempfile.TemporaryDirectory() as directory:
            unavailable.bind(("127.0.0.1", 0))
            url = f"http://127.0.0.1:{unavailable.getsockname()[1]}"
            cases = [{"id": "one", "source": "self-check", "steps": [{"path": "/", "expect": {"status": 200}}]}]
            code, report = self.run_cli(cases, {"suites": {"smoke": ["one"]}, "backend_commit": "fixture", "frontend_commit": "fixture"}, url, Path(directory))
        self.assertEqual(code, 1)
        self.assertEqual(report["cases"][0]["status"], "failed")

    def test_mismatch_fails_cli_and_never_claims_parity(self):
        class Handler(BaseHTTPRequestHandler):
            def do_GET(self):
                self.send_response(200)
                self.end_headers()
                self.wfile.write(b'{"status":"success"}')

            def log_message(self, *args):
                pass

        with ThreadingHTTPServer(("127.0.0.1", 0), Handler) as server:
            thread = threading.Thread(target=server.serve_forever, daemon=True)
            thread.start()
            try:
                valid = {"id": "one", "source": "self-check", "steps": [{"path": "/", "expect": {"status": 200, "json": {"status": "success"}}, "state": {"subs": []}}]}
                for kind in ("valid", "status", "body", "required-field", "state"):
                    case = copy.deepcopy(valid)
                    if kind == "status":
                        case["steps"][0]["expect"]["status"] = 404
                    elif kind == "body":
                        case["steps"][0]["expect"]["json"] = {"status": "failed"}
                    elif kind == "required-field":
                        case["steps"][0]["expect"]["json"]["data"] = {}
                    with self.subTest(kind=kind), tempfile.TemporaryDirectory() as directory:
                        state_dir = Path(directory)
                        (state_dir / "sub-store.json").write_text('{"subs":[{}]}' if kind == "state" else '{"subs":[]}', encoding="utf-8")
                        code, report = self.run_cli([case], {"suites": {"smoke": ["one"]}, "backend_commit": "fixture", "frontend_commit": "fixture"}, f"http://127.0.0.1:{server.server_port}", state_dir)
                        self.assertEqual(code, 0 if kind == "valid" else 1)
                        self.assertFalse(report["rust_parity"])
                        self.assertFalse(report["full_coverage"])
            finally:
                server.shutdown()
                thread.join(timeout=5)

    def test_full_suite_rejects_incomplete_coverage(self):
        with self.assertRaisesRegex(ValueError, "M1-complete-inventory"):
            runner.load_cases(runner.HERE / "manifest.json", runner.HERE / "fixtures/http-cases.json", "full")

    def test_upstream_request_and_cache_assertions(self):
        from upstream import ControlledUpstream, substitute
        with ControlledUpstream() as upstream:
            step = {"path": "/subscription", "method": "HEAD", "headers": {"User-Agent": "fixture-client"}}
            response = runner.request(upstream.url, step)
            self.assertEqual(response["status"], 200)
            self.assertEqual(response["headers"]["subscription-userinfo"], "upload=10; download=20; total=1000")
            runner.check_json(upstream.requests, [{"method": "HEAD", "path": "/subscription", "headers": {"user-agent": "fixture-client"}, "body": ""}], subset=True)
            with self.assertRaises(AssertionError):
                runner.check_json(upstream.requests, [], subset=True)
            with self.assertRaises(AssertionError):
                runner.check_json(upstream.requests, [{"method": "GET"}], subset=True)
            self.assertEqual(substitute({"url": "$UPSTREAM/subscription", "nil": None}, upstream.url), {"url": upstream.url + "/subscription", "nil": None})


if __name__ == "__main__":
    unittest.main()
