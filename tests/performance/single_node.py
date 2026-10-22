"""Paired single-node parsing measurements; public fixtures, Actions only."""
import argparse
from contextlib import contextmanager
import hashlib
import json
import math
import os
from pathlib import Path
import platform
import select
import statistics
import subprocess
import time

import benchmark as b

BLOCKS = 7
TARGET_NS = 250_000_000
HTTP_REQUESTS = 30
VARIANTS = ("node", "rust_warm_parser", "rust_fresh_parser")


class Worker:
    def __init__(self, child):
        self.child = child
        self.pending = b""

    def call(self, index, iterations, warmup=0, fresh=False):
        self.child.stdin.write((json.dumps({"case": index, "iterations": iterations,
                                          "warmup": warmup, "fresh": fresh}) + "\n").encode())
        self.child.stdin.flush()
        until = time.monotonic() + 120
        while time.monotonic() < until:
            if b"\n" not in self.pending:
                if not select.select([self.child.stdout], [], [], max(0, until - time.monotonic()))[0]:
                    break
                chunk = os.read(self.child.stdout.fileno(), 65536)
                if not chunk:
                    raise RuntimeError("parser worker exited")
                self.pending += chunk
                continue
            line, self.pending = self.pending.split(b"\n", 1)
            at = line.find(b"SINGLE_NODE ")
            if at >= 0:
                row = json.loads(line[at + len("SINGLE_NODE "):])
                assert row["iterations"] == row["checksum"] == iterations, "expected one node per call"
                assert len(row["outputs"][0]) == 1 and row["outputs"][0] == row["outputs"][1]
                return row
        raise TimeoutError("parser worker watchdog")


@contextmanager
def workers(args, suite, fixtures):
    env = {**suite.env, "SINGLE_NODE_FIXTURES": str(fixtures)}
    state = args.scratch / "pure-node-state"
    state.mkdir()
    env["SUB_STORE_DATA_BASE_PATH"] = str(state)
    rust_env = {**env, "PATH": ""}
    commands = {
        "node": [suite.node, Path(__file__).with_name("single-node.cjs").resolve(), args.original],
        "rust": [args.rust_test, "single_node_profile::compare", "--ignored", "--exact", "--nocapture", "--test-threads=1"],
    }
    children = {}
    try:
        for kind, command in commands.items():
            log = (args.scratch / f"pure-{kind}.log").open("w")
            child = subprocess.Popen(list(map(str, command)), cwd=args.original / "backend" if kind == "node" else args.rust,
                                     env=env if kind == "node" else rust_env, stdin=subprocess.PIPE,
                                     stdout=subprocess.PIPE, stderr=log, bufsize=0)
            log.close()
            children[kind] = child
        yield {kind: Worker(child) for kind, child in children.items()}
    finally:
        for child in children.values():
            child.stdin.close()
            try:
                child.wait(timeout=5)
            except subprocess.TimeoutExpired:
                child.kill()
                child.wait()


def pure(args, suite, fixtures, cases, report):
    with workers(args, suite, fixtures) as running:
        for index, case in enumerate(cases):
            expected = running["node"].call(index, 1000, warmup=10000)["outputs"]
            counts = {}
            for variant in VARIANTS:
                worker = running["node" if variant == "node" else "rust"]
                count = 1
                for _ in range(5):
                    sample = worker.call(index, count, warmup=2, fresh=variant == "rust_fresh_parser")
                    assert sample["outputs"] == expected, f"{case['id']}: {variant} output mismatch"
                    if sample["elapsed_ns"] >= TARGET_NS // 2:
                        break
                    count = min(1_000_000, max(count * 2, math.ceil(count * TARGET_NS / max(1, sample["elapsed_ns"]))))
                counts[variant] = count
            for block in range(BLOCKS):
                order = VARIANTS if block % 2 == 0 else VARIANTS[::-1]
                for variant in order:
                    sample = running["node" if variant == "node" else "rust"].call(
                        index, counts[variant], fresh=variant == "rust_fresh_parser")
                    assert sample.pop("outputs") == expected, f"{case['id']}: measured output mismatch"
                    report["pure"].append({"case": case["id"], "variant": variant, "block": block, **sample})
            print("single-node pure complete: " + case["id"], flush=True)


def endpoint(args, suite, cases, report):
    with suite.server("node", suite.directory("http-node"), clone=False) as node, \
            suite.server("rust", suite.directory("http-rust"), clone=False) as rust:
        origins = {"node": node[0], "rust": rust[0]}
        for case in cases:
            payloads = [{"data": case["raw"] + suffix, "client": "JSON", "type": "internal"}
                        for suffix in ("", "\n")]
            expected = b.api(origins["node"], "proxy/parse", "POST", payloads[0])
            assert len(expected["par_res"]) == 1
            for kind in origins:
                for index in range(20):
                    assert b.api(origins[kind], "proxy/parse", "POST", payloads[index % 2]) == expected
            for block in range(BLOCKS):
                for kind in (("node", "rust") if block % 2 == 0 else ("rust", "node")):
                    samples = []
                    for index in range(HTTP_REQUESTS):
                        began = time.perf_counter_ns()
                        response = b.http(origins[kind] + "/backend/api/proxy/parse", "POST", payloads[index % 2])
                        samples.append(time.perf_counter_ns() - began)
                        body = json.loads(response)
                        assert body["status"] == "success" and body["data"] == expected
                    report["http"].append({"case": case["id"], "variant": kind, "block": block, "ns": samples})
            print("single-node HTTP complete: " + case["id"], flush=True)


def summary(report):
    lines = ["Single-node parsing; one node per function call or HTTP request.",
             "Pure: median of seven block means, us/call. HTTP: median of 210 requests, ms/request.",
             "Equivalent original/newline inputs alternate; no last-input preprocessing cache hits.",
             "HTTP includes JSON production and transport, starts with empty application state.",
             "", "| Format | Node pure us | Rust reused parser us | Rust new parser us | Node HTTP ms | Rust HTTP ms |",
             "|---|---:|---:|---:|---:|---:|"]
    for case in report["cases"]:
        values = []
        for variant in VARIANTS:
            rows = [row for row in report["pure"] if row["case"] == case and row["variant"] == variant]
            values.append(statistics.median(row["elapsed_ns"] / row["iterations"] / 1000 for row in rows))
        for variant in ("node", "rust"):
            samples = [sample for row in report["http"] if row["case"] == case and row["variant"] == variant for sample in row["ns"]]
            values.append(statistics.median(samples) / 1_000_000)
        lines.append("| " + case + " | " + " | ".join(f"{value:.3f}" for value in values) + " |")
    return "\n".join(lines) + "\n"


def main():
    if os.environ.get("GITHUB_ACTIONS") != "true":
        raise SystemExit("Run in GitHub Actions only")
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("original", "rust", "rust-test", "scratch", "output"):
        parser.add_argument("--" + name, required=True, type=lambda value: Path(value).resolve())
    args = parser.parse_args()
    args.scratch.mkdir(parents=True)
    args.output.mkdir(parents=True)
    args.frontend = args.scratch / "unused-frontend"
    (args.frontend / "dist").mkdir(parents=True)
    fixture = Path(__file__).with_name("single-node.json").resolve()
    cases = json.loads(fixture.read_bytes())
    assert len({case["id"] for case in cases}) == len(cases)
    report = {"version": 1, "original": b.ORIGINAL, "rust": b.command(["git", "rev-parse", "HEAD"], args.rust).decode().strip(),
              "fixture_sha256": hashlib.sha256(fixture.read_bytes()).hexdigest(), "machine": platform.machine(),
              "cases": [case["id"] for case in cases], "pure": [], "http": [], "status": "running"}
    assert b.command(["git", "rev-parse", "HEAD"], args.original).decode().strip() == b.ORIGINAL
    suite = b.Suite(args, {})
    try:
        pure(args, suite, fixture, cases, report)
        endpoint(args, suite, cases, report)
        report["status"] = "passed"
        text = summary(report)
        (args.output / "summary.md").write_text(text)
        print(text, flush=True)
    except Exception:
        report["status"] = "failed"
        raise
    finally:
        (args.output / "metrics.json").write_text(json.dumps(report, indent=2) + "\n")


if __name__ == "__main__":
    main()
