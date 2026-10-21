"""Meaningful workload, process measurement and publication invariants; CI only."""
import asyncio
import copy
from contextlib import nullcontext
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time
from types import SimpleNamespace
import unittest
from unittest.mock import patch
from urllib.parse import unquote, urlsplit

import benchmark as b

if os.environ.get("GITHUB_ACTIONS") != "true":
    raise SystemExit("Run benchmark self-checks in GitHub Actions only")


class Contracts(unittest.TestCase):
    def test_rust_comparison_uses_distinct_binaries_and_identical_disposable_state(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            seed = root / "seed"
            seed.mkdir()
            (seed / "root.json").write_text('{"same":"imported state"}')
            suite = object.__new__(b.Suite)
            suite.args = SimpleNamespace(rust=root / "candidate", rust_baseline=root / "baseline", frontend=root / "frontend")
            suite.pristine, suite.env = {"rust": seed}, {}
            for kind, source in (("rust", "candidate"), ("rust_baseline", "baseline"), ("rust_repeat", "candidate")):
                directory = root / kind
                directory.mkdir()
                with patch.object(b, "process", return_value=nullcontext("owned child")) as launch, patch.object(b, "ready"):
                    with suite.server(kind, directory) as (_, child, state, _):
                        self.assertEqual(child, "owned child")
                        self.assertEqual((state / "root.json").read_bytes(), (seed / "root.json").read_bytes())
                        (state / "root.json").write_text("disposable mutation")
                    argv, cwd, env, _ = launch.call_args.args
                    self.assertEqual(argv, [root / source / "rust/target/release/sub-store-rust"])
                    self.assertEqual(cwd, root / source)
                    self.assertEqual(env["PATH"], "")
            self.assertEqual((seed / "root.json").read_text(), '{"same":"imported state"}')

    def test_health_profile_preserves_results_errors_and_restores_helpers(self):
        source = """async function operator(value) {
          if (ProxyUtils.produce(value) !== value) throw Error('identity');
          ProxyUtils.hex_md5('public');
          scriptResourceCache.get('public'); scriptResourceCache.set('public', 1);
          if (value.fail) throw Error('original failure');
          return value;
        }"""
        fixture = """const assert = require('node:assert/strict');
        let profile;
        const ProxyUtils = {produce(value) { assert.equal(this, ProxyUtils); return value; }, hex_md5() { return 'x'; }};
        const scriptResourceCache = {get() {}, set() {}};
        const original = {...ProxyUtils};
        const $substore = {write(value, key) { assert.equal(key, '#benchmark-health-profile'); profile = value; }};
        """
        checks = """(async () => {
          const value = {}; assert.equal(await operator(value), value);
          assert.deepEqual(ProxyUtils, original);
          assert.equal(profile.md5_calls, 1); assert.equal(profile.produce_calls, 1);
          assert.equal(profile.cache_get_calls, 1); assert.equal(profile.cache_set_calls, 1);
          await assert.rejects(operator({fail: true}), /original failure/);
          assert.deepEqual(ProxyUtils, original);
          assert(Object.values(profile).every(v => typeof v === 'number' && Number.isFinite(v)));
          process.stdout.write(JSON.stringify(profile));
        })().catch(error => { console.error(error); process.exitCode = 1; });"""
        profile = json.loads(b.command(["node", "-e", fixture + b.profile_health_script(source) + checks]))
        b.public({"health_profile": profile})

    def test_direct_sources_preserve_all_inputs_and_private_mismatch_evidence(self):
        with tempfile.TemporaryDirectory() as directory:
            scratch = Path(directory)
            (scratch / "mirrors").mkdir()
            for index in range(22):
                (scratch / "mirrors" / f"{index}.txt").write_bytes(f"private-source-{index}".encode())
            suite = object.__new__(b.Suite)
            suite.args, suite.report = SimpleNamespace(scratch=scratch), {}
            suite.directory = lambda _: scratch
            suite.publish = lambda: b.public(suite.report)
            child = SimpleNamespace(pid=os.getpid())
            observed = []
            def preview(origin, path, method, data):
                self.assertEqual((path, method, data["source"], data["process"]), ("preview/sub", "POST", "local", []))
                observed.append(data["content"])
                changed = origin == "rust" and data["content"] == "private-source-3"
                return {"processed": [{"password": "private-changed" if changed else "private-original"}]}
            servers = [nullcontext((kind, child, None, None)) for kind in b.KINDS]
            with patch.object(suite, "server", side_effect=servers), patch.object(b, "api", side_effect=preview), patch.object(b, "cpu", return_value=0), patch("builtins.print"):
                suite.diagnose_sources()
            self.assertEqual(observed, [f"private-source-{i}" for i in range(22)] * 2)
            rows = suite.report["direct_sources"]
            self.assertEqual(len(rows), 44)
            self.assertEqual([(r["kind"], r["source_index"]) for r in rows if r["status"] == "failed"], [("rust", 3)])
            self.assertIn("failure_observations", rows[25])
            self.assertNotIn("private-", json.dumps(suite.report))

    def test_real_pinned_script_removes_only_five_maintenance_sites(self):
        source = b.get_script()
        variant = b.stress_script(source)
        changes = [(x, y) for x, y in zip(source.splitlines(), variant.splitlines()) if x != y]
        self.assertEqual(len(changes), 5)
        self.assertIn("if (shouldRefresh && maxChecks > 0)", source)
        for preserved in ("attempt <= retries", "await mapLimit(batch, concurrency", "cache.set(cacheKey", "budget_skipped=${skippedBudget}"):
            self.assertIn(preserved, variant)
        with self.assertRaises(b.Failure):
            b.stress_script(source + source)

    def test_import_suffix_is_precise_and_other_data_is_lossless(self):
        original = {"name": "demo", "displayName": "name_123", "display-name": "old",
                    "process": [{"args": {"namespace": "private", "max_checks": "150"}}]}
        imported = {**copy.deepcopy(original), "name": "demo_1792461600123", "displayName": "name_123_1792461600123", "display-name": "name_123_1792461600123"}
        b.import_transform(original, imported)
        broken = copy.deepcopy(imported)
        broken["process"][0]["args"]["max_checks"] = 150
        with self.assertRaises(b.Failure):
            b.import_transform(original, broken)
        with self.assertRaises(b.Failure):
            b.import_transform(original, {**imported, "name": "demo_123"})

    def test_link_arguments_are_delivered_through_the_actual_fragment_contract(self):
        original = {"name": "demo", "url": "https://private.invalid", "process": [{} for _ in range(7)]}
        original["process"][4] = {"type": "Script Operator", "args": {"mode": "link", "content": "https://script.invalid#max_checks=150", "arguments": {"max_checks": "150", "namespace": "private", "refresh_mode": "always"}}}
        suite = object.__new__(b.Suite)
        suite.fixture_origin = "http://127.0.0.1:12345"
        with patch.object(b, "api", side_effect=[[{"name": "imported"}], None]) as api:
            config = suite.configure("http://127.0.0.1:1", original)
        actual = json.loads(unquote(urlsplit(config["process"][4]["args"]["content"]).fragment))
        self.assertEqual(actual, {"max_checks": "150", "namespace": "private", "refresh_mode": "always", "http_meta_port": "12345"})
        self.assertEqual(len(config["url"].splitlines()), 22)
        self.assertEqual(config["process"][4]["args"]["mode"], "link")
        self.assertEqual(api.call_args.args[3], config)
        self.assertNotIn("http_meta_port", original["process"][4]["args"]["arguments"])

    def test_equivalence_keeps_credentials_order_and_metadata(self):
        nodes = [{"name": "x", "password": "secret", "_latency": "1", "_subName": "x", "nested": {"_latency": "a"}}]
        other = copy.deepcopy(nodes)
        other[0]["_latency"] = "2"
        self.assertEqual(b.nodes_hash(nodes), b.nodes_hash(other))
        for key in ("password", "_subName", "nested"):
            changed = copy.deepcopy(other)
            changed[0][key] = "different"
            self.assertNotEqual(b.nodes_hash(nodes), b.nodes_hash(changed))

    def test_publication_rejects_raw_nested_credentials_and_errors(self):
        for value in ({"url": "https://provider.invalid/?token=secret"},
                      {"state": {"nodes": ["ss://secret@server.invalid:1"]}},
                      {"error": "password=secret"}, {"name": "a private node"},
                      {"rss": float("nan")}):
            with self.assertRaises(b.Failure):
                b.public(value)
        b.public({"status": "passed", "sha256": "a" * 64, "bytes": 123, "cpu_s": .25})

    def test_random_hopping_port_is_semantic_only_in_parse_diagnostics(self):
        left = [{"name": "x", "ports": "100-102,200", "port": 101}]
        right = [{**left[0], "port": 200}]
        self.assertNotEqual(b.nodes_hash(left), b.nodes_hash(right))
        b.compare_nodes(left, right, hopping=True)
        for changed in ({"port": 99}, {"port": "101"}, {"ports": "100-103,200"}, {"password": "different"}):
            with self.assertRaises(b.Failure):
                b.compare_nodes(left, [{**right[0], **changed}], hopping=True)
        with self.assertRaises(b.Failure):
            b.compare_nodes(left, [{"name": "x", "ports": "100-102,200"}], hopping=True)
        self.assertEqual(left[0]["port"], 101)

    def test_mismatch_diagnostics_keep_paths_but_never_values_or_custom_keys(self):
        with self.assertRaises(b.Failure) as failure:
            b.compare_nodes([{"password": "secret-a", "private-token-key": ["private-value"], "_latency": 1}],
                            [{"password": "secret-b", "private-token-key": ["changed"], "_latency": 2}])
        observations = failure.exception.observations
        b.public(observations)
        text = json.dumps(observations)
        self.assertIn("password", text)
        for private in ("secret-a", "secret-b", "private-token-key", "private-value", "changed", "_latency"):
            self.assertNotIn(private, text)
        b.compare_nodes([{"name": "same", "_latency": 1}], [{"name": "same", "_latency": 2}])

    def test_paired_statistics_use_blocks_and_units(self):
        result = b.paired_interval([(2, 1), (4, 2), (8, 4)])
        self.assertAlmostEqual(result["node_over_rust"], 2)
        self.assertEqual(result["pairs"], 3)
        self.assertNotIn("p95", b.distribution([.01, .02]))
        self.assertEqual(b.distribution(list(range(1, 101)))["p95"], 95)

    def test_failed_correctness_cells_keep_metrics_without_speedup_estimates(self):
        with tempfile.TemporaryDirectory() as tmp:
            row = {"case": "parse", "block": 0, "concurrency": 1, "latencies_s": [.1],
                   "elapsed_s": .1, "cpu_s": .05, "rps": 10, "rss_peak_bytes": 1000,
                   "os_lifetime_peak_bytes": 2000, "output_count": 1, "output_sha256": "a" * 64}
            report = {"status": "failed", "imports": [], "samples": [
                {**row, "kind": "node", "status": "passed"},
                {**row, "kind": "rust", "status": "failed", "failure": "output_mismatch"}]}
            suite = object.__new__(b.Suite)
            suite.args, suite.report = SimpleNamespace(publish=Path(tmp)), report
            suite.publish()
            self.assertEqual(json.loads((Path(tmp) / "metrics.json").read_bytes())["estimates"], [])
            summary = (Path(tmp) / "summary.md").read_text()
            self.assertIn("| parse | 1 | rust | 0/1 |", summary)
            self.assertIn("FAILED parse rust", summary)

    def test_missing_host_cgroup_quota_is_unavailable(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "cpu.max"
            self.assertEqual(b.cpu_quota(path), {"cpu_quota_us": None, "cpu_period_us": None})
            path.write_text("max 100000\n")
            self.assertEqual(b.cpu_quota(path), {"cpu_quota_us": None, "cpu_period_us": 100000})
            path.write_text("200000 100000\n")
            self.assertEqual(b.cpu_quota(path), {"cpu_quota_us": 200000, "cpu_period_us": 100000})

    def test_partial_work_and_empty_snapshot_are_rejected(self):
        with tempfile.TemporaryDirectory() as tmp:
            state = Path(tmp)
            cache = {"http-meta:health-inc:v1:key": {"data": {"records": {"a": {"lastProbe": 1, "lastOk": 1}, "b": {"lastProbe": 1, "lastOk": 1}}}},
                     "health-snapshot:v1:key": {"data": {"version": 1, "count": 1, "generatedAt": int(time.time() * 1000), "proxies": [{"name": "x"}]}}}
            (state / "root.json").write_text(json.dumps({"sub-store-cached-script-resource": json.dumps(cache)}))
            suite = object.__new__(b.Suite)
            suite.expected_snapshot_hash = None
            counts = {"starts": 1, "stops": 1, "selected": 2, "unique_probes": 2, "probes": 2}
            with patch.object(b, "api", return_value={"processed": [{"name": "x"}]}):
                self.assertEqual(suite.audit(state, [{"name": "x"}], True, counts, "http://127.0.0.1:1")["identities"], 2)
                self.assertEqual(suite.audit(state, [{"name": "x"}], True, {**counts, "probes": 3}, "http://127.0.0.1:1")["identities"], 2)
                self.assertEqual(suite.audit(state, [{"name": "x"}], None, {**counts, "starts": 2}, "http://127.0.0.1:1")["identities"], 2)
                for key in ("selected", "unique_probes", "probes"):
                    with self.assertRaises(b.Failure):
                        suite.audit(state, [{"name": "x"}], True, {**counts, key: 1}, "http://127.0.0.1:1")
                with self.assertRaises(b.Failure):
                    suite.audit(state, [], True, counts, "http://127.0.0.1:1")
                with self.assertRaises(b.Failure):
                    suite.audit(state, [{"name": "x"}], False, counts, "http://127.0.0.1:1")

    def test_resource_units_refer_to_the_owned_server_pid(self):
        script = "import sys,time\nsys.stdin.buffer.read(1)\nend=time.monotonic()+.25\nwhile time.monotonic()<end: pass\nprint('done',flush=True)\nsys.stdin.buffer.read(1)"
        with subprocess.Popen([sys.executable, "-c", script], stdin=subprocess.PIPE, stdout=subprocess.PIPE) as child:
            try:
                with b.Meter(child, None) as meter:
                    child.stdin.write(b"x")
                    child.stdin.flush()
                    self.assertEqual(child.stdout.readline(), b"done\n")
                result = meter.result(1)
                self.assertGreater(result["cpu_s"], .1)
                self.assertGreater(result["rss_peak_bytes"], 1024 * 1024)
                self.assertGreaterEqual(result["os_lifetime_peak_bytes"], result["rss_peak_bytes"])
                self.assertGreater(len(result["rss_trace"]), 1)
            finally:
                child.stdin.write(b"x")
                child.stdin.flush()


class DetectorContract(unittest.IsolatedAsyncioTestCase):
    async def test_distinct_ports_count_unique_work_and_cleanup(self):
        fixture = b.Fixture(Path("."), 0, 0)
        server = await asyncio.start_server(fixture.handle, "127.0.0.1", 0)
        origin = f"http://127.0.0.1:{server.sockets[0].getsockname()[1]}"
        try:
            start = json.loads(await asyncio.to_thread(b.http, origin + "/start", "POST", {"proxies": [{}, {}, {}]}))
            self.assertEqual(len(set(start["ports"])), 3)
            await asyncio.gather(*(asyncio.to_thread(b.http, f"http://127.0.0.1:{port}/", "HEAD") for port in start["ports"]))
            await asyncio.to_thread(b.http, f"http://127.0.0.1:{start['ports'][0]}/", "HEAD")
            await asyncio.to_thread(b.http, origin + "/stop", "POST", {"pid": [start["pid"]]})
            self.assertEqual(fixture.counts["unique_probes"], 3)
            self.assertEqual(fixture.counts["probes"], 4)
            self.assertFalse(fixture.batches)
            with self.assertRaises(b.Failure) as failure:
                await asyncio.to_thread(b.http, origin + "/missing")
            self.assertEqual(failure.exception.observations["http_status"], 404)
            b.public(failure.exception.observations)
        finally:
            server.close()
            await server.wait_closed()


if __name__ == "__main__":
    unittest.main()
