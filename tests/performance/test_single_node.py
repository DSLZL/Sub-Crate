"""Complete-request profiles must include late teardown and validate public events."""
import json
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import single_node as s

if os.environ.get("GITHUB_ACTIONS") != "true":
    raise SystemExit("Run benchmark self-checks in GitHub Actions only")


class Profiles(unittest.TestCase):
    def test_tail_statistics_pool_requests_and_preserve_round_and_sample_identity(self):
        rows = [{"case": "clash_hysteria2", "mode": "keep_alive", "variant": "rust", "block": 0,
                 "round": index, "ns": [value] * 95 + [value * 10] * 5}
                for index, value in enumerate((1000, 2000))]
        text = s.tail_summary({"http": rows})
        values = [json.loads(line.removeprefix("TAIL_HTTP ")) for line in text.splitlines() if line.startswith("TAIL_HTTP ")]
        total, first, second = values
        self.assertEqual((total["requests"], total["p95_ns"]), (200, 2000))
        self.assertEqual((first["round"], second["round"]), (0, 1))
        self.assertEqual(second["slowest"][0], [95, 20000])
        self.assertEqual((first["p95_ns"], second["p95_ns"]), (1000, 2000))
        for invalid in ([], [True], [0], [-1]):
            with self.subTest(samples=invalid), self.assertRaises(AssertionError):
                s.distribution(invalid)

    def test_tail_profiles_keep_slow_client_and_worker_events_paired(self):
        events = [{"wall_ns": wall, "rows": [{"stage": stage, "wall_ns": value, "calls": 1}
                   for stage, value in (("worker_queue", queue), ("parse_native", 10), ("produce", 5),
                                        ("conversion_reuse", 1), ("conversion_retain", 1))]}
                  for wall, queue in ((50, 1), (500, 400))]
        group = {"case": "clash_hysteria2", "events": events, "client_ns": [100, 1000], "server_ns": [90, 900]}
        text = s.tail_summary({"http": [], "profiles": [group]})
        slow = [json.loads(line.removeprefix("TAIL_PROFILE_SLOW ")) for line in text.splitlines()
                if line.startswith("TAIL_PROFILE_SLOW ")]
        self.assertEqual((slow[0]["index"], slow[0]["client_ns"], slow[0]["worker_ns"], slow[0]["worker_queue"]),
                         (1, 1000, 500, 400))
        with self.assertRaises(AssertionError):
            s.tail_summary({"http": [], "profiles": [{**group, "server_ns": [90]}]})

    def test_conversion_lifecycle_rejects_false_reuse_and_incomplete_cleanup(self):
        def event(stages):
            return [{"rows": [{"stage": stage, "calls": 1}
                              for stage in [*stages, "parse", "parse_native", "produce"]]}]
        reused = ["worker_queue", "conversion_reuse", "conversion_cleanup", "conversion_retain"]
        created = ["worker_queue", "conversion_miss", "runtime_setup", "context_setup",
                   "bootstrap", "conversion_cleanup", "conversion_retain"]
        for valid in (reused, created, [*reused[:-1], "runtime_teardown"]):
            s.conversion_lifecycle(event(valid))
        for invalid in ([*reused, "bootstrap"], reused[:-1], reused[1:],
                        [stage for stage in reused if stage != "conversion_cleanup"],
                        [*reused, "conversion_miss"], [*reused, "bootstrap_state_encode"],
                        [stage for stage in created if stage != "context_setup"]):
            with self.subTest(stages=invalid), self.assertRaises(AssertionError):
                s.conversion_lifecycle(event(invalid))

    def test_waits_for_teardown_and_rejects_extra_or_private_http_events(self):
        row = {"id": 0, "parent": None, "stage": "runtime_teardown", "index": None,
               "calls": 1, "wall_ns": 10, "active_ns": 10, "self_ns": 10, "max_wall_ns": 10}
        worker = {"version": 1, "job": 1, "operation": "other", "success": True, "wall_ns": 100,
                  "dropped": 0, "open_scopes": 0, "unfinished_spans": 0, "rows": [row]}
        http = b'SUB_STORE_HTTP_PROFILE {"version":1,"wall_ns":90}\n'
        teardown = b"SUB_STORE_PROFILE " + json.dumps(worker).encode() + b"\n"
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "server.log"
            prefix = b"private preceding log\n"
            path.write_bytes(prefix + http + teardown[:-3])
            with patch.object(s.time, "sleep", side_effect=lambda _: path.write_bytes(prefix + http + teardown)) as wait:
                self.assertEqual(s.complete_profiles(path, len(prefix), 1), ([worker], [90]))
                wait.assert_called_once()
            for bad in (http + http, b'SUB_STORE_HTTP_PROFILE {"version":1,"wall_ns":90,"secret":"private"}\n',
                        b'SUB_STORE_HTTP_PROFILE {"version":1,"wall_ns":true}\n'):
                path.write_bytes(prefix + bad + teardown)
                with self.assertRaises(AssertionError):
                    s.complete_profiles(path, len(prefix), 1)

    def test_missing_worker_profile_is_a_failure(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "server.log"
            path.write_bytes(b'SUB_STORE_HTTP_PROFILE {"version":1,"wall_ns":90}\n')
            with patch.object(s.time, "monotonic", side_effect=[0, 0, 31]), \
                    patch.object(s.time, "sleep"), self.assertRaises(TimeoutError):
                s.complete_profiles(path, 0, 1)


if __name__ == "__main__":
    unittest.main()
