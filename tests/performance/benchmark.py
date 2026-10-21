"""Actions-only comparison of the supplied subscription, with private runner scratch."""
import argparse
import asyncio
import base64
from concurrent.futures import ThreadPoolExecutor
from contextlib import contextmanager
import copy
import difflib
import gzip
import hashlib
import io
from http.client import HTTPConnection
import json
import math
import os
from pathlib import Path
import random
import re
import resource
import shutil
import signal
import socket
import statistics
import subprocess
import sys
import tarfile
import threading
import time
import traceback
from urllib.parse import quote, urlencode, urlsplit
from urllib.request import urlopen

import psutil

ORIGINAL = "9cc6b5fac966993364ae33da8797f446bbf7f571"
RUST = "37c6d9c415983a1d977a4b91be083f7aa70ed642"
RUST_BASELINE = "887ddccb36f7724a831a89b4aad416e9c97c7c51"
FRONTEND = "4bd7b0acbe65466b933ccea14fc61dbebe036ead"
INPUT_HASH = "a6e6bb9cbe0587f33de0cf130ec391ecffd3483009888fed0ea59d39492ae890"
INPUT_NAME = "sub-store_subscription_大杂烩-cron-new_2026-10-20_14-11-50.json"
SCRIPT_BLOB = "5fc44546b85b965abaa710924e507fae3f50a83d"
SCRIPT_HASH = "03d4c889ddec7a56576eb14f191e5187a7571d8122bc987431bb4eb48b3b1da8"
WATCHDOG = 300
KINDS = ("node", "rust")
COMPARISON_KINDS = ("node", "rust", "rust_baseline", "rust_repeat")
CASES = ("import", "cold", "warm", "pressure", "real", "parse", "pre_health", "no_publisher")
MATRIX = (("parse", 3), ("pre_health", 1), ("cold", 10), ("warm", 5), ("pressure", 3), ("no_publisher", 3))
SHARDS = {f"{case}_{block}_{clients}": (case, block, clients)
          for case, blocks in MATRIX for clients in ((1, 2, 4, 8) if case == "pressure" else (1,))
          for block in range(blocks)}
SHARDS["real_0_1"] = ("real", 0, 1)
ERRORS = {
    "input", "asset", "source", "startup", "http", "browser", "import_fields",
    "import_network", "persistence", "capture", "script_anchor", "empty_output",
    "output_mismatch", "snapshot", "work_incomplete", "warm_miss", "timeout",
    "fixture", "privacy", "unexpected", "clash", "resource", "external_io",
}
PUBLIC_STRINGS = set(COMPARISON_KINDS + CASES) | ERRORS | {
    "passed", "failed", "running", "fixture", "real_network", "empty_or_failed",
    "nonempty", "linux_x64", "not_run", "warming", "unpaired", "partial_sources",
    "parse_diagnostic", "health_diagnostic",
}
PUBLIC_STRINGS.update(SHARDS)
PROFILE_STAGES = set("""
executor_setup state_snapshot runtime_setup context_setup job bootstrap input_decode execute output_encode
sources pipeline_parse pipeline_process pipeline_produce pipeline_utility process processor
script_operator script_filter useless_filter region_filter regex_filter type_filter conditional_filter
quick_setting_operator flag_operator sort_operator regex_sort_operator regex_rename_operator regex_delete_operator
handle_duplicate_operator add_proxies_operator resolve_domain_operator unknown_processor
builtin_prepare builtin_apply script_load script_prepare script_compile script_bind script_call script_await script_fallback
response_transformer response_normalize
input_clone clone_stringify clone_parse parse parse_runtime parse_native parse_preprocess parse_nodes
produce produce_validate produce_filter filter_callback produce_prepare prepare_callback produce_collect
produce_project_nodes produce_project_options produce_reconcile produce_native produce_decode
value_project project_stringify project_extract project_parse project_restore value_stringify
value_decode decode_classify decode_encode decode_parse decode_restore decode_build
""".split())
PUBLIC_STRINGS.update(PROFILE_STAGES | {"download_sub", "download_collection", "download_file", "other"})
NODE_FIELDS = {"name", "type", "server", "port", "ports", "password", "uuid", "sni", "tls",
               "_subName", "_subDisplayName", "_latency", "_resolved", "resolved", "id",
               "network", "cipher", "udp", "tfo", "skip-cert-verify", "client-fingerprint",
               "ws-opts", "reality-opts", "grpc-opts", "http-opts", "alpn", "servername"}
PUBLIC_STRINGS.update(NODE_FIELDS | {"missing", "object", "array", "string", "number", "boolean", "null"})


class Failure(Exception):
    def __init__(self, code):
        assert code in ERRORS
        self.code = code
        super().__init__(code)


def require(condition, code):
    if not condition:
        raise Failure(code)


def digest(value):
    return hashlib.sha256(value if isinstance(value, bytes) else encoded(value)).hexdigest()


def encoded(value):
    return json.dumps(value, ensure_ascii=False, sort_keys=True, separators=(",", ":")).encode()


def write_json(path, value):
    path.write_bytes(encoded(value))


def public(value):
    """Reports accept only code-owned enums, hashes, versions and numeric observations."""
    if value is None or isinstance(value, (bool, int)):
        return
    if isinstance(value, float):
        require(math.isfinite(value), "privacy")
    elif isinstance(value, str):
        require(value in PUBLIC_STRINGS or re.fullmatch(r"[0-9a-f]{40}|[0-9a-f]{64}|\d+(?:\.\d+){1,3}", value), "privacy")
    elif isinstance(value, list):
        for item in value:
            public(item)
    elif isinstance(value, dict):
        for key, item in value.items():
            require(re.fullmatch(r"[a-z][a-z0-9_]*", key) is not None, "privacy")
            public(item)
    else:
        raise Failure("privacy")


def pipeline_profiles(path, offset):
    """Read only opt-in numeric events, never publish surrounding private logs."""
    reports = []
    with path.open("rb") as source:
        source.seek(offset)
        for line in source:
            if not line.startswith(b"SUB_STORE_PROFILE "):
                continue
            try:
                report = json.loads(line[len(b"SUB_STORE_PROFILE "):])
            except (ValueError, UnicodeError) as error:
                raise Failure("resource") from error
            public(report)
            require(set(report) == {"version", "job", "operation", "success", "wall_ns", "dropped", "open_scopes", "unfinished_spans", "rows"}, "resource")
            require(report["version"] == 1 and report["operation"] == "download_sub" and report["success"] is True, "resource")
            require(report["dropped"] == report["open_scopes"] == report["unfinished_spans"] == 0, "resource")
            rows = report["rows"]
            require(isinstance(rows, list) and 0 < len(rows) <= 2048, "resource")
            for index, row in enumerate(rows):
                require(set(row) == {"id", "parent", "stage", "index", "calls", "wall_ns", "active_ns", "self_ns", "max_wall_ns"}, "resource")
                require(row["id"] == index and (row["parent"] is None or type(row["parent"]) is int and 0 <= row["parent"] < index), "resource")
                require(row["stage"] in PROFILE_STAGES and (row["index"] is None or type(row["index"]) is int and row["index"] >= 0), "resource")
                require(all(type(row[key]) is int and row[key] >= 0 for key in ("calls", "wall_ns", "active_ns", "self_ns", "max_wall_ns")), "resource")
                require(row["calls"] > 0 and row["self_ns"] <= row["active_ns"] <= row["wall_ns"] and row["max_wall_ns"] <= row["wall_ns"], "resource")
            require(sum(row["self_ns"] for row in rows) == sum(row["active_ns"] for row in rows if row["parent"] is None), "resource")
            require(any(row["stage"] == "processor" for row in rows), "resource")
            reports.append(report)
    return reports


def pipeline_summary(samples):
    lines = []
    for sample in samples:
        profiles = sample.get("pipeline_profiles", [])
        if not profiles:
            continue
        lines += ["", f"### Pipeline profile: {sample['case']} ({len(profiles)} requests)", "",
                  "`rust_repeat` enables the profiler on the same candidate binary. Its latency includes instrumentation and report writing. Active time measures elapsed time in scoped polls/calls, not OS CPU time. Wall time includes waiting; concurrent wall times overlap. Self time excludes nested active scopes. The complete parent/index tree is in metrics.json.", "",
                  "| Process | Processor (zero based) | Type | Mean wall ms | Mean active ms |", "|---:|---:|---|---:|---:|"]
        processors, stages = {}, {}
        for profile in profiles:
            rows = profile["rows"]
            for row in rows:
                total = stages.setdefault(row["stage"], [0, 0, 0])
                for index, key in enumerate(("calls", "active_ns", "self_ns")):
                    total[index] += row[key]
                if row["stage"] != "processor":
                    continue
                parent = row["parent"]
                while parent is not None and rows[parent]["stage"] != "process":
                    parent = rows[parent]["parent"]
                process_index = rows[parent]["index"] if parent is not None else None
                kind = next((child["stage"] for child in rows if child["parent"] == row["id"]), "skipped")
                total = processors.setdefault((process_index, row["index"], kind), [0, 0])
                total[0] += row["wall_ns"]
                total[1] += row["active_ns"]
        scale = len(profiles) * 1e6
        for (process_index, index, kind), (wall, active) in sorted(processors.items(), key=lambda item: str(item[0])):
            lines.append(f"| {process_index} | {index} | {kind} | {wall / scale:.3f} | {active / scale:.3f} |")
        lines += ["", "Top stages by mean self time (inclusive active columns must not be added):", "",
                  "| Stage | Mean calls/request | Mean active ms | Mean self ms |", "|---|---:|---:|---:|"]
        for stage, (calls, active, own) in sorted(stages.items(), key=lambda item: item[1][2], reverse=True)[:20]:
            lines.append(f"| {stage} | {calls / len(profiles):.1f} | {active / scale:.3f} | {own / scale:.3f} |")
    return lines


def semantic_nodes(nodes, hopping=False):
    result = [{k: v for k, v in item.items() if k != "_latency"} for item in nodes]
    if hopping:
        # Parse-only exports randomly choose a port; retain the declaration and reject out-of-range choices.
        for item in result:
            ports, port = item.get("ports"), item.get("port")
            if isinstance(ports, str) and re.fullmatch(r"\d+(?:-\d+)?(?:[,/]\d+(?:-\d+)?)*", ports):
                ranges = [tuple(map(int, part.split("-"))) for part in re.split(r"[,/]", ports)]
                bounds = [(part[0], part[-1]) for part in ranges]
                if all(1 <= low <= high <= 65535 for low, high in bounds) and type(port) is int and any(low <= port <= high for low, high in bounds):
                    item["port"] = bounds[0][0]
    return result


def nodes_hash(nodes, hopping=False):
    return digest(semantic_nodes(nodes, hopping))


def compare_nodes(expected, actual, code="output_mismatch", hopping=False):
    if nodes_hash(expected, hopping) == nodes_hash(actual, hopping):
        return
    missing, differences = object(), []
    def kind(value):
        return "missing" if value is missing else "null" if value is None else "boolean" if isinstance(value, bool) else "object" if isinstance(value, dict) else "array" if isinstance(value, list) else "string" if isinstance(value, str) else "number"
    def visit(left, right, path):
        if len(differences) >= 20 or left == right:
            return
        if isinstance(left, dict) and isinstance(right, dict):
            for key in sorted(left.keys() | right.keys()):
                if not (len(path) == 1 and key == "_latency"):
                    visit(left.get(key, missing), right.get(key, missing), path + [key if key in NODE_FIELDS else digest(key.encode())])
        elif isinstance(left, list) and isinstance(right, list):
            for index in range(max(len(left), len(right))):
                visit(left[index] if index < len(left) else missing, right[index] if index < len(right) else missing, path + [index])
        else:
            differences.append({"path": path, "expected_type": kind(left), "actual_type": kind(right),
                                "expected_sha256": digest(left) if left is not missing else None,
                                "actual_sha256": digest(right) if right is not missing else None})
    visit(semantic_nodes(expected, hopping), semantic_nodes(actual, hopping), [])
    error = Failure(code)
    error.observations = {"expected_count": len(expected), "actual_count": len(actual), "differences": differences}
    raise error


def import_transform(original, payload):
    suffix = payload.get("name", "")[len(original["name"]):]
    require(re.fullmatch(r"_\d{13}", suffix) is not None, "import_fields")
    expected = {**original, "name": original["name"] + suffix,
                "displayName": original["displayName"] + suffix,
                "display-name": original["displayName"] + suffix}
    require(payload == expected, "import_fields")


def stress_script(source):
    replacements = {
        "const concurrency = Math.floor(num('concurrency', 150, 1, 256));": "const concurrency = Math.max(1, proxies.length);",
        "const batchSize = Math.floor(num('batch_size', 200, 1, 500));": "const batchSize = Math.max(1, proxies.length);",
        "const maxChecks = Math.floor(num('max_checks', 600, 0, 5000));": "const maxChecks = Math.max(1, proxies.length);",
        "const maxRunMs = Math.floor(num('max_run_ms', 90000, 10000, 600000));": "const maxRunMs = Number.POSITIVE_INFINITY;",
        "const leaseMs = Math.max(60000, startDelay + nWaves * Math.max(oneProbeMax, proxyLeaseTimeout) + 45000);": "const leaseMs = 600000;",
    }
    for old, new in replacements.items():
        require(source.count(old) == 1, "script_anchor")
        source = source.replace(old, new)
    return source


def profile_health_script(source, *, enabled=True):
    """Diagnostic-only wrappers retain calls/results and never alter the full-run script."""
    if not enabled:
        return source
    return source + r"""
const benchmarkOperator = operator;
operator = async function (...args) {
  const profile = {}, restore = [], began = Date.now();
  function measure(owner, key, name) {
    const original = owner[key], calls = name + '_calls', elapsed = name + '_ms';
    profile[calls] = profile[elapsed] = 0;
    owner[key] = function (...values) {
      const start = Date.now();
      profile[calls]++;
      try { return Reflect.apply(original, this, values); }
      finally { profile[elapsed] += Date.now() - start; }
    };
    restore.push(() => { owner[key] = original; });
  }
  measure(ProxyUtils, 'hex_md5', 'md5');
  measure(ProxyUtils, 'produce', 'produce');
  measure(scriptResourceCache, 'get', 'cache_get');
  measure(scriptResourceCache, 'set', 'cache_set');
  try { return await Reflect.apply(benchmarkOperator, this, args); }
  finally {
    profile.total_ms = Date.now() - began;
    for (const reset of restore) reset();
    $substore.write(profile, '#benchmark-health-profile');
  }
};
"""


def distribution(values):
    ordered = sorted(values)
    if not ordered:
        return {"n": 0}
    result = {"n": len(ordered), "median": statistics.median(ordered),
              "min": ordered[0], "max": ordered[-1],
              "mean": statistics.mean(ordered), "stdev": statistics.stdev(ordered) if len(ordered) > 1 else 0}
    if len(ordered) >= 20:
        result["p95"] = ordered[math.ceil(len(ordered) * .95) - 1]
    return result


def paired_interval(pairs):
    require(all(a > 0 and b > 0 for a, b in pairs), "resource")
    logs = [math.log(a / b) for a, b in pairs]
    rng = random.Random(20261020)
    boots = sorted(math.exp(statistics.mean(rng.choices(logs, k=len(logs)))) for _ in range(2000))
    return {"pairs": len(pairs), "node_over_rust": math.exp(statistics.mean(logs)),
            "bootstrap_low": boots[49], "bootstrap_high": boots[1949]}


def command(argv, cwd=None, timeout=90, stdin=None):
    rtk = shutil.which("rtk")
    result = subprocess.run(([rtk, "proxy"] if rtk else []) + list(map(str, argv)),
                            cwd=cwd, input=stdin, capture_output=True, timeout=timeout)
    require(result.returncode == 0, "source")
    return result.stdout


def http(url, method="GET", data=None, timeout=WATCHDOG):
    parsed = urlsplit(url)
    connection = HTTPConnection(parsed.hostname, parsed.port, timeout=timeout)
    body = encoded(data) if data is not None else None
    try:
        connection.request(method, parsed.path + ("?" + parsed.query if parsed.query else ""), body,
                           {"Content-Type": "application/json", "Connection": "close"} if body is not None else {"Connection": "close"})
        response = connection.getresponse()
        raw = response.read()
        if not 200 <= response.status < 300:
            error = Failure("http")
            text = raw.decode("utf-8", errors="replace")
            error.observations = {"http_status": response.status, "response_sha256": digest(raw),
                                  "empty_nodes": "不含有效节点" in text,
                                  "detector_start_failed": "HTTP-META /start failed" in text,
                                  "cache_unavailable": "scriptResourceCache unavailable" in text,
                                  "snapshot_error": "HealthSnapshot" in text or "Snapshot" in text}
            raise error
        return raw
    except (TimeoutError, socket.timeout):
        raise Failure("timeout") from None
    finally:
        connection.close()


def api(origin, path, method="GET", data=None):
    payload = json.loads(http(origin + "/backend/api/" + path, method, data))
    require(payload.get("status") == "success", "http")
    return payload.get("data")


def free_port():
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        return sock.getsockname()[1]


@contextmanager
def process(argv, directory, env, log):
    # Own the actual PID: a wrapper would corrupt accounting and process cleanup.
    with log.open("ab") as stream:
        child = subprocess.Popen(list(map(str, argv)), cwd=directory, env=env,
                                 stdout=stream, stderr=stream, start_new_session=True)
        try:
            yield child
        finally:
            os.killpg(child.pid, signal.SIGTERM) if child.poll() is None else None
            try:
                child.wait(timeout=5)
            except subprocess.TimeoutExpired:
                os.killpg(child.pid, signal.SIGKILL)
                child.wait(timeout=5)


def ready(child, url):
    until = time.monotonic() + 40
    while child.poll() is None and time.monotonic() < until:
        try:
            http(url, timeout=1)
            return
        except (OSError, Failure):
            time.sleep(.05)
    raise Failure("startup")


def cpu(proc):
    value = proc.cpu_times()
    return value.user + value.system


class Meter:
    def __init__(self, child, fixture, auxiliary=None):
        self.proc = psutil.Process(child.pid)
        self.fixture = psutil.Process(fixture.pid) if fixture else None
        self.driver = psutil.Process()
        self.auxiliary = psutil.Process(auxiliary.pid) if auxiliary else None
        self.aux_peak = 0
        self.trace = []
        self.stop = threading.Event()
        self.thread = threading.Thread(target=self.sample, daemon=True)

    def sample(self):
        started = time.thread_time()
        while not self.stop.is_set():
            at = time.monotonic()
            self.trace.append([at, self.proc.memory_info().rss])
            if self.auxiliary:
                try:
                    self.aux_peak = max(self.aux_peak, sum(p.memory_info().rss for p in [self.auxiliary, *self.auxiliary.children(recursive=True)] if p.is_running()))
                except psutil.NoSuchProcess:
                    pass
            self.stop.wait(.05)
        self.sampler_cpu_s = time.thread_time() - started

    def __enter__(self):
        self.before_cpu = cpu(self.proc)
        self.before_driver = cpu(self.driver)
        self.before_fixture = cpu(self.fixture) if self.fixture else 0
        self.before_io = self.proc.io_counters()
        self.before_aux = self.aux_cpu()
        self.started = time.monotonic()
        self.thread.start()
        return self

    def __exit__(self, *_):
        self.elapsed = time.monotonic() - self.started
        self.after_cpu = cpu(self.proc)
        self.after_driver = cpu(self.driver)
        self.after_fixture = cpu(self.fixture) if self.fixture else 0
        self.after_aux = self.aux_cpu()
        self.after_io = self.proc.io_counters()
        self.stop.set()
        self.thread.join()

    def result(self, completed):
        peak = re.search(r"^VmHWM:\s+(\d+) kB", Path(f"/proc/{self.proc.pid}/status").read_text(), re.M)
        intervals = [b[0] - a[0] for a, b in zip(self.trace, self.trace[1:])]
        seconds = self.after_cpu - self.before_cpu
        end_rss = self.proc.memory_info().rss
        time.sleep(.25)
        return {"elapsed_s": self.elapsed, "cpu_s": seconds,
                "cpu_s_per_request": seconds / completed if completed else None,
                "rps": completed / self.elapsed, "completed": completed,
                "rss_peak_bytes": max((row[1] for row in self.trace), default=0),
                "rss_end_bytes": end_rss, "rss_settled_bytes": self.proc.memory_info().rss,
                "os_lifetime_peak_bytes": int(peak[1]) * 1024 if peak else None,
                "io_read_bytes": self.after_io.read_bytes - self.before_io.read_bytes,
                "io_write_bytes": self.after_io.write_bytes - self.before_io.write_bytes,
                "sampler_cpu_s": self.sampler_cpu_s,
                "driver_cpu_s": self.after_driver - self.before_driver,
                "fixture_cpu_s": self.after_fixture - self.before_fixture if self.fixture else 0,
                "fixture_rss_bytes": self.fixture.memory_info().rss if self.fixture else 0,
                "detector_cpu_s": self.after_aux - self.before_aux,
                "detector_rss_peak_bytes": self.aux_peak,
                "sample_interval_s": distribution(intervals),
                "rss_trace": [[at - self.started, rss] for at, rss in self.trace]}

    def aux_cpu(self):
        if not self.auxiliary:
            return 0
        at = self.auxiliary.cpu_times()
        # Linux waited-child counters retain Mihomo CPU after /stop reaps it.
        return at.user + at.system + at.children_user + at.children_system + sum(cpu(p) for p in self.auxiliary.children(recursive=True) if p.is_running())


class Fixture:
    """One asynchronous child owns mirrors and a controlled or forwarded detector."""
    def __init__(self, directory, port, meta_port):
        self.directory, self.port, self.meta_port = directory, port, meta_port
        self.batches = {}
        self.next_pid = 1
        self.reset()
        self.real = False

    def reset(self):
        self.counts = {"sources": 0, "scripts": 0, "starts": 0, "stops": 0,
                       "selected": 0, "probes": 0, "unique_probes": 0,
                       "active": 0, "max_active": 0}

    async def handle(self, reader, writer, identity=None):
        try:
            header = (await asyncio.wait_for(reader.readuntil(b"\r\n\r\n"), 30)).decode("latin1")
            first, *lines = header.split("\r\n")
            method, path, _ = first.split(" ", 2)
            headers = {k.lower(): v.strip() for line in lines if ":" in line for k, v in [line.split(":", 1)]}
            body = await reader.readexactly(int(headers.get("content-length", 0)))
            status, output = 200, b""
            if identity is not None:
                self.counts["active"] += 1
                self.counts["max_active"] = max(self.counts["max_active"], self.counts["active"])
                self.counts["probes"] += 1
                if not identity[1]:
                    self.counts["unique_probes"] += 1
                    identity[1] = True
                await asyncio.sleep(.001)
                self.counts["active"] -= 1
                status = 204
            elif path == "/counts":
                output = encoded(self.counts)
            elif path == "/reset":
                require(not self.batches, "fixture")
                self.reset()
                self.real = bool(json.loads(body or b"{} ").get("real"))
                output = b"{}"
            elif path.startswith("/source/"):
                self.counts["sources"] += 1
                output = (self.directory / (path.rsplit("/", 1)[1] + ".txt")).read_bytes()
            elif path == "/health.js":
                self.counts["scripts"] += 1
                output = (self.directory / "stress.js").read_bytes()
            elif path == "/start":
                data = json.loads(body)
                self.counts["starts"] += 1
                self.counts["selected"] += len(data["proxies"])
                if self.real:
                    output = await asyncio.to_thread(http, f"http://127.0.0.1:{self.meta_port}/start", "POST", data, 60)
                else:
                    pid = self.next_pid
                    self.next_pid += 1
                    servers, ports = [], []
                    self.batches[pid] = servers
                    for index in range(len(data["proxies"])):
                        marker = [index, False]
                        async def probe(r, w, marker=marker):
                            await self.handle(r, w, marker)
                        server = await asyncio.start_server(probe, "127.0.0.1", 0, backlog=4096)
                        servers.append(server)
                        ports.append(server.sockets[0].getsockname()[1])
                    output = encoded({"pid": pid, "ports": ports})
            elif path == "/stop":
                self.counts["stops"] += 1
                if self.real:
                    output = await asyncio.to_thread(http, f"http://127.0.0.1:{self.meta_port}/stop", "POST", json.loads(body), 30)
                else:
                    for pid in json.loads(body)["pid"]:
                        for server in self.batches.pop(pid):
                            server.close()
                            await server.wait_closed()
                    output = b"{}"
            else:
                status = 404
            writer.write(f"HTTP/1.1 {status} OK\r\nContent-Length: {len(output)}\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n".encode() + output)
            await writer.drain()
        except Exception:
            writer.write(b"HTTP/1.1 500 Error\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
        finally:
            writer.close()
            await writer.wait_closed()

    async def serve(self):
        server = await asyncio.start_server(self.handle, "127.0.0.1", self.port, limit=1024 * 1024, backlog=4096)
        async with server:
            await server.serve_forever()


class Browser:
    def __init__(self, directory, name):
        self.directory, self.name, self.index = directory, name, 0

    def run(self, *arguments):
        self.index += 1
        argv = [shutil.which("agent-browser"), "--session", self.name, "--json", *map(str, arguments)]
        rtk = shutil.which("rtk")
        with (self.directory / f"browser-{self.index}.out").open("wb") as out, (self.directory / f"browser-{self.index}.err").open("wb") as err:
            result = subprocess.run(([rtk, "proxy"] if rtk else []) + argv, stdout=out, stderr=err, timeout=60)
        require(result.returncode == 0, "browser")
        reply = json.loads((self.directory / f"browser-{self.index}.out").read_bytes())
        require(reply.get("success"), "browser")
        return reply["data"]

    def evaluate(self, expression):
        return self.run("eval", "-b", base64.b64encode(expression.encode()).decode())["result"]


class Suite:
    def __init__(self, args, report):
        self.args, self.report = args, report
        self.env = {key: value for key, value in os.environ.items()
                    if not key.startswith(("SUB_STORE_", "AGENT_BROWSER_", "BENCHMARK_", "HTTP_PROXY", "HTTPS_PROXY", "ALL_PROXY"))}
        self.env.update(NODE_ENV="production", TZ="UTC", NO_PROXY="127.0.0.1,localhost")
        self.node = Path(shutil.which("node")).resolve()
        self.fixture = None
        self.fixture_origin = None
        self.pristine = {}
        self.expected_hashes = {}
        self.expected_outputs = {}
        self.expected_snapshot_hash = None
        self.expected_snapshot = None
        self.clash_hash = None
        self.ordinal = 0
        self.meta = None

    def directory(self, label):
        self.ordinal += 1
        directory = self.args.scratch / f"{self.ordinal}-{label}"
        directory.mkdir()
        return directory

    @contextmanager
    def server(self, kind, directory, clone=True):
        state = directory / "state"
        if clone:
            shutil.copytree(self.pristine["node" if kind == "node" else "rust"], state)
        else:
            state.mkdir()
        port = free_port()
        origin = f"http://127.0.0.1:{port}"
        env = {**self.env, "SUB_STORE_DATA_BASE_PATH": str(state),
               "SUB_STORE_BACKEND_API_HOST": "127.0.0.1", "SUB_STORE_BACKEND_API_PORT": str(port),
               "SUB_STORE_FRONTEND_PATH": str(self.args.frontend / "dist"),
               "SUB_STORE_FRONTEND_BACKEND_PATH": "/backend", "SUB_STORE_BACKEND_MERGE": "true",
               "SUB_STORE_CORS_ALLOWED_ORIGINS": origin}
        if kind == "node":
            argv = [self.node, self.args.original / "backend/dist/sub-store.bundle.js"]
            cwd = self.args.original / "backend"
        else:
            cwd = self.args.rust_baseline if kind == "rust_baseline" else self.args.rust
            argv = [cwd / "rust/target/release/sub-store-rust"]
            env["PATH"] = ""
            if kind == "rust_repeat" and getattr(self.args, "profile_pipeline", False):
                env["SUB_STORE_PROFILE"] = "1"
        started = time.monotonic()
        with process(argv, cwd, env, directory / "server.log") as child:
            ready(child, origin + "/backend/api/utils/env")
            startup = time.monotonic() - started
            yield origin, child, state, startup

    def imports(self, original, input_file):
        for block in range(5):
            for kind in (KINDS if block % 2 == 0 else KINDS[::-1]):
                directory = self.directory("import-" + kind)
                with self.server(kind, directory, clone=False) as (origin, child, state, startup):
                    browser = Browser(directory, f"performance-{kind}-{block}")
                    try:
                        browser.run("open", "about:blank")
                        runtime = browser.evaluate("navigator.userAgent")
                        match = re.search(r"(?:HeadlessChrome|Chrome)/([\d.]+)", runtime)
                        require(match is not None, "browser")
                        self.report["chromium"] = match[1]
                        browser.run("network", "har", "start")
                        browser.run("open", origin + "/subs?api=" + quote(origin + "/backend", safe=""))
                        browser.run("wait", "--fn", "localStorage.getItem('envCache') !== null")
                        browser.run("wait", ".empty-state-wrapper .nut-button")
                        browser.run("click", ".empty-state-wrapper .nut-button")
                        browser.run("wait", ".add-sub-popup")
                        began = time.monotonic()
                        browser.run("upload", 'input[type="file"]', input_file)
                        until = time.monotonic() + 20
                        items = []
                        while time.monotonic() < until:
                            items = api(origin, "subs")
                            if items:
                                break
                            time.sleep(.05)
                        wall = time.monotonic() - began
                        require(isinstance(items, list) and len(items) == 1, "import_fields")
                        stored = items[0]
                        browser.run("wait", "--text", stored["displayName"])
                        browser.run("reload")
                        browser.run("wait", "--text", stored["displayName"])
                        browser.run("open", origin + "/edit/subs/" + quote(stored["name"], safe=""))
                        editor_check = "Array.from(document.querySelectorAll('input')).some(el => el.value === " + json.dumps(stored["name"]) + ")"
                        browser.run("wait", "--fn", editor_check)
                        require(browser.evaluate(editor_check), "import_fields")
                        har = directory / "import.har"
                        browser.run("network", "har", "stop", har)
                        entries = json.loads(har.read_bytes())["log"]["entries"]
                        posts = [e for e in entries if e["request"]["method"] == "POST" and urlsplit(e["request"]["url"]).path == "/backend/api/subs"]
                        require(len(posts) == 1 and posts[0]["response"]["status"] == 201, "import_network")
                        payload = json.loads(posts[0]["request"]["postData"]["text"])
                        import_transform(original, payload)
                        require(all(stored.get(k) == v for k, v in payload.items()), "import_fields")
                        document = json.loads((state / "sub-store.json").read_bytes())
                        require(document["subs"] == items, "persistence")
                        row = {"kind": kind, "case": "import", "block": block,
                               "startup_s": startup, "ui_import_wall_s": wall,
                               "api_import_s": posts[0]["time"] / 1000,
                               "payload_sha256": digest(payload), "processors": len(stored["process"]),
                               "fields": len(payload), "reload": True, "editor": True, "restart": False}
                        self.report["imports"].append(row)
                    finally:
                        browser.run("close")
                # A stopped, UI-created state is the only repetition seed.
                saved_seed = self.pristine.get(kind)
                self.pristine[kind] = state
                restart_dir2 = self.directory("restart-check-" + kind)
                with self.server(kind, restart_dir2) as (origin, child, state2, _):
                    require(api(origin, "subs") == items, "persistence")
                row["restart"] = True
                if saved_seed:
                    self.pristine[kind] = saved_seed
                self.publish()

    def configure(self, origin, original, fixture=True, variant="full"):
        stored = api(origin, "subs")
        require(len(stored) == 1, "import_fields")
        config = copy.deepcopy(original)
        if fixture:
            config["url"] = "\n".join(self.fixture_origin + f"/source/{index}" for index in range(22))
            linked = config["process"][4]["args"]
            linked["arguments"]["http_meta_port"] = str(urlsplit(self.fixture_origin).port)
            # Link mode reads URL fragments, not args.arguments (which is the UI editor value).
            linked["content"] = self.fixture_origin + "/health.js#" + quote(encoded(linked["arguments"]), safe="")
        if variant == "parse":
            config["process"] = []
        elif variant == "no_publisher":
            config["process"] = config["process"][:-1]
        elif variant == "pre_health":
            config["process"] = config["process"][:4]
        api(origin, "sub/" + quote(stored[0]["name"], safe=""), "PATCH", config)
        return config

    def download(self, origin, name, target="JSON"):
        return http(origin + "/backend/download/" + quote(name, safe="") + "?" + urlencode({"target": target}))

    def source_texts(self, origin, config):
        # produceType=raw already preprocesses YAML; context.raw retains the original text.
        inspection = {**config, "process": [{"type": "Script Operator", "args": {
            "mode": "script", "content": "function operator(proxies, target, context) { return context.raw; }"}}]}
        return api(origin, "preview/sub", "POST", inspection)["processed"]

    def capture(self, original):
        directory = self.directory("capture")
        with self.server("node", directory) as (origin, child, state, _):
            config = self.configure(origin, original, fixture=False)
            began = time.monotonic()
            sources = self.source_texts(origin, config)
            elapsed = time.monotonic() - began
            require(len(sources) == 23 and all(isinstance(s, str) for s in sources), "capture")
            require(sources[-1] == original["content"], "capture")
            require(any(sources[:22]), "capture")
            self.report["capture"] = {"elapsed_s": elapsed, "source_count": 22, "before_preprocessing": True,
                                      "nonempty": sum(bool(s) for s in sources[:22]),
                                      "empty_or_failed": sum(not s for s in sources[:22]),
                                      "sources": [{"index": i, "bytes": len(s.encode()), "sha256": digest(s.encode()),
                                                   "status": "nonempty" if s else "empty_or_failed"} for i, s in enumerate(sources[:22])]}
            for i, source in enumerate(sources[:22]):
                (self.args.scratch / "mirrors" / f"{i}.txt").write_bytes(source.encode())
        self.publish()

    def replay(self, original):
        expected = [(self.args.scratch / "mirrors" / f"{i}.txt").read_bytes().decode() for i in range(22)] + [original["content"]]
        self.report["replay"] = []
        for kind in KINDS:
            directory = self.directory("replay-" + kind)
            with self.server(kind, directory) as (origin, _, _, _):
                config = self.configure(origin, original)
                actual = self.source_texts(origin, config)
                require(isinstance(actual, list) and all(isinstance(s, str) for s in actual), "capture")
                self.report["replay"].append({"kind": kind, "expected_sha256": digest(expected),
                                              "actual_sha256": digest(actual), "equal": actual == expected,
                                              "source_count": len(actual),
                                              "matching_sources": sum(a == b for a, b in zip(expected, actual)),
                                              "timeout_log_entries": (directory / "server.log").read_text(encoding="utf-8", errors="replace").count("exceeds the timeout"),
                                              "sources": [{"index": i, "bytes": len(s.encode()), "sha256": digest(s.encode())}
                                                          for i, s in enumerate(actual)]})
        self.publish()
        require(all(row["equal"] for row in self.report["replay"]), "capture")

    def counts(self, reset=False, real=False):
        return json.loads(http(self.fixture_origin + ("/reset" if reset else "/counts"),
                               "POST" if reset else "GET", {"real": real} if reset else None))

    def audit(self, state, output, cold, counts, origin, real=False, cross_compare=True):
        root = json.loads((state / "root.json").read_bytes())
        cache = json.loads(root["sub-store-cached-script-resource"])
        health = [v["data"] for k, v in cache.items() if k.startswith("http-meta:health-inc:v1:")]
        snapshots = [(k, v["data"]) for k, v in cache.items() if k.startswith("health-snapshot:v1:")]
        require(len(health) == 1 and len(snapshots) == 1, "snapshot")
        (snapshot_key, snapshot), records = snapshots[0], health[0]["records"]
        require(snapshot["version"] == 1 and snapshot["count"] == len(output) > 0, "snapshot")
        require(isinstance(snapshot["generatedAt"], (int, float)) and 0 <= time.time() * 1000 - snapshot["generatedAt"] < WATCHDOG * 1000, "snapshot")
        snapshot_hash = nodes_hash(snapshot["proxies"])
        if not real:
            if self.expected_snapshot_hash is None:
                self.expected_snapshot_hash = snapshot_hash
                self.expected_snapshot = snapshot["proxies"]
            if cross_compare:
                compare_nodes(self.expected_snapshot, snapshot["proxies"])
        # Publication precedes the final producer (SNI/ports/etc.); validate using that actual producer.
        projected = api(origin, "preview/sub", "POST", {
            "name": "benchmark-snapshot-validation", "source": "local",
            "content": "ss://YWVzLTEyOC1nY206YmVuY2g@127.0.0.1:1#validation",
            "process": [{"type": "Script Operator", "args": {"mode": "script",
                "content": "function operator() { const s = scriptResourceCache.get($arguments.key); if (!s) throw new Error('missing snapshot'); return ProxyUtils.produce(JSON.parse(JSON.stringify(s.proxies)), 'JSON', 'internal'); }",
                "arguments": {"key": snapshot_key}}}]})
        require(nodes_hash(projected["processed"]) == nodes_hash(output), "snapshot")
        require(all(v.get("lastProbe", 0) > 0 for v in records.values()), "work_incomplete")
        if cold is True:
            require(counts["starts"] == counts["stops"] == 1 and counts["selected"] == len(records) > 0, "work_incomplete")
            if not real:
                # The original retry policy remains active; all identities must still be reached.
                require(counts["unique_probes"] == len(records) <= counts["probes"], "work_incomplete")
        elif cold is False:
            require(counts["starts"] == counts["selected"] == counts["probes"] == 0, "warm_miss")
        return {"identities": len(records), "published": snapshot["count"],
                "generated_at_ms": snapshot["generatedAt"],
                "snapshot_sha256": snapshot_hash,
                "verified": sum(v.get("lastOk", 0) > 0 for v in records.values())}

    def check_output(self, raw, compare=True, key="full"):
        output = json.loads(raw)
        require(isinstance(output, list) and len(output) > 0 and all(isinstance(v, dict) for v in output), "empty_output")
        fingerprint = nodes_hash(output, hopping=key == "parse")
        if compare:
            self.expected_hashes.setdefault(key, fingerprint)
            self.expected_outputs.setdefault(key, output)
            compare_nodes(self.expected_outputs[key], output, hopping=key == "parse")
        return output

    def diagnose_sources(self):
        expected = []
        self.report["direct_sources"] = []
        for kind in KINDS:
            with self.server(kind, self.directory("direct-" + kind)) as (origin, child, _, _):
                for index in range(22):
                    source = (self.args.scratch / "mirrors" / f"{index}.txt").read_bytes().decode()
                    before, began = cpu(psutil.Process(child.pid)), time.monotonic()
                    result = api(origin, "preview/sub", "POST", {
                        "name": "parse-diagnostic", "source": "local", "content": source, "process": []})
                    elapsed, used = time.monotonic() - began, cpu(psutil.Process(child.pid)) - before
                    output = result["processed"]
                    require(isinstance(output, list), "http")
                    row = {"kind": kind, "source_index": index, "input_bytes": len(source.encode()),
                           "elapsed_s": elapsed, "cpu_s": used, "output_count": len(output),
                           "output_sha256": nodes_hash(output), "status": "passed"}
                    self.report["direct_sources"].append(row)
                    if kind == "node":
                        expected.append(output)
                    else:
                        try:
                            compare_nodes(expected[index], output)
                        except Failure as error:
                            row.update(status="failed", failure=error.code,
                                       failure_observations=error.observations)
                    print(f"Direct parse {kind} source={index}: {elapsed:.3f}s, CPU={used:.3f}s, nodes={len(output)}, {row['status']}.", flush=True)
                    self.publish()

    def run_case(self, kind, case, block, original, concurrency=1, repeats=None):
        directory = self.directory(case + "-" + kind)
        self.counts(reset=True, real=case == "real")
        row = {"kind": kind, "case": case, "block": block, "concurrency": concurrency,
               "status": "running", "latencies_s": [], "output_counts": [], "errors": 0}
        self.report["samples"].append(row)
        self.publish()
        try:
            with self.server(kind, directory) as (origin, child, state, startup):
                variant = case if case in ("parse", "pre_health", "no_publisher") else "full"
                config = self.configure(origin, original, variant=variant)
                row["startup_s"] = startup
                warm = case in ("warm", "pressure", "no_publisher")
                if warm:
                    row["warmup_output_counts"] = []
                    for _ in range(2):
                        warm_raw = self.download(origin, config["name"])
                        warm_output = self.check_output(warm_raw, compare=False)
                        row["warmup_output_counts"].append(len(warm_output))
                    warm_raw_hash = digest(warm_raw)
                    row["warmup_counts"] = self.counts()
                    if case != "no_publisher":
                        # Warm-up may populate initially absent source/cache entries over both requests.
                        self.audit(state, warm_output, None, self.counts(), origin, cross_compare=False)
                    self.counts(reset=True)
                last = None
                profile_offset = (directory / "server.log").stat().st_size
                mismatch = None
                if case == "pressure":
                    # A separate runner has no earlier cold/warm cell to seed this oracle.
                    self.expected_outputs.setdefault("full", warm_output)
                    self.expected_hashes.setdefault("full", nodes_hash(warm_output))
                if case == "pressure" and nodes_hash(warm_output) != self.expected_hashes["full"]:
                    mismatch = warm_output

                def once():
                    nonlocal mismatch
                    began = time.monotonic()
                    raw = self.download(origin, config["name"])
                    elapsed = time.monotonic() - began
                    fingerprint = digest(raw)
                    if case == "pressure":
                        # Warm-cache output is byte-stable; avoid a Python JSON/GIL load-generator bottleneck.
                        require(fingerprint == warm_raw_hash, "output_mismatch")
                        output = warm_output
                    else:
                        output = self.check_output(raw, compare=False, key=case)
                    row["output_counts"].append(len(output))
                    if case not in ("real", "pressure"):
                        key = case if case in ("parse", "pre_health") else "full"
                        expected = self.expected_outputs.setdefault(key, output)
                        self.expected_hashes.setdefault(key, nodes_hash(expected, hopping=case == "parse"))
                        if nodes_hash(output, hopping=case == "parse") != self.expected_hashes[key] and mismatch is None:
                            mismatch = output
                    return elapsed, output, fingerprint

                with Meter(child, self.fixture, self.meta if case == "real" else None) as meter:
                    if case == "pressure":
                        until = time.monotonic() + 20
                        def worker():
                            observations = []
                            while time.monotonic() < until:
                                observations.append(once())
                            return observations
                        with ThreadPoolExecutor(max_workers=concurrency) as pool:
                            for observations in pool.map(lambda _: worker(), range(concurrency)):
                                for elapsed, output, raw_hash in observations:
                                    row["latencies_s"].append(elapsed)
                                    last = output
                    else:
                        repeats = repeats or (20 if case in ("warm", "parse", "pre_health", "no_publisher") else 1)
                        for _ in range(repeats):
                            elapsed, last, raw_hash = once()
                            row["latencies_s"].append(elapsed)
                row.update(meter.result(len(row["latencies_s"])))
                row["output_count"] = len(last)
                row["output_sha256"] = nodes_hash(last, hopping=case == "parse")
                row["raw_output_sha256"] = raw_hash
                row["counts"] = self.counts()
                if getattr(self.args, "profile_pipeline", False):
                    row["profiler_enabled"] = kind == "rust_repeat"
                    profiles = pipeline_profiles(directory / "server.log", profile_offset)
                    require(len(profiles) == (len(row["latencies_s"]) if row["profiler_enabled"] else 0), "resource")
                    row["pipeline_profiles"] = profiles
                    for index, profile in enumerate(profiles):
                        print(f"Pipeline profile {kind}/{case} sample={index}: " + json.dumps(profile, sort_keys=True), flush=True)
                if getattr(self.args, "diagnose_health", False) and not getattr(self.args, "profile_pipeline", False) and case != "pre_health":
                    root = json.loads((state / "root.json").read_bytes())
                    row["health_profile"] = root["benchmark-health-profile"]
                    print(f"Health profile {kind}/{case}: " + json.dumps(row["health_profile"], sort_keys=True), flush=True)
                if case not in ("parse", "pre_health", "no_publisher"):
                    row["work"] = self.audit(state, last, not warm, row["counts"], origin, case == "real", cross_compare=False)
                if case == "warm" and block == 0:
                    clash = self.download(origin, config["name"], target="ClashMeta")
                    # Parse via the original's existing locked YAML utility, outside timing.
                    js = "const fs=require('fs');const yaml=require('yaml');process.stdout.write(JSON.stringify(yaml.parse(fs.readFileSync(0,'utf8'))))"
                    parsed = json.loads(command([self.node, "-e", js], cwd=self.args.original / "backend", stdin=clash))
                    proxies = parsed["proxies"] if isinstance(parsed, dict) else parsed
                    row["clash_count"] = len(proxies)
                    row["clash_sha256"] = digest(proxies)
                    require(len(proxies) == len(last), "clash")
                    if self.clash_hash is None:
                        self.clash_hash = digest(proxies)
                    require(self.clash_hash == digest(proxies), "clash")
                if mismatch is not None:
                    key = case if case in ("parse", "pre_health") else "full"
                    compare_nodes(self.expected_outputs[key], mismatch, hopping=case == "parse")
                if case != "real" and "work" in row:
                    require(row["work"]["snapshot_sha256"] == self.expected_snapshot_hash, "output_mismatch")
                row["status"] = "passed"
        except Exception as error:
            row.update(status="failed", failure=error.code if isinstance(error, Failure) else "unexpected", errors=1)
            if hasattr(error, "observations"):
                row["failure_observations"] = error.observations
            row["counts"] = self.counts()
            log = (directory / "server.log").read_text(encoding="utf-8", errors="replace")
            matches = re.findall(r"input=(\d+) supported=(\d+) identities=(\d+)\s+probe_coalesced=(\d+) cache=(?:hit|empty)\s+probed=(\d+)/(\d+) ok=(\d+) failed=(\d+)\s+guarded=(\d+) budget_skipped=(\d+)\s+output=(\d+)", log)
            row["health_counts"] = [list(map(int, match)) for match in matches]
            root = json.loads((directory / "state/root.json").read_bytes())
            cache = json.loads(root.get("sub-store-cached-script-resource", "{}"))
            health = [v["data"] for k, v in cache.items() if k.startswith("http-meta:health-inc:v1:")]
            row["health_records"] = sum(len(v["records"]) for v in health)
            row["health_verified"] = sum(bool(r.get("lastOk")) for v in health for r in v["records"].values())
            # Product incompatibility remains a failed sample; collect independent cells too.
            if not isinstance(error, Failure) or error.code not in {"output_mismatch", "snapshot", "work_incomplete", "warm_miss", "clash", "empty_output", "http", "timeout"}:
                raise
        finally:
            self.publish()
            print(f"Cell {case}/{kind} block={block + 1} clients={concurrency}: {row['status']}; requests={row.get('completed', 0)}, output={row.get('output_count', 0)}, elapsed_s={row.get('elapsed_s', 0):.2f}, failure={row.get('failure', 'none')}.", flush=True)
            if row["latencies_s"]:
                print(f"Latency {case}/{kind}: " + json.dumps(distribution(row["latencies_s"]), sort_keys=True), flush=True)

    def publish(self):
        public(self.report)
        self.args.publish.mkdir(exist_ok=True)
        rows = self.report["samples"]
        lines = ["# Supplied subscription: original / Rust", "", f"Status: **{self.report['status']}**. Linux x64 GitHub-hosted runner only.", "",
                 "Original: `" + ORIGINAL + "`; Rust: `" + RUST + "`.", "",
                 "Original frontend file picker imports the unchanged secret file; 22 captured sources are replayed identically.",
                 "The shared five-site script variant removes maintenance caps. Filters/dedup, probe retries/timeouts and snapshot publication remain active.", "",
                 "Observed timings include failed correctness cells. Different output/work counts invalidate speedup comparisons; only verified equivalent pairs enter estimates.", "",
                 "| Case | Clients | Backend | Passed/measured blocks | Requests | Median ms | P95 ms | RPS | CPU s/request | Peak RSS MiB | OS lifetime peak MiB |", "|---|---:|---|---:|---:|---:|---:|---:|---:|---:|---:|"]
        for case in CASES:
            for concurrency in (1, 2, 4, 8):
                for kind in COMPARISON_KINDS:
                    group = [r for r in rows if r["case"] == case and r["kind"] == kind and r["concurrency"] == concurrency and "elapsed_s" in r]
                    if not group:
                        continue
                    timings = [v for row in group for v in row["latencies_s"]]
                    dist = distribution(timings)
                    elapsed = sum(r["elapsed_s"] for r in group)
                    cpu_s = sum(r["cpu_s"] for r in group)
                    p95 = f"{dist['p95'] * 1000:.2f}" if "p95" in dist else "insufficient n"
                    lines.append(f"| {case} | {concurrency} | {kind} | {sum(r['status']=='passed' for r in group)}/{len(group)} | {len(timings)} | {dist['median']*1000:.2f} | {p95} | {len(timings)/elapsed:.2f} | {cpu_s/len(timings):.5f} | {max(r['rss_peak_bytes'] for r in group)/2**20:.1f} | {max(r['os_lifetime_peak_bytes'] or 0 for r in group)/2**20:.1f} |")
        lines += ["", "File import (five fresh states per backend; browser driver wall time and actual HAR API time are separate):"]
        for kind in KINDS:
            imports = [r for r in self.report["imports"] if r["kind"] == kind]
            if imports:
                lines.append(f"- {kind}: imports={len(imports)}, startup median={statistics.median(r['startup_s'] for r in imports)*1000:.2f}ms, API import median={statistics.median(r['api_import_s'] for r in imports)*1000:.2f}ms, UI wall median={statistics.median(r['ui_import_wall_s'] for r in imports)*1000:.2f}ms, restart passes={sum(r['restart'] for r in imports)}.")
        lines += ["", "Paired block estimates (Node / Rust; >1 means Rust has lower latency. Pressure uses Node RPS / Rust RPS, where <1 means Rust has higher throughput):"]
        estimates = []
        for case in ("cold", "warm", "pressure", "parse", "pre_health", "no_publisher"):
            for concurrency in (1, 2, 4, 8):
                pairs = []
                for block in range(10):
                    pair = [next((r for r in rows if r["case"] == case and r["kind"] == kind and r["block"] == block and r["concurrency"] == concurrency and r["status"] == "passed"), None) for kind in KINDS]
                    if all(pair) and pair[0]["output_sha256"] == pair[1]["output_sha256"]:
                        pairs.append(tuple((r["rps"] if case == "pressure" else statistics.median(r["latencies_s"])) for r in pair))
                if pairs:
                    interval = paired_interval(pairs)
                    estimates.append({"case": case, "concurrency": concurrency, **interval})
                    lines.append(f"- {case}, clients={concurrency}: {interval['node_over_rust']:.3f}, paired bootstrap interval [{interval['bootstrap_low']:.3f}, {interval['bootstrap_high']:.3f}], blocks={len(pairs)}.")
        self.report["estimates"] = estimates
        capture = self.report.get("capture", {})
        if capture:
            lines += ["", f"Capture: {capture['nonempty']}/22 nonempty source bodies; {capture['empty_or_failed']} empty/failed. Capture wall time {capture['elapsed_s']:.2f}s. Source text is captured before subscription preprocessing; HTTP/TLS envelopes are excluded."]
        for row in self.report.get("replay", []):
            lines.append(f"- Raw replay {row['kind']}: equal={row['equal']}, matching sources={row['matching_sources']}/{row['source_count']}.")
        if self.report.get("diagnostic"):
            lines += ["", "Short parse diagnostic only; the complete stress matrix was not run.", "",
                      "| Source | Backend | Input bytes | Nodes | Wall s | CPU s | Status |",
                      "|---:|---|---:|---:|---:|---:|---|"]
            for row in self.report.get("direct_sources", []):
                lines.append(f"| {row['source_index']} | {row['kind']} | {row['input_bytes']} | {row['output_count']} | {row['elapsed_s']:.3f} | {row['cpu_s']:.3f} | {row['status']} |")
        for row in rows:
            if row["status"] == "failed":
                lines.append(f"- FAILED {row['case']} {row['kind']} block={row['block']} clients={row['concurrency']}: {row['failure']}, output count={row.get('output_count', 'unavailable')}. Field-path/type/hash diagnostics are in metrics.json.")
        for row in rows:
            if row["case"] in ("cold", "real") and "work" in row:
                lines.append(f"- {row['case']} {row['kind']} block={row['block']}: selected={row['counts']['selected']}, identities={row['work']['identities']}, verified={row['work']['verified']}, requests/unique={row['counts']['probes']}/{row['counts']['unique_probes']}, output/snapshot={row['output_count']}/{row['work']['published']}.")
        lines += ["", "Attribution and measurement headroom:"]
        for kind in KINDS:
            group = [r for r in rows if r["kind"] == kind and "elapsed_s" in r]
            for case in ("pressure", "real"):
                matches = [r for r in group if r["case"] == case]
                if matches:
                    elapsed = sum(r["elapsed_s"] for r in matches)
                    lines.append(f"- {kind} {case}: backend CPU={sum(r['cpu_s'] for r in matches)/elapsed:.2f} cores, driver CPU={sum(r['driver_cpu_s'] for r in matches)/elapsed:.2f} cores, fixture CPU={sum(r['fixture_cpu_s'] for r in matches)/elapsed:.2f} cores, sampler CPU={sum(r['sampler_cpu_s'] for r in matches)/elapsed:.3f} cores, detector CPU={sum(r['detector_cpu_s'] for r in matches):.2f}s, detector peak RSS={max(r['detector_rss_peak_bytes'] for r in matches)/2**20:.1f}MiB.")
            attribution = []
            for case in ("parse", "pre_health", "no_publisher", "warm"):
                timings = [v for r in group if r["case"] == case for v in r["latencies_s"]]
                if timings:
                    attribution.append(f"{case}={statistics.median(timings)*1000:.2f}ms")
            if attribution:
                lines.append(f"- {kind} diagnostics: " + ", ".join(attribution) + ". Differences are diagnostic observations; these cases change the work and run in different blocks.")
        machine = self.report.get("provenance", {}).get("runner")
        if machine:
            lines += ["", f"Runner: {machine['cpu_model']}, logical/affinity CPUs={machine['logical_cpus']}/{machine['affinity_cpus']}, RAM={machine['memory_bytes']/2**30:.2f}GiB, cgroup memory limit={machine['memory_limit_bytes']}, CPU quota/period={machine['cpu_quota_us']}/{machine['cpu_period_us']}us."]
        if self.report.get("workers"):
            lines += ["", f"Parallel workers: {len(self.report['workers'])}. Each Node/Rust pair ran on the same worker; runner labels do not imply identical CPUs or contention. Paired estimates retain those worker pairs; pooled latency distributions span the recorded machines."]
        failures = [r for r in rows if r["status"] == "failed"]
        if failures or self.report.get("failure"):
            lines += ["", "Failures: " + ", ".join([r.get("failure", "unexpected") for r in failures] + ([self.report["failure"]] if self.report.get("failure") else [])) + ". Failed/incomplete cases are never speedups."]
        lines += pipeline_summary(rows)
        lines += ["", "Limits: fixed source-text replay excludes provider transport/decryption; the pinned input has no age secret URL arguments. Exact ordered replay equality on both backends is required before measurement. Measured requests still execute normal source preprocessing, parsing, conversion, scripts, cache behavior and state publication, with output semantics checked. Controlled detector is a fixture; real HTTP-META/Mihomo is reported separately, with changing reachability. Top-level `_latency` is excluded from node fingerprints; only parse-only exports also canonicalize a valid random port within the unchanged declared hopping ranges. Raw output hashes remain recorded. Snapshot timestamp is observed separately. Two warm-up requests precede warm measurements; their node/work counts remain recorded, and measured warm probes must be zero. Warm requests still publish snapshots. RSS sampling is 50ms with actual intervals recorded; VmHWM is process-lifetime peak. CPU counters have OS clock granularity. Low block counts and hosted-runner contention limit confidence; no P99 or cross-platform ranking is claimed.",
                  "", "Raw input, proxy data, state, browser HAR and child logs remain private in ephemeral runner scratch. The only published data are allowlisted metrics, counts and hashes."]
        public(self.report)
        write_json(self.args.publish / "metrics.json", self.report)
        text = "\n".join(lines) + "\n"
        (self.args.publish / "summary.md").write_text(text, encoding="utf-8")
        if os.environ.get("GITHUB_STEP_SUMMARY"):
            Path(os.environ["GITHUB_STEP_SUMMARY"]).write_text(text, encoding="utf-8")


def get_asset(url, expected, path):
    for attempt in range(3):
        try:
            with urlopen(url, timeout=90) as response:
                body = response.read()
            require(digest(body) == expected, "asset")
            path.write_bytes(body)
            return body
        except OSError:
            if attempt == 2:
                raise Failure("asset") from None
            time.sleep(1)


def get_script():
    with urlopen("https://api.github.com/repos/DSLZL/scripts/git/blobs/" + SCRIPT_BLOB, timeout=60) as response:
        body = base64.b64decode(json.load(response)["content"])
    require(digest(body) == SCRIPT_HASH, "asset")
    return body.decode()


def cgroup_limit(name):
    path = Path("/sys/fs/cgroup") / name
    if not path.exists():
        return None
    text = path.read_text().strip()
    return None if text == "max" else int(text)


def cpu_quota(path=Path("/sys/fs/cgroup/cpu.max")):
    if not path.exists():
        return {"cpu_quota_us": None, "cpu_period_us": None}
    quota, period = path.read_text().split()
    return {"cpu_quota_us": None if quota == "max" else int(quota), "cpu_period_us": int(period)}


def provenance(args):
    sources = [(args.original, ORIGINAL), (args.rust, RUST), (args.frontend, FRONTEND)]
    if args.rust_baseline:
        sources.append((args.rust_baseline, RUST_BASELINE))
    for path, revision in sources:
        require(command(["git", "rev-parse", "HEAD"], cwd=path).decode().strip() == revision, "source")
        command(["git", "diff", "--exit-code"], cwd=path)
    rust_binary = digest((args.rust / "rust/target/release/sub-store-rust").read_bytes())
    baseline_binary = digest((args.rust_baseline / "rust/target/release/sub-store-rust").read_bytes()) if args.rust_baseline else None
    if args.rust_baseline:
        require(RUST != RUST_BASELINE and rust_binary != baseline_binary, "source")
    require(command(["node", "--version"]).decode().strip() == "v24.16.0", "source")
    require(psutil.__version__ == "7.2.2", "source")
    cpu_model = next(line.split(":", 1)[1].strip() for line in Path("/proc/cpuinfo").read_text().splitlines() if line.startswith("model name"))
    PUBLIC_STRINGS.add(cpu_model)
    return {"original": ORIGINAL, "rust": RUST, "frontend": FRONTEND,
            "rust_baseline": RUST_BASELINE if args.rust_baseline else None,
            "rust_baseline_binary_sha256": baseline_binary,
            "original_lock_sha256": digest((args.original / "backend/pnpm-lock.yaml").read_bytes()),
            "rust_lock_sha256": digest((args.rust / "rust/Cargo.lock").read_bytes()),
            "node_bundle_sha256": digest((args.original / "backend/dist/sub-store.bundle.js").read_bytes()),
            "rust_binary_sha256": rust_binary,
            "frontend_assets_sha256": digest([digest(p.read_bytes()) for p in sorted((args.frontend / "dist").rglob("*")) if p.is_file()]),
            "node_version": "24.16.0", "rust_toolchain": "1.98.0", "python_version": sys.version.split()[0],
            "browser_driver_version": "0.27.3", "psutil_version": psutil.__version__,
            "runner": {"target": "linux_x64", "cpu_model": cpu_model,
                       "logical_cpus": psutil.cpu_count(), "affinity_cpus": len(psutil.Process().cpu_affinity()),
                       **cpu_quota(),
                       "memory_bytes": psutil.virtual_memory().total, "memory_limit_bytes": cgroup_limit("memory.max"),
                       "load_before": list(os.getloadavg())}}


def frozen_cipher(operation, input_file, payload):
    result = subprocess.run([shutil.which("node"), str(Path(__file__).with_name("frozen-inputs.cjs")), operation, str(input_file)],
                            input=payload, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    require(result.returncode == 0, "capture")
    return result.stdout


def seal_inputs(suite, input_file, destination):
    stream = io.BytesIO()
    with tarfile.open(fileobj=stream, mode="w:gz") as archive:
        for kind, directory in suite.pristine.items():
            require(not any(p.is_symlink() for p in directory.rglob("*")), "capture")
            archive.add(directory, arcname="pristine-" + kind)
        archive.add(suite.args.scratch / "mirrors", arcname="mirrors")
        metadata = encoded(suite.report)
        entry = tarfile.TarInfo("prepared.json")
        entry.size = len(metadata)
        archive.addfile(entry, io.BytesIO(metadata))
    destination.write_bytes(frozen_cipher("seal", input_file, stream.getvalue()))


def load_inputs(suite, input_file, source):
    payload = frozen_cipher("open", input_file, source.read_bytes())
    with tarfile.open(fileobj=io.BytesIO(payload), mode="r:gz") as archive:
        archive.extractall(suite.args.scratch, filter="data")
    prepared = json.loads((suite.args.scratch / "prepared.json").read_bytes())
    require(prepared["preparation_complete"] and prepared["input_sha256"] == INPUT_HASH, "capture")
    require(prepared.get("native_pipeline_profile", False) == getattr(suite.args, "profile_pipeline", False), "capture")
    require(all(prepared["provenance"].get(k) == v for k, v in suite.report["provenance"].items() if k != "runner"), "source")
    require(len(prepared["imports"]) == 10 and all(row["restart"] for row in prepared["imports"])
            and all(sum(row["kind"] == kind for row in prepared["imports"]) == 5 for kind in KINDS), "persistence")
    require(prepared["capture"]["before_preprocessing"] and len(prepared["capture"]["sources"]) == 22
            and {row["index"] for row in prepared["capture"]["sources"]} == set(range(22)), "capture")
    for row in prepared["capture"]["sources"]:
        require(digest((suite.args.scratch / "mirrors" / f"{row['index']}.txt").read_bytes()) == row["sha256"], "capture")
    suite.pristine = {kind: suite.args.scratch / ("pristine-" + kind) for kind in KINDS}
    for key in ("capture", "imports", "script", "chromium"):
        suite.report[key] = prepared[key]


def main():
    require(os.environ.get("GITHUB_ACTIONS") == "true", "source")
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--original", type=Path)
    parser.add_argument("--rust", type=Path)
    parser.add_argument("--rust-baseline", type=Path)
    parser.add_argument("--frontend", type=Path)
    parser.add_argument("--scratch", type=Path, required=True)
    parser.add_argument("--publish", type=Path)
    parser.add_argument("--fixture-port", type=int)
    parser.add_argument("--meta-port", type=int)
    parser.add_argument("--diagnose-parse", action="store_true")
    parser.add_argument("--diagnose-health", action="store_true")
    parser.add_argument("--profile-pipeline", action="store_true")
    parser.add_argument("--prepare-only", action="store_true")
    parser.add_argument("--frozen", type=Path)
    parser.add_argument("--shard", choices=tuple(SHARDS))
    args = parser.parse_args()
    require(not args.prepare_only or (args.frozen and not args.shard), "input")
    require(not args.shard or (args.frozen and not args.diagnose_parse and not args.diagnose_health), "input")
    require(not (args.diagnose_parse and args.diagnose_health), "input")
    require(not args.rust_baseline or args.diagnose_health, "input")
    require(not args.profile_pipeline or (args.diagnose_health and args.rust_baseline), "input")
    soft, hard = resource.getrlimit(resource.RLIMIT_NOFILE)
    resource.setrlimit(resource.RLIMIT_NOFILE, (hard, hard))
    if args.fixture_port:
        asyncio.run(Fixture(args.scratch, args.fixture_port, args.meta_port).serve())
        return
    require(all((args.original, args.rust, args.frontend, args.publish)), "source")
    args.scratch.mkdir(mode=0o700)
    args.publish.mkdir(mode=0o700)
    report = {"status": "running", "input_sha256": INPUT_HASH, "imports": [], "samples": []}
    if args.profile_pipeline:
        report["native_pipeline_profile"] = True
    if args.shard:
        report["shard"] = args.shard
    if args.diagnose_parse:
        report["diagnostic"] = "parse_diagnostic"
    elif args.diagnose_health:
        report["diagnostic"] = "health_diagnostic"
    suite = Suite(args, report)
    try:
        report["provenance"] = provenance(args)
        raw = base64.b64decode(os.environ.pop("BENCHMARK_SUBSCRIPTION_B64", ""), validate=True)
        require(digest(raw) == INPUT_HASH, "input")
        input_file = args.scratch / INPUT_NAME
        input_file.write_bytes(raw)
        original = json.loads(raw)
        require(len(original["process"]) == 7 and len(original["url"].strip().splitlines()) == 22, "input")
        mirrors = args.scratch / "mirrors"
        if args.frozen and not args.prepare_only:
            load_inputs(suite, input_file, args.frozen)
        else:
            mirrors.mkdir()
            source = get_script()
            variant = stress_script(source)
            (args.publish / "stress.patch").write_text("".join(difflib.unified_diff(source.splitlines(True), variant.splitlines(True), "pinned.js", "stress.js")), encoding="utf-8")
            report["script"] = {"blob": SCRIPT_BLOB, "original_sha256": SCRIPT_HASH,
                                "stress_sha256": digest(variant.encode()), "changed_sites": 5}
            if args.diagnose_health:
                instrumented = profile_health_script(variant, enabled=not args.profile_pipeline)
                if instrumented != variant:
                    (args.publish / "health-profile.patch").write_text("".join(difflib.unified_diff(variant.splitlines(True), instrumented.splitlines(True), "stress.js", "profile.js")), encoding="utf-8")
                    variant = instrumented
                    report["script"]["profile_sha256"] = digest(variant.encode())
            (mirrors / "stress.js").write_text(variant, encoding="utf-8")
            suite.publish()
            print("Importing the unchanged file on both backends (five paired observations).", flush=True)
            suite.imports(original, input_file)
            print("Capturing source text before preprocessing through the original backend.", flush=True)
            suite.capture(original)
        fixture_port, meta_port = free_port(), free_port()
        fixture_args = [sys.executable, Path(__file__).resolve(), "--scratch", mirrors,
                        "--fixture-port", fixture_port, "--meta-port", meta_port]
        with process(fixture_args, args.scratch, suite.env, args.scratch / "fixture.log") as fixture:
            suite.fixture = fixture
            suite.fixture_origin = f"http://127.0.0.1:{fixture_port}"
            ready(fixture, suite.fixture_origin + "/counts")
            began, driver_before, fixture_before = time.monotonic(), cpu(psutil.Process()), cpu(psutil.Process(fixture.pid))
            with ThreadPoolExecutor(max_workers=8) as pool:
                list(pool.map(lambda _: http(suite.fixture_origin + "/health.js"), range(160)))
            report["fixture_calibration"] = {"requests": 160, "elapsed_s": time.monotonic() - began,
                                             "driver_cpu_s": cpu(psutil.Process()) - driver_before,
                                             "fixture_cpu_s": cpu(psutil.Process(fixture.pid)) - fixture_before}
            suite.replay(original)
            if args.prepare_only:
                report.update(status="passed", preparation_complete=True)
                suite.publish()
                seal_inputs(suite, input_file, args.frozen)
                raise SystemExit(0)
            if args.diagnose_parse:
                suite.diagnose_sources()
            matrix = (("pre_health", 1), ("cold", 1), ("warm", 1), ("no_publisher", 1)) if args.diagnose_health else (("parse", 1), ("pre_health", 1)) if args.diagnose_parse else MATRIX
            for case, blocks in matrix:
                for concurrency in ((1, 2, 4, 8) if case == "pressure" else (1,)):
                    for block in range(blocks):
                        if args.shard and SHARDS[args.shard] != (case, block, concurrency):
                            continue
                        order = COMPARISON_KINDS if args.rust_baseline else KINDS
                        for kind in (order if block % 2 == 0 else order[::-1]):
                            print(f"Measuring {case} / {kind}, block {block + 1}, clients {concurrency}.", flush=True)
                            suite.run_case(kind, case, block, original, concurrency,
                                           repeats=3 if (args.diagnose_parse or args.diagnose_health) and case != "cold" else None)
            if args.diagnose_parse or args.diagnose_health:
                report["status"] = "passed" if all(r["status"] == "passed" for r in report["samples"] + report.get("direct_sources", [])) else "failed"
                print("Short diagnostic status: " + report["status"], flush=True)
                raise SystemExit(0 if report["status"] == "passed" else 1)
            if args.shard and SHARDS[args.shard][0] != "real":
                report.update(status="passed" if all(row["status"] == "passed" for row in report["samples"]) else "failed", shard=args.shard)
                raise SystemExit(0 if report["status"] == "passed" else 1)
            print("Running the separately reported real HTTP-META/Mihomo pair.", flush=True)
            meta_dir = args.scratch / "http-meta"
            (meta_dir / "meta").mkdir(parents=True)
            assets = (
                ("https://github.com/xream/http-meta/releases/download/1.3.0/http-meta.bundle.js", "912d378dc36ca1939cda079c7b2db4b85ce7445ce4e67119592c0d98192ed03b", meta_dir / "http-meta.bundle.js"),
                ("https://github.com/xream/http-meta/releases/download/1.3.0/tpl.yaml", "bcf79d9de3c72526435e8dc5f2cf2b641ac79c8cb8e593aecf34da2fba744ec4", meta_dir / "meta/tpl.yaml"),
                ("https://github.com/MetaCubeX/mihomo/releases/download/v1.19.32/mihomo-linux-amd64-compatible-v1.19.32.gz", "ba3ce607747a07f948fc35780e108a4a7c7f552a38b9bd4d115f313ebcb89c20", meta_dir / "mihomo.gz"),
            )
            for url, fingerprint, path in assets:
                get_asset(url, fingerprint, path)
            binary = meta_dir / "meta/http-meta"
            binary.write_bytes(gzip.decompress((meta_dir / "mihomo.gz").read_bytes()))
            binary.chmod(0o700)
            report["detector"] = {"version": "1.3.0", "mihomo_version": "1.19.32",
                                  "assets_sha256": [item[1] for item in assets]}
            meta_env = {**suite.env, "HOST": "127.0.0.1", "PORT": str(meta_port),
                        "META_FOLDER": str(meta_dir / "meta"), "META_TEMP_FOLDER": str(meta_dir),
                        "META_BINARY_PATH": str(binary), "BODY_JSON_LIMIT": "100mb"}
            with process([suite.node, meta_dir / "http-meta.bundle.js"], meta_dir, meta_env, meta_dir / "service.log") as meta:
                suite.meta = meta
                until = time.monotonic() + 30
                while time.monotonic() < until and meta.poll() is None:
                    try:
                        http(f"http://127.0.0.1:{meta_port}/stats", "POST", {})
                        break
                    except (OSError, Failure):
                        time.sleep(.1)
                require(meta.poll() is None, "startup")
                for kind in KINDS:
                    suite.run_case(kind, "real", 0, original)
        require(len(report["imports"]) == 10 and all(r["restart"] for r in report["imports"]), "persistence")
        report["matrix_complete"] = not args.shard
        if args.shard:
            report["shard"] = args.shard
        report["status"] = "passed" if all(r["status"] == "passed" for r in report["samples"]) else "failed"
    except Exception as error:
        report.update(status="failed", failure=error.code if isinstance(error, Failure) else "unexpected",
                      failure_stack=[frame.lineno for frame in traceback.extract_tb(error.__traceback__) if frame.filename == __file__])
        (args.scratch / "failure.txt").write_text(traceback.format_exc(), encoding="utf-8")
    finally:
        suite.publish()
    print("Sanitized report status: " + report["status"], flush=True)
    raise SystemExit(0 if report["status"] == "passed" else 1)


if __name__ == "__main__":
    main()
