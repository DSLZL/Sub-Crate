"""Actions matrix and strict aggregation for same-runner Node/Rust pairs."""
import argparse
import copy
import json
import os
from pathlib import Path
import shutil
from types import SimpleNamespace

import benchmark as b


def aggregate(prepared, workers, diagnostic=None):
    report = copy.deepcopy(prepared)
    report.pop("preparation_complete", None)
    report.update(status="running", samples=[], workers=[], matrix_complete=False)
    expected = set(b.SHARDS) if not diagnostic else {"diagnostic"}
    seen = set()
    valid = prepared.get("status") == "passed" and prepared.get("preparation_complete") is True
    valid &= len(prepared.get("imports", [])) == 10 and all(r["restart"] for r in prepared.get("imports", []))
    if diagnostic:
        report["diagnostic"] = diagnostic + "_diagnostic"
    for shard, worker in workers:
        valid &= shard in expected and shard not in seen
        seen.add(shard)
        provenance = worker.get("provenance", {})
        runner = provenance.get("runner", {})
        if runner.get("cpu_model"):
            b.PUBLIC_STRINGS.add(runner["cpu_model"])
        valid &= {k: v for k, v in provenance.items() if k != "runner"} == {k: v for k, v in prepared["provenance"].items() if k != "runner"}
        valid &= worker.get("capture") == prepared.get("capture") and worker.get("script") == prepared.get("script")
        replay = worker.get("replay", [])
        valid &= len(replay) == 2 and {r["kind"] for r in replay} == set(b.KINDS)
        valid &= all(r["equal"] and r["matching_sources"] == r["source_count"] == 23 for r in replay)
        cells = worker.get("samples", [])
        if not diagnostic and shard in b.SHARDS:
            case, block, clients = b.SHARDS[shard]
            valid &= {(r["case"], r["block"], r["concurrency"], r["kind"]) for r in cells} == {(case, block, clients, kind) for kind in b.KINDS} and len(cells) == 2
        elif diagnostic:
            cases = ("parse", "pre_health") if diagnostic == "parse" else ("pre_health", "cold", "warm", "no_publisher")
            kinds = b.KINDS if diagnostic == "parse" else b.COMPARISON_KINDS
            valid &= {(r["case"], r["block"], r["concurrency"], r["kind"]) for r in cells} == {(case, 0, 1, kind) for case in cases for kind in kinds} and len(cells) == len(cases) * len(kinds)
            if diagnostic == "parse":
                direct = worker.get("direct_sources", [])
                valid &= len(direct) == 44 and {(r["kind"], r["source_index"]) for r in direct} == {(kind, index) for kind in b.KINDS for index in range(22)}
                valid &= all(r["status"] == "passed" for r in direct)
        valid &= worker.get("status") == "passed" and all(r["status"] == "passed" for r in cells)
        report["samples"].extend({**row, "worker": shard} if not diagnostic else row for row in cells)
        report["workers"].append({"runner": runner, "replay": replay,
                                  "fixture_calibration": worker.get("fixture_calibration"),
                                  "status": worker.get("status", "failed")})
        if not diagnostic:
            report["workers"][-1]["shard"] = shard
        if diagnostic:
            report["direct_sources"] = worker.get("direct_sources", [])
    valid &= seen == expected
    # Pair checks happen on each runner; also reject inconsistent controlled work across workers.
    for case in ("parse", "pre_health", "full"):
        rows = [r for r in report["samples"] if (r["case"] if r["case"] in ("parse", "pre_health") else "full") == case and r["case"] != "real" and r["status"] == "passed"]
        valid &= len({r.get("output_sha256") for r in rows}) <= 1
    report["matrix_complete"] = not diagnostic and seen == expected and len(report["samples"]) == 70
    report["status"] = "passed" if valid else "failed"
    if not valid:
        report["failure"] = "work_incomplete"
    return report


def latest_workers(directory):
    # A failed-job rerun reuses successful earlier pairs. Prefer the newest attempt
    # of each pair, while retaining every older artifact as immutable evidence.
    selected = {}
    # download-artifact v6 flattens a sole pattern match, even with merge=false.
    if (directory / "metrics.json").exists():
        worker = json.loads((directory / "metrics.json").read_bytes())
        shard = worker.get("shard", "diagnostic" if worker.get("diagnostic") else "")
        b.require(shard in b.SHARDS or shard == "diagnostic", "source")
        return [(shard, worker)]
    for path in sorted(directory.glob("*/metrics.json")):
        shard, run, attempt = path.parent.name.removeprefix("subscription-performance-shard-").rsplit("-", 2)
        b.require(run == os.environ["GITHUB_RUN_ID"] and attempt.isdecimal(), "source")
        if shard not in selected or int(attempt) > selected[shard][0]:
            selected[shard] = (int(attempt), path)
    return [(shard, json.loads(path.read_bytes())) for shard, (_, path) in selected.items()]


def main():
    b.require(os.environ.get("GITHUB_ACTIONS") == "true", "source")
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=("matrix", "aggregate"))
    parser.add_argument("--diagnostic", choices=("parse", "health"))
    parser.add_argument("--prepared", type=Path)
    parser.add_argument("--workers", type=Path)
    parser.add_argument("--publish", type=Path)
    args = parser.parse_args()
    if args.command == "matrix":
        print(json.dumps({"include": [{"shard": name} for name in (["diagnostic"] if args.diagnostic else b.SHARDS)]}))
        return
    prepared = json.loads((args.prepared / "metrics.json").read_bytes())
    b.PUBLIC_STRINGS.add(prepared["provenance"]["runner"]["cpu_model"])
    report = aggregate(prepared, latest_workers(args.workers), args.diagnostic)
    publisher = object.__new__(b.Suite)
    publisher.report, publisher.args = report, SimpleNamespace(publish=args.publish)
    args.publish.mkdir(parents=True)
    for patch in args.prepared.glob("*.patch"):
        shutil.copyfile(patch, args.publish / patch.name)
    publisher.publish()
    print(f"Aggregation {report['status']}: workers={len(report['workers'])}, cells={len(report['samples'])}, matrix_complete={report['matrix_complete']}", flush=True)
    raise SystemExit(0 if report["status"] == "passed" else 1)


if __name__ == "__main__":
    main()
