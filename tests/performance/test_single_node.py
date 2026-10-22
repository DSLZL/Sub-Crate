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
