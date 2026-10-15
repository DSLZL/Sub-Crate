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
    def test_controlled_node_info_proxy_replies_locally_and_rejects_other_hosts(self):
        from http.client import HTTPConnection
        from upstream import ControlledUpstream
        with ControlledUpstream() as upstream:
            connection = HTTPConnection("127.0.0.1", upstream.server.server_port, timeout=2)
            connection.request("GET", "http://ip-api.com/json/2001%3Adb8%3A%3A1?lang=zh-CN")
            response = connection.getresponse()
            self.assertEqual(response.status, 200)
            self.assertEqual(json.loads(response.read()), {"status": "success", "query": "2001:db8::1", "countryCode": "US", "city": "fixture-city", "lat": 0, "mobile": False})
            connection.close()
            connection = HTTPConnection("127.0.0.1", upstream.server.server_port, timeout=2)
            connection.request("GET", "http://other.invalid/")
            response = connection.getresponse()
            self.assertEqual(response.status, 502)
            self.assertEqual(json.loads(response.read()), {"message": "Fixture proxy forbids this authority"})
            connection.close()
            self.assertEqual([(item["method"], item["path"]) for item in upstream.requests], [
                ("GET", "http://ip-api.com/json/2001%3Adb8%3A%3A1?lang=zh-CN"), ("GET", "http://other.invalid/")])

    def test_native_probe_timeout_fails_and_preserves_partial_output(self):
        import reference
        import subprocess
        with tempfile.TemporaryDirectory() as directory:
            with patch("reference.shutil.which", return_value="rtk.exe"), patch("reference.subprocess.run", side_effect=subprocess.TimeoutExpired(["probe"], 1, b"partial\r\n", b"blocked")) as process:
                result = reference.execute(["probe"], directory, {"PATH": "fixture"}, Path(directory), "native-probe", timeout=1, direct_reason="Owned fixture binary watchdog")
            evidence = json.loads((Path(directory) / "native-probe.command.json").read_text(encoding="utf-8"))
            self.assertEqual(result.returncode, 124)
            self.assertTrue(evidence["timed_out"])
            self.assertEqual(process.call_args.args[0], ["probe"])
            self.assertEqual(evidence["rtk_exception"], "Owned fixture binary watchdog")
            self.assertEqual((Path(directory) / "native-probe.stdout.log").read_text(encoding="utf-8"), "partial\n")
            self.assertEqual((Path(directory) / "native-probe.stdout.log").read_bytes(), b"partial\r\n")
            self.assertIn("blocked", (Path(directory) / "native-probe.stderr.log").read_text(encoding="utf-8"))

    def test_browser_workflow_requires_its_actual_successful_backend_request(self):
        from frontend import check_network
        cases = [{"id": "FE01-url-backend-setup"}]
        entry = {"request": {"method": "GET", "url": "http://127.0.0.1:1/backend/api/utils/env?share=true"}, "response": {"status": 200}}
        self.assertEqual(len(check_network([entry], cases, "http://127.0.0.1:1/backend")), 1)
        public = {"request": {"method": "GET", "url": "http://127.0.0.1:1/share/file/browser-file-renamed?token=fixture"}, "response": {"status": 200}}
        share_case = [{"id": "FE09-open-public-share"}]
        self.assertEqual(len(check_network([public], share_case, "http://127.0.0.1:1/backend")), 1)
        wrong_prefix = copy.deepcopy(public)
        wrong_prefix["request"]["url"] = "http://127.0.0.1:1/backend/share/file/browser-file-renamed?token=fixture"
        with self.assertRaises(AssertionError):
            check_network([wrong_prefix], share_case, "http://127.0.0.1:1/backend")
        archive_requests = [
            {"request": {"method": "GET", "url": "http://127.0.0.1:1/backend/api/archives"}, "response": {"status": 200}},
            {"request": {"method": "POST", "url": "http://127.0.0.1:1/backend/api/archives/123-abc/restore"}, "response": {"status": 200}},
        ]
        archive_case = [{"id": "FE15-restore-file-archive", "archive_id": "123-abc"}]
        self.assertEqual(len(check_network(archive_requests, archive_case, "http://127.0.0.1:1/backend")), 2)
        with self.assertRaises(AssertionError):
            check_network(archive_requests, [{"id": "FE15-restore-file-archive", "archive_id": "wrong"}], "http://127.0.0.1:1/backend")
        with self.assertRaises(KeyError):
            check_network(archive_requests, [{"id": "FE15-restore-file-archive"}], "http://127.0.0.1:1/backend")
        log_requests = [
            {"request": {"method": "PATCH", "url": "http://127.0.0.1:1/backend/api/file/browser-file-renamed"}, "response": {"status": 200}},
            {"request": {"method": "GET", "url": "http://127.0.0.1:1/backend/api/logs?keyword=selected&limit=20"}, "response": {"status": 200}},
        ]
        self.assertEqual(len(check_network(log_requests, [{"id": "FE17-read-filter-logs", "keyword": "selected"}], "http://127.0.0.1:1/backend")), 2)
        with self.assertRaises(AssertionError):
            check_network(log_requests, [{"id": "FE17-read-filter-logs", "keyword": "wrong"}], "http://127.0.0.1:1/backend")
        for mutation in ("missing", "error", "wrong-origin", "wrong-method"):
            entries = [copy.deepcopy(entry)]
            if mutation == "missing":
                entries = []
            elif mutation == "error":
                entries[0]["response"]["status"] = 500
            elif mutation == "wrong-origin":
                entries[0]["request"]["url"] = "http://127.0.0.1:2/backend/api/utils/env"
            else:
                entries[0]["request"]["method"] = "POST"
            with self.subTest(mutation=mutation), self.assertRaises(AssertionError):
                check_network(entries, cases, "http://127.0.0.1:1/backend")

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

    def test_denied_origin_must_not_receive_an_allow_origin_header(self):
        response = {"status": 403, "headers": {}, "body": "CORS origin not allowed"}
        expected = {"status": 403, "absent_headers": ["Access-Control-Allow-Origin"]}
        runner.check_response(response, expected)
        for value in ("*", "https://denied.invalid", ""):
            with self.subTest(value=value), self.assertRaises(AssertionError):
                runner.check_response({**response, "headers": {"access-control-allow-origin": value}}, expected)

    def test_provider_state_and_base64_payload_mismatch_fail(self):
        import base64
        from upstream import ControlledUpstream
        payload = {"settings": {}, "unknown": {"nil": None, "zero": 0, "flag": False}}
        stored = {"backup": {"description": "fixture", "files": {"Sub-Store": {"content": base64.b64encode(json.dumps(payload).encode()).decode()}}}}
        cases = [{"id": "provider-check", "source": "self-check", "upstream_gists": stored, "steps": [
            {"path": "/unused", "expect": {"status": 200}, "upstream_state": copy.deepcopy(stored),
             "base64_backup": {"gist": "backup", "file": "Sub-Store", "json": payload}}
        ]}]
        for mutation in (None, "wrong-provider-state", "wrong-payload", "invalid-base64"):
            altered = copy.deepcopy(cases)
            if mutation == "wrong-provider-state":
                altered[0]["steps"][0]["upstream_state"]["backup"]["description"] = "wrong"
            elif mutation == "wrong-payload":
                altered[0]["steps"][0]["base64_backup"]["json"]["unknown"]["flag"] = 0
            elif mutation == "invalid-base64":
                altered[0]["upstream_gists"]["backup"]["files"]["Sub-Store"]["content"] = "not base64!"
            with self.subTest(mutation=mutation), ControlledUpstream() as upstream:
                with patch("runner.request", return_value={"status": 200, "headers": {}, "body": "{}"}):
                    result = runner.run_cases("http://127.0.0.1:1", None, altered, upstream)
                self.assertEqual(result[0]["status"], "passed" if mutation is None else "failed")
                if mutation is None:
                    upstream.gists["backup"]["description"] = "later-change"
                    self.assertEqual(result[0]["steps"][0]["upstream_state"], stored)

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
                        self.assertIn("state", report["cases"][0]["steps"][0])
                        if kind == "state":
                            self.assertEqual(report["cases"][0]["steps"][0]["state"], {"subs": [{}]})
            finally:
                server.shutdown()
                thread.join(timeout=5)

    def test_full_suite_rejects_incomplete_coverage(self):
        with self.assertRaisesRegex(ValueError, "M1-complete-inventory"):
            runner.load_cases(runner.HERE / "manifest.json", runner.HERE / "fixtures/http-cases.json", "full")

    def test_dynamic_values_are_validated_and_reused_in_requests_and_state(self):
        captures = {"ARCHIVE_ID": {"pointer": "/data/0/id", "type": "string", "pattern": r"[0-9]+-[a-z0-9]+"},
                    "ARCHIVED_AT": {"pointer": "/data/0/archivedAt", "type": "integer"}}
        entry = {"id": "123-abc", "archivedAt": 123}
        for mutation in (None, "missing", "bad-id", "boolean-time", "zero-time", "state"):
            actual = copy.deepcopy(entry)
            if mutation == "missing":
                del actual["id"]
            elif mutation == "bad-id":
                actual["id"] = ""
            elif mutation == "boolean-time":
                actual["archivedAt"] = True
            elif mutation == "zero-time":
                actual["archivedAt"] = 0
            response = {"status": 200, "headers": {}, "body": json.dumps({"data": [actual]})}
            case = {"id": "capture", "source": "self-check", "steps": [
                {"path": "/api/archives", "capture": captures, "expect": {"status": 200, "json": {"data": [{"id": "{{ARCHIVE_ID}}", "archivedAt": "{{ARCHIVED_AT}}"}]}},
                 "state": {"archives": [{"id": "{{ARCHIVE_ID}}", "archivedAt": "{{ARCHIVED_AT}}"}]}},
                {"path": "/api/archives/{{ARCHIVE_ID}}/restore", "method": "POST", "expect": {"status": 200, "body": "restored"}}]}
            with self.subTest(mutation=mutation), tempfile.TemporaryDirectory() as directory:
                state_dir = Path(directory)
                stored = {**entry, "id": "wrong"} if mutation == "state" else entry
                (state_dir / "sub-store.json").write_text(json.dumps({"archives": [stored]}), encoding="utf-8")
                with patch("runner.request", side_effect=[response, {"status": 200, "headers": {}, "body": "restored"}]) as request:
                    result = runner.run_cases("http://127.0.0.1:1/backend", state_dir, [case])[0]
                self.assertEqual(result["status"], "passed" if mutation is None else "failed")
                if mutation is None:
                    self.assertEqual(request.call_args.args[1]["path"], "/api/archives/123-abc/restore")
                    self.assertEqual(result["steps"][0]["captured"]["ARCHIVED_AT"], 123)
        escaped = {"body": '{"a/b":{"~key":"123-abc"}}'}
        self.assertEqual(runner.capture_values(escaped, {"ID": {"pointer": "/a~1b/~0key", "type": "string", "pattern": ".+"}}), {"ID": "123-abc"})
        with self.assertRaises(KeyError):
            runner.interpolate("/api/archives/{{MISSING}}", {})

    def test_upstream_request_and_cache_assertions(self):
        from upstream import ControlledUpstream, substitute
        with ControlledUpstream() as upstream:
            step = {"path": "/subscription", "method": "HEAD", "headers": {"User-Agent": "fixture-client"}}
            response = runner.request(upstream.url, step)
            self.assertEqual(response["status"], 200)
            self.assertEqual(response["headers"]["subscription-userinfo"], "upload=10; download=20; total=1000")
            self.assertEqual(runner.request(upstream.url + "/backend", {**step, "origin": True})["status"], 200)
            upstream.requests.pop()
            runner.check_json(upstream.requests, [{"method": "HEAD", "path": "/subscription", "headers": {"user-agent": "fixture-client"}, "body": ""}], subset=True)
            with self.assertRaises(AssertionError):
                runner.check_json(upstream.requests, [], subset=True)
            with self.assertRaises(AssertionError):
                runner.check_json(upstream.requests, [{"method": "GET"}], subset=True)
            self.assertEqual(substitute({"url": "$UPSTREAM/subscription", "nil": None}, upstream.url), {"url": upstream.url + "/subscription", "nil": None})
            failed = runner.run_cases(upstream.url, None, [{"id": "failure-evidence", "source": "self-check", "steps": [{**step, "expect": {"status": 500}}]}], upstream)[0]
            self.assertEqual(failed["status"], "failed")
            self.assertEqual(failed["steps"][0]["response"]["status"], 200)
            self.assertEqual(len(failed["steps"][0]["upstream_requests"]), 1)


if __name__ == "__main__":
    unittest.main()
