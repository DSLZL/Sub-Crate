"""Paired single-node parsing measurements; public fixtures, Actions only."""
import argparse
from contextlib import ExitStack, contextmanager
import hashlib
import json
import math
import os
from pathlib import Path
import platform
import re
import select
import statistics
import subprocess
import time

import benchmark as b

BLOCKS = 7
TARGET_NS = 250_000_000
HTTP_REQUESTS = 30
VARIANTS = ("node", "rust_baseline_warm_parser", "rust_baseline_fresh_parser", "rust_warm_parser", "rust_fresh_parser")


def parser_worker(variant):
    return "node" if variant == "node" else "rust_baseline" if variant.startswith("rust_baseline_") else "rust"


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
        "rust_baseline": [args.rust_baseline_test, "single_node_profile::compare", "--ignored", "--exact", "--nocapture", "--test-threads=1"],
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
            assert expected[0][0]["type"] == case["protocol"], f"{case['id']}: unexpected protocol"
            counts = {}
            for variant in VARIANTS:
                worker = running[parser_worker(variant)]
                count = 1
                for _ in range(5):
                    sample = worker.call(index, count, warmup=2, fresh=variant.endswith("_fresh_parser"))
                    assert sample["outputs"] == expected, f"{case['id']}: {variant} output mismatch"
                    if sample["elapsed_ns"] >= TARGET_NS // 2:
                        break
                    count = min(1_000_000, max(count * 2, math.ceil(count * TARGET_NS / max(1, sample["elapsed_ns"]))))
                counts[variant] = count
            for block in range(BLOCKS):
                order = VARIANTS if block % 2 == 0 else VARIANTS[::-1]
                for variant in order:
                    sample = running[parser_worker(variant)].call(
                        index, counts[variant], fresh=variant.endswith("_fresh_parser"))
                    assert sample.pop("outputs") == expected, f"{case['id']}: measured output mismatch"
                    report["pure"].append({"case": case["id"], "variant": variant, "block": block, **sample})
            print("single-node pure complete: " + case["id"], flush=True)


def endpoint(args, suite, cases, report):
    with ExitStack() as stack:
        origins = {kind: stack.enter_context(suite.server(kind, suite.directory("http-" + kind), clone=False))[0]
                   for kind in ("node", "rust_baseline", "rust")}
        fixtures = json.loads((Path(__file__).parents[1] / "compat/fixtures/http-cases.json").read_bytes())
        source = next(case for case in fixtures if case["id"] == "P02-input-surge-native")["steps"][0]["body"]
        probe_inputs = [*source["data"].splitlines(), source["data"],
                        "not a proxy", "type: ss",
                        "/*prefix*/{type:'ss',name:'generic',server:'json.example',port:443,cipher:'aes-128-gcm',password:'secret'}//=ss,other.example,443,password=other",
                        "shadowsocks=ss.example.com:8388,method=aes-128-gcm,password=secret,tag=type: ss",
                        '"Surge = ss, ss.example.com, 8388, encrypt-method=aes-128-gcm, password=secret"']
        for index, raw in enumerate(probe_inputs):
            expected = None
            for kind, origin in origins.items():
                parsed = b.urlsplit(origin)
                connection = b.HTTPConnection(parsed.hostname, parsed.port, timeout=30)
                try:
                    connection.request("POST", "/backend/api/proxy/parse", b.encoded({**source, "data": raw}),
                                       {"Content-Type": "application/json"})
                    response = connection.getresponse()
                    body = json.loads(response.read())
                finally:
                    connection.close()
                assert response.status == 200, f"public parser input {index}, {kind}: {body}"
                if expected is None:
                    expected = body
                else:
                    assert body == expected, f"public parser input {index}, {kind}: output mismatch"
        print(f"single-node public parser preflight complete: inputs={len(probe_inputs)}, backends={len(origins)}", flush=True)
        connections = {}
        for kind, origin in origins.items():
            parsed = b.urlsplit(origin)
            connections[kind] = b.HTTPConnection(parsed.hostname, parsed.port, timeout=30)
            stack.callback(connections[kind].close)
        for case in cases:
            payloads = [{"data": case["raw"] + suffix, "client": "JSON", "type": "internal"}
                        for suffix in ("", "\n")]
            expected = b.api(origins["node"], "proxy/parse", "POST", payloads[0])
            assert len(expected["par_res"]) == 1
            suite.expected_outputs[case["id"]] = expected
            for mode in ("new_connection", "keep_alive"):
                def request(kind, index):
                    if mode == "new_connection":
                        return b.http(origins[kind] + "/backend/api/proxy/parse", "POST", payloads[index % 2])
                    connection = connections[kind]
                    previous = connection.sock
                    connection.request("POST", "/backend/api/proxy/parse", b.encoded(payloads[index % 2]),
                                       {"Content-Type": "application/json"})
                    response = connection.getresponse()
                    body = response.read()
                    assert response.status == 200 and connection.sock is not None
                    assert previous is None or connection.sock is previous, "keep-alive connection was replaced"
                    return body
                for kind in origins:
                    for index in range(20):
                        assert json.loads(request(kind, index)) == {"status": "success", "data": expected}
                for block in range(BLOCKS):
                    for kind in (tuple(origins) if block % 2 == 0 else tuple(origins)[::-1]):
                        samples = []
                        for index in range(HTTP_REQUESTS):
                            began = time.perf_counter_ns()
                            response = request(kind, index)
                            samples.append(time.perf_counter_ns() - began)
                            assert json.loads(response) == {"status": "success", "data": expected}
                        report["http"].append({"case": case["id"], "variant": kind, "mode": mode,
                                               "block": block, "ns": samples})
            print("single-node HTTP complete: " + case["id"], flush=True)


def complete_profiles(path, offset, count):
    until = time.monotonic() + 30
    while time.monotonic() < until:
        with path.open("rb") as source:
            source.seek(offset)
            lines = [line for line in source if line.endswith(b"\n")]
        workers = sum(line.startswith(b"SUB_STORE_PROFILE ") for line in lines)
        http = [json.loads(line[len(b"SUB_STORE_HTTP_PROFILE "):])
                for line in lines if line.startswith(b"SUB_STORE_HTTP_PROFILE ")]
        assert workers <= count and len(http) <= count, "unexpected extra profile events"
        if workers == len(http) == count:
            events = b.pipeline_profiles(path, offset, operation="other", processor=False)
            for event in http:
                assert set(event) == {"version", "wall_ns"} and event["version"] == 1
                assert type(event["wall_ns"]) is int and event["wall_ns"] > 0
            return events, [event["wall_ns"] for event in http]
        time.sleep(.01)
    raise TimeoutError("profile completion watchdog")


def profiles(args, suite, cases, report):
    # Timed endpoint samples above retain default profiling-off behavior.
    args.profile_pipeline = True
    directory = suite.directory("profile-rust")
    with suite.server("rust_repeat", directory, clone=False) as server:
        for case in cases:
            payload = {"data": case["raw"], "client": "JSON", "type": "internal"}
            expected = suite.expected_outputs[case["id"]]
            path = directory / "server.log"
            offset = path.stat().st_size
            for _ in range(2):
                assert b.api(server[0], "proxy/parse", "POST", payload) == expected
            complete_profiles(path, offset, 2)
            offset = path.stat().st_size
            client_ns = []
            for _ in range(BLOCKS):
                began = time.perf_counter_ns()
                response = b.http(server[0] + "/backend/api/proxy/parse", "POST", payload)
                client_ns.append(time.perf_counter_ns() - began)
                assert json.loads(response) == {"status": "success", "data": expected}
            events, server_ns = complete_profiles(path, offset, BLOCKS)
            assert len(events) == BLOCKS
            if case["format"] in ("URI", "Surge", "QX"):
                assert all(row["stage"] != "parse_context_init" for event in events for row in event["rows"]), \
                    f"{case['id']}: native parsing initialized the generic engine"
            assert all(row["stage"] != "parse_utilities_init" for event in events for row in event["rows"]), \
                f"{case['id']}: parsing initialized the generic libraries"
            assert all(sum(row["calls"] for row in event["rows"] if row["stage"] == "worker_queue") == 1
                       and sum(row["calls"] for row in event["rows"] if row["stage"] == "runtime_teardown") == 1
                       for event in events)
            assert all(row["stage"] != "bootstrap_state_encode" for event in events for row in event["rows"])
            report["profiles"].append({"case": case["id"], "events": events,
                                       "server_ns": server_ns, "client_ns": client_ns})


def summary(report):
    lines = ["Single-node parsing; one node per function call or HTTP request.",
             "Pure: median of seven block means, us/call. HTTP: median of 210 requests, ms/request.",
             "Equivalent original/newline inputs alternate; no last-input preprocessing cache hits.",
             "HTTP includes JSON production and transport, starts with empty application state; the first table opens a connection per request.",
             f"Coverage: {len(report['cases'])} fixtures, including all {len(report['clash_types'])} pinned Clash node types and Surge external.",
             "This measures configuration parsing/conversion, not network traffic through those protocols.",
             "", "| Case | Protocol | Input | Node pure us | Previous Rust reused us | Previous Rust fresh us | Rust reused us | Rust fresh us | Node HTTP ms | Previous Rust HTTP ms | Rust HTTP ms |",
             "|---|---|---|---:|---:|---:|---:|---:|---:|---:|---:|"]
    for case in report["cases"]:
        values = []
        for variant in VARIANTS:
            rows = [row for row in report["pure"] if row["case"] == case and row["variant"] == variant]
            values.append(statistics.median(row["elapsed_ns"] / row["iterations"] / 1000 for row in rows))
        for variant in ("node", "rust_baseline", "rust"):
            samples = [sample for row in report["http"] if row["case"] == case and row["variant"] == variant
                       and row["mode"] == "new_connection" for sample in row["ns"]]
            values.append(statistics.median(samples) / 1_000_000)
        info = report["case_info"][case]
        lines.append("| " + case + " | " + info["protocol"] + " | " + info["format"] + " | " +
                     " | ".join(f"{value:.3f}" for value in values) + " |")
    lines += ["", "Reused HTTP connections; same full endpoint, seven blocks of 30 requests per backend.",
              "Connection identity is checked; no pipeline or result cache is bypassed.",
              "| Case | Node HTTP ms | Previous Rust HTTP ms | Rust HTTP ms |", "|---|---:|---:|---:|"]
    for case in report["cases"]:
        values = [statistics.median(sample for row in report["http"] if row["case"] == case and row["variant"] == variant
                                    and row["mode"] == "keep_alive" for sample in row["ns"]) / 1e6
                  for variant in ("node", "rust_baseline", "rust")]
        lines.append("| " + case + " | " + " | ".join(f"{value:.3f}" for value in values) + " |")
    if report["profiles"]:
        groups = {group["case"]: group["events"] for group in report["profiles"]}
        lines += ["", "Separate Rust profiles: median active ms over seven requests per format.",
                  "Profiler-on samples do not enter the latency table. Parent/child stages overlap; active time is not OS CPU time.",
                  "", "| Stage | SS URI | Surge SS | QX SS |", "|---|---:|---:|---:|"]
        for stage in ("executor_setup", "state_snapshot", "runtime_setup", "context_setup", "job", "bootstrap",
                      "bootstrap_values", "bootstrap_load", "bootstrap_eval", "bootstrap_state_encode", "bootstrap_state_parse",
                      "parse_runtime", "parse_context_init", "parse_utilities_init", "parse_scalar_probe",
                      "parse_native", "produce", "output_encode", "runtime_teardown"):
            values = []
            for case in ("ss_uri", "surge_ss", "qx_ss"):
                samples = [sum(row["active_ns"] for row in event["rows"] if row["stage"] == stage) / 1e6 for event in groups[case]]
                values.append(f"{statistics.median(samples):.3f}")
            lines.append("| " + stage + " | " + " | ".join(values) + " |")
        lines += ["", "Complete request components (separate profiler-on medians, us).",
                  "Worker lifetime includes queueing and teardown; the response can be delivered before teardown finishes.",
                  "Server time stops when the response is ready, before socket transmission. Client time includes HTTP transport and client work.",
                  "These intervals overlap; do not add them or subtract independent medians to infer exact overhead.",
                  "| Case | Queue | Worker lifetime | Bootstrap | Native parse | Produce | Teardown | Server response ready | Client HTTP |",
                  "|---|---:|---:|---:|---:|---:|---:|---:|---:|"]
        for group in report["profiles"]:
            values = []
            for stage in ("worker_queue", None, "bootstrap", "parse_native", "produce", "runtime_teardown"):
                samples = [event["wall_ns"] if stage is None else
                           sum(row["wall_ns"] for row in event["rows"] if row["stage"] == stage)
                           for event in group["events"]]
                values.append(statistics.median(samples) / 1000)
            values += [statistics.median(group[key]) / 1000 for key in ("server_ns", "client_ns")]
            lines.append("| " + group["case"] + " | " + " | ".join(f"{value:.3f}" for value in values) + " |")
        lines += ["", "Generic-parser setup per request (separate profiles).",
                  "| Format | Median context calls | Context active ms | Median library calls | Library active ms |",
                  "|---|---:|---:|---:|---:|"]
        for group in report["profiles"]:
            values = []
            for stage in ("parse_context_init", "parse_utilities_init"):
                rows = [[row for row in event["rows"] if row["stage"] == stage] for event in group["events"]]
                calls = [sum(row["calls"] for row in sample) for sample in rows]
                times = [sum(row["active_ns"] for row in sample) / 1e6 for sample in rows]
                values += [f"{statistics.median(calls):.0f}", f"{statistics.median(times):.3f}"]
            lines.append("| " + group["case"] + " | " + " | ".join(values) + " |")
    return "\n".join(lines) + "\n"


def main():
    if os.environ.get("GITHUB_ACTIONS") != "true":
        raise SystemExit("Run in GitHub Actions only")
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("original", "rust", "rust-baseline", "rust-test", "rust-baseline-test", "scratch", "output"):
        parser.add_argument("--" + name, required=True, type=lambda value: Path(value).resolve())
    args = parser.parse_args()
    args.scratch.mkdir(parents=True)
    args.output.mkdir(parents=True)
    args.frontend = args.scratch / "unused-frontend"
    (args.frontend / "dist").mkdir(parents=True)
    fixture = Path(__file__).with_name("single-node.json").resolve()
    cases = json.loads(fixture.read_bytes())
    assert len({case["id"] for case in cases}) == len(cases)
    source = (args.original / "backend/src/core/proxy-utils/parsers/index.js").read_text()
    inventory = re.search(r"function Clash_All\(\).*?!\[(.*?)\]\.includes\(proxy\.type\)", source, re.S)
    assert inventory, "pinned Clash protocol inventory not found"
    clash_types = re.findall(r"'([^']+)'", inventory[1])
    covered = {json.loads(case["raw"])["type"] for case in cases if case["format"] == "Clash JSON"}
    assert covered == set(clash_types), f"protocol coverage: missing={set(clash_types) - covered}, extra={covered - set(clash_types)}"
    assert {case["protocol"] for case in cases} == covered | {"external"}
    assert any(case["protocol"] == "external" and case["format"] == "Surge" for case in cases)
    report = {"version": 1, "original": b.ORIGINAL, "rust": b.command(["git", "rev-parse", "HEAD"], args.rust).decode().strip(),
              "fixture_sha256": hashlib.sha256(fixture.read_bytes()).hexdigest(), "machine": platform.machine(),
              "clash_types": clash_types,
              "case_info": {case["id"]: {key: case[key] for key in ("protocol", "format")} for case in cases},
              "cases": [case["id"] for case in cases], "pure": [], "http": [], "profiles": [], "status": "running"}
    assert b.command(["git", "rev-parse", "HEAD"], args.original).decode().strip() == b.ORIGINAL
    report["rust_baseline"] = b.command(["git", "rev-parse", "HEAD"], args.rust_baseline).decode().strip()
    assert report["rust_baseline"] == "274a4ff1f63fdbe997dda0e0dcb99daf68135d06"
    report["binary_sha256"] = {kind: hashlib.sha256((root / "rust/target/release/sub-store-rust").read_bytes()).hexdigest()
                               for kind, root in (("rust", args.rust), ("rust_baseline", args.rust_baseline))}
    assert len(set(report["binary_sha256"].values())) == 2, "candidate and baseline must be distinct binaries"
    print("SINGLE_NODE_BINARIES " + json.dumps(report["binary_sha256"]), flush=True)
    suite = b.Suite(args, {})
    try:
        pure(args, suite, fixture, cases, report)
        endpoint(args, suite, cases, report)
        profiles(args, suite, cases, report)
        report["status"] = "passed"
        text = summary(report)
        (args.output / "summary.md").write_text(text)
        print(text, flush=True)
        for case in report["cases"]:
            samples = {variant: [round(row["elapsed_ns"] / row["iterations"] / 1000, 3)
                                for row in report["pure"] if row["case"] == case and row["variant"] == variant]
                       for variant in VARIANTS}
            samples["http_p95_ms"] = {variant: sorted(value / 1e6 for row in report["http"]
                                                   if row["case"] == case and row["variant"] == variant
                                                   and row["mode"] == "new_connection" for value in row["ns"])[math.ceil(BLOCKS * HTTP_REQUESTS * .95) - 1]
                                      for variant in ("node", "rust_baseline", "rust")}
            samples["keep_alive_p95_ms"] = {variant: sorted(value / 1e6 for row in report["http"]
                                                         if row["case"] == case and row["variant"] == variant
                                                         and row["mode"] == "keep_alive" for value in row["ns"])[math.ceil(BLOCKS * HTTP_REQUESTS * .95) - 1]
                                            for variant in ("node", "rust_baseline", "rust")}
            print("SINGLE_NODE_SAMPLES " + json.dumps({"case": case, **samples}), flush=True)
    except Exception:
        report["status"] = "failed"
        raise
    finally:
        (args.output / "metrics.json").write_text(json.dumps(report, indent=2) + "\n")


if __name__ == "__main__":
    main()
