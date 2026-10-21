"""Coverage and integrity checks for the parallel benchmark; Actions only."""
import copy
import json
import os
from pathlib import Path
import tempfile
from types import SimpleNamespace
import unittest

import benchmark as b
import parallel

if os.environ.get("GITHUB_ACTIONS") != "true":
    raise SystemExit("Run benchmark self-checks in GitHub Actions only")


def preparation():
    return {"status": "passed", "preparation_complete": True, "input_sha256": b.INPUT_HASH,
            "imports": [{"kind": kind, "restart": True} for kind in b.KINDS for _ in range(5)],
            "provenance": {"harness_sha": "a" * 40, "runner": {"cpu_model": "fixture"}},
            "capture": {"before_preprocessing": True}, "script": {"stress_sha256": "b" * 64},
            "samples": []}


def workers(prepared):
    return [(name, {**copy.deepcopy(prepared), "replay": [
        {"kind": kind, "equal": True, "source_count": 23, "matching_sources": 23} for kind in b.KINDS],
        "samples": [{"kind": kind, "case": case, "block": block, "concurrency": clients,
                     "status": "passed", "output_sha256": "c" * 64} for kind in b.KINDS]})
        for name, (case, block, clients) in b.SHARDS.items()]


class ParallelContract(unittest.TestCase):
    def test_complete_matrix_and_rejected_partial_or_unequal_work(self):
        prepared = preparation()
        complete = workers(prepared)
        dict(complete)["real_0_1"]["detector"] = {"version": "1.3.0", "assets_sha256": ["f" * 64]}
        report = parallel.aggregate(prepared, complete)
        self.assertEqual((report["status"], report["matrix_complete"], len(report["samples"]), len(report["workers"])),
                         ("passed", True, 70, 35))
        self.assertNotIn("workers", prepared)
        self.assertEqual(report["detector"], dict(complete)["real_0_1"]["detector"])
        for defect in ("missing", "duplicate", "source", "replay", "binary", "output", "failed", "cell", "empty"):
            with self.subTest(defect=defect):
                broken = copy.deepcopy(complete)
                worker = broken[0][1]
                if defect == "missing": broken.pop()
                elif defect == "duplicate": broken.append(copy.deepcopy(broken[0]))
                elif defect == "source": worker["capture"] = {}
                elif defect == "replay": worker["replay"][1]["matching_sources"] = 22
                elif defect == "binary": worker["provenance"]["harness_sha"] = "d" * 40
                elif defect == "output": worker["samples"][1]["output_sha256"] = "d" * 64
                elif defect == "failed": worker["samples"][1]["status"] = "failed"
                elif defect == "cell": worker["samples"][1]["block"] = 99
                elif defect == "empty": worker["samples"] = []
                self.assertEqual(parallel.aggregate(prepared, broken)["status"], "failed")
        # Runner models may differ; the comparison still pairs both servers on one runner.
        complete[0][1]["provenance"]["runner"]["cpu_model"] = "fixture"
        self.assertEqual(parallel.aggregate(prepared, complete)["status"], "passed")

    def test_diagnostic_cannot_pass_without_all_direct_and_full_input_cells(self):
        prepared = preparation()
        for diagnostic in ("parse", "health"):
            worker = workers(prepared)[0][1]
            cases = ("parse", "pre_health") if diagnostic == "parse" else ("pre_health", "cold", "warm", "no_publisher")
            kinds = b.KINDS if diagnostic == "parse" else b.COMPARISON_KINDS
            worker["samples"] = [{"case": case, "block": 0, "concurrency": 1, "kind": kind,
                                  "status": "passed", "output_sha256": "c" * 64} for case in cases for kind in kinds]
            worker["direct_sources"] = [{"kind": kind, "source_index": index, "status": "passed"}
                                         for kind in b.KINDS for index in range(22)]
            self.assertEqual(parallel.aggregate(prepared, [("diagnostic", worker)], diagnostic)["status"], "passed")
            worker["samples"].pop()
            self.assertEqual(parallel.aggregate(prepared, [("diagnostic", worker)], diagnostic)["status"], "failed")
            worker["samples"] = []
            self.assertEqual(parallel.aggregate(prepared, [("diagnostic", worker)], diagnostic)["status"], "failed")

    def test_failed_job_rerun_selects_latest_attempt_per_pair(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for shard, attempt in (("parse_0_1", 1), ("parse_0_1", 2), ("warm_0_1", 1)):
                folder = root / f"subscription-performance-shard-{shard}-{os.environ['GITHUB_RUN_ID']}-{attempt}"
                folder.mkdir()
                (folder / "metrics.json").write_text(json.dumps({"attempt": attempt}))
            self.assertEqual(dict(parallel.latest_workers(root)), {"parse_0_1": {"attempt": 2}, "warm_0_1": {"attempt": 1}})
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            worker = {"diagnostic": "parse_diagnostic", "samples": []}
            (root / "metrics.json").write_text(json.dumps(worker))
            self.assertEqual(parallel.latest_workers(root), [("diagnostic", worker)])

    def test_capsule_authentication_rejects_changed_input_run_and_public_digest(self):
        fixture = r"""
        const assert = require('node:assert/strict'), {createHash} = require('node:crypto');
        const {seal, open} = require('./tests/performance/frozen-inputs.cjs');
        const input = Buffer.from('private-subscription-'.repeat(100));
        const plaintext = Buffer.from('private-raw-yaml\r\n节点\0private-seed');
        const encrypted = seal(input, plaintext, '123');
        assert(open(input, encrypted, '123').equals(plaintext));
        assert(!encrypted.includes(plaintext));
        const changed = Buffer.from(encrypted); changed[changed.length - 1] ^= 1;
        for (const [password, body, run] of [
          [input, changed, '123'], [input, encrypted, '124'], [Buffer.from('wrong'), encrypted, '123'],
          [createHash('sha256').update(input).digest(), encrypted, '123'],
          [createHash('sha256').update(input).digest('hex'), encrypted, '123']]) {
          assert.throws(() => open(password, body, run));
        }
        """
        b.command(["node", "-e", fixture])

    def test_encrypted_seed_roundtrip_preserves_raw_text_and_detects_wrong_capture(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            input_file = root / "private-input.json"
            input_file.write_bytes(b"private-input" * 100)
            prepared = preparation()
            prepared["chromium"] = "1.2.3"
            prepared["capture"]["sources"] = []
            sender = object.__new__(b.Suite)
            sender.args, sender.report, sender.pristine = SimpleNamespace(scratch=root / "sender"), prepared, {}
            mirrors = sender.args.scratch / "mirrors"
            mirrors.mkdir(parents=True)
            for index in range(22):
                raw = f"private-{index}\r\n节点".encode() if index != 3 else b""
                (mirrors / f"{index}.txt").write_bytes(raw)
                prepared["capture"]["sources"].append({"index": index, "sha256": b.digest(raw)})
            for kind in b.KINDS:
                seed = sender.args.scratch / ("pristine-" + kind)
                seed.mkdir()
                (seed / "root.json").write_bytes(b"private-ui-created-state")
                sender.pristine[kind] = seed
            capsule = root / "frozen.bin"
            for corrupt in (False, True):
                if corrupt:
                    prepared["capture"]["sources"][2]["sha256"] = "d" * 64
                b.seal_inputs(sender, input_file, capsule)
                receiver = object.__new__(b.Suite)
                receiver.args, receiver.report = SimpleNamespace(scratch=root / str(corrupt)), {"provenance": prepared["provenance"]}
                receiver.args.scratch.mkdir()
                if corrupt:
                    with self.assertRaises(b.Failure): b.load_inputs(receiver, input_file, capsule)
                else:
                    b.load_inputs(receiver, input_file, capsule)
                    for kind in b.KINDS:
                        self.assertEqual((receiver.pristine[kind] / "root.json").read_bytes(), b"private-ui-created-state")
                    self.assertEqual((receiver.args.scratch / "mirrors/0.txt").read_bytes(), (mirrors / "0.txt").read_bytes())
                    self.assertEqual(receiver.report["capture"], prepared["capture"])


if __name__ == "__main__":
    unittest.main()
