"""Capture native rquickjs and shared Node/Rust host-contract evidence."""
import argparse
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import platform

from reference import execute, HERE, ROOT, source_provenance
from runner import check_json
from upstream import ControlledUpstream
from dns_fixtures import ControlledDns, query_path, query_bytes, response_bytes
from mmdb_fixtures import FIXTURES as MMDB_FIXTURES, prepare as prepare_mmdb, verify_fixtures as verify_mmdb_fixtures, check_state as check_mmdb_state

HOST_CASES = [
    "SH01-storage-values-and-aliases", "SH02-root-key-routing", "SH03-storage-undefined-delete-and-errors",
    "SH04-all-http-methods", "SH05-binary-text-headers-and-string-url", "SH06-binary-request-accept-and-auth",
    "SH07-request-and-async-response-events", "SH08-timeout-hooks-and-late-response",
    "SH09-setup-errors-and-non-success-status", "SH10-redirect-limit", "SH11-timer-interval-arguments-and-cancellation",
]
PROCESSOR_CASES = [
    "PC01-full-operator-injection-and-cloning", "PC02-shortcut-operator-pending-http",
    "PC03-full-and-shortcut-filter-truthiness", "PC04-operator-fallback-retains-mutation",
    "PC05-operator-return-truthiness", "PC06-user-errors-and-recovery",
    "PC07-full-response-transform-and-aliases", "PC08-shortcut-response-pending-http",
    "PC09-file-shortcut-and-local-options", "PC10-mihomo-yaml-patch-branches",
    "PC11-mihomo-async-main-and-legacy-type", "PC12-nested-rust-reentry-pending-processor",
    "PC13-script-cache-defaults-and-ttl-coercion", "PC14-script-cache-expiry-removal-and-prefix",
    "PC15-script-cache-root-persistence-and-aliases", "PC16-project-yaml-parse-merge-tags-and-errors",
    "PC17-project-yaml-dump-options-and-json-boundary",
    "PC18-ip-validation-and-coercion", "PC19-address4-construction-subnets-and-bigint",
    "PC20-address6-construction-mapping-and-url", "PC21-uuid-shape-and-types",
    "PC22-port-selection-boundaries-and-legacy-inputs", "PC23-country-flags-iso-and-unicode-removal",
    "PC24-json5-parsing-reviver-and-own-properties", "PC25-json5-stringify-options-and-errors",
    "PC26-md5-fixed-ascii-and-unicode-vectors", "PC27-flow-header-casing-encoding-and-numbers",
    "PC28-flow-parser-malformed-and-falsy-inputs", "PC29-flow-units-quota-and-expiry-boundary",
    "PC30-flow-reset-calendar-and-invalid-options", "PC31-flow-normalization-duplicates-and-fallback",
    "PC32-public-helpers-in-actual-script-operator",
    "PC33-flow-download-short-circuits", "PC34-flow-head-cache-ttl-and-no-cache",
    "PC35-flow-head-to-get-fallback", "PC36-flow-url-body-and-encoded-headers",
    "PC37-flow-body-fallback-and-status-rejection", "PC38-flow-rejection-empty-and-timeout",
    "PC39-flow-request-queue-and-wait-time", "PC40-flow-http-injected-into-pending-script",
    "PC41-doh-a-packet-edns-and-cname", "PC42-doh-aaaa-and-ipv6-edns",
    "PC43-doh-defaults-status-and-rcode", "PC44-doh-malformed-and-custom-timeout",
    "PC45-custom-dns-cache-alias-ttl-and-key", "PC46-custom-dns-aaaa-filter",
    "PC47-custom-dns-empty-wrong-and-nxdomain", "PC48-custom-dns-sequential-fallback",
    "PC49-custom-dns-bounded-race-and-winner-cache", "PC50-custom-dns-exhaustion-retains-errors",
    "PC51-custom-dns-invalid-inputs-before-network", "PC52-doh-and-resolver-in-pending-script",
    "PC53-native-dns-address-validation", "PC54-udp-ipv4-ipv6-and-edns",
    "PC55-tcp-fragments-inactivity-and-first-frame", "PC56-tcp-premature-header-and-body-eof",
    "PC57-udp-and-tcp-malformed-packet", "PC58-udp-and-tcp-timeout-no-late-cache",
    "PC59-dot-default-verification-and-explicit-boolean", "PC60-dot-ip-without-sni",
    "PC61-dot-fragments-eof-and-timeout", "PC62-custom-mixed-native-fallback-and-cache",
    "PC63-custom-dot-certificate-cache-partition", "PC64-native-dns-in-async-script",
    "PC65-mmdb-process-defaults-and-sync-results", "PC66-mmdb-optional-readers-and-falsy-paths",
    "PC67-mmdb-explicit-and-falsy-default-precedence", "PC68-mmdb-country-v4-v6-and-absent-field",
    "PC69-mmdb-asn-v4-v6-number-and-absent-organization", "PC70-mmdb-invalid-ip-errors-and-recovery",
    "PC71-mmdb-unmapped-address-errors", "PC72-mmdb-database-kind-and-validation-order",
    "PC73-mmdb-missing-files-and-constructor-order", "PC74-mmdb-corrupt-buffer-errors",
    "PC75-mmdb-eager-read-instance-isolation-and-source-methods", "PC76-mmdb-in-async-script-and-persistence",
]


def check_host(actual, expected):
    check_json(actual["passed"], HOST_CASES)
    check_json(actual, expected)


def check_processors(actual, expected):
    check_json(actual["passed"], PROCESSOR_CASES)
    check_json(actual, expected)


def check_processor_progress(events, resume=False):
    check_json(events, [] if resume else [{"id": case, "event": event}
               for case in PROCESSOR_CASES for event in ("start", "end")])


def check_processor_requests(requests):
    expected = [
        {"method": "GET", "path": "/host/echo?name=zero", "body_bytes": [], "probe": "processor-shortcut", "accept": "*/*"},
        {"method": "GET", "path": "/host/echo?name=false", "body_bytes": [], "probe": "processor-shortcut", "accept": "*/*"},
        {"method": "POST", "path": "/host/echo", "body_bytes": list("订阅🙂\r\n".encode()), "probe": "processor-response", "accept": "*/*"},
        {"method": "POST", "path": "/host/echo", "body_bytes": list("嵌套🙂\r\n".encode()), "probe": "processor-nested", "accept": "*/*"},
    ]
    actual = [{"method": item["method"], "path": item["path"], "body_bytes": item["body_bytes"],
               "probe": item["headers"].get("x-probe"), "accept": item["headers"].get("accept")}
              for item in requests[:4]]
    check_json(actual, expected)
    expected_flow = [("HEAD", "head", "Flow Helper/1", "direct")] * 3 + [
        ("HEAD", "get", "Flow Default/1", None), ("GET", "get", "Flow Default/1", None),
        ("GET", "body", "Flow Fragment/1", "fragment"),
        ("GET", "fallback", "Flow Explicit/1", "argument"), ("GET", "badstatus", "Flow Explicit/1", "argument"),
        ("GET", "rejected", "Flow Default/1", None), ("HEAD", "empty", "Flow Default/1", None),
        ("GET", "empty", "Flow Default/1", None), ("GET", "delay", "Flow Default/1", None),
        ("HEAD", "queued/first", "Flow Default/1", None), ("HEAD", "queued/second", "Flow Default/1", None),
        ("HEAD", "head", "Flow Default/1", None),
    ]
    actual_flow = [{"method": item["method"], "path": item["path"], "body_bytes": item["body_bytes"],
                    "user_agent": item["headers"].get("user-agent"), "token": item["headers"].get("x-flow-token"),
                    "accept": item["headers"].get("accept")} for item in requests[4:19]]
    check_json(actual_flow, [{"method": method, "path": "/host/flow/" + path, "body_bytes": [],
                              "user_agent": ua, "token": token, "accept": "*/*"}
                             for method, path, ua, token in expected_flow])
    check_dns_requests(requests[19:])


def expected_dns_requests():
    queries = [
        ("good", "packet.fixture", 1, "203.0.113.99"), ("good", "ipv6.fixture", 28, "2001:db8:1234:5678::99"),
        ("status", "status.fixture"), ("rcode", "rcode.fixture"),
        ("malformed", "malformed.fixture"), ("delay", "timeout.fixture"),
        ("good", "cache.fixture", 1, "203.0.113.99"), *[("good", "cache.fixture")] * 3,
        ("good", "custom6.fixture", 28, "2001:db8:1234:5678::99"),
        ("empty", "empty.fixture"), ("wrong", "wrong.fixture"), ("nxdomain", "nxdomain.fixture"),
        ("serial-empty", "serial.fixture"), ("serial-good", "serial.fixture"),
        ("race-slow", "race.fixture"), ("race-fast", "race.fixture"),
        ("release", "release.fixture"),
        ("empty", "exhausted.fixture"), ("wrong", "exhausted.fixture"),
        ("good", "injected.fixture"), ("good", "injected-cache.fixture"),
    ]
    return [{"method": "GET", "path": query_path(*query), "body_bytes": [], "accept": "application/dns-message"} for query in queries]


def check_dns_requests(requests):
    expected = expected_dns_requests()
    actual = [{"method": item["method"], "path": item["path"], "body_bytes": item["body_bytes"],
               "accept": item["headers"].get("accept")} for item in requests]
    # Only the two deliberately concurrent requests can arrive in either order.
    actual[16:18] = sorted(actual[16:18], key=lambda item: item["path"])
    expected[16:18] = sorted(expected[16:18], key=lambda item: item["path"])
    check_json(actual, expected)
    check_json([item.get("dns_overlap") for item in requests[16:18]], [True, True])


def expected_native_dns_requests():
    queries = [
        ("udp", "udp-a.fixture", 1, "203.0.113.99"), ("udp", "udp-hostname.fixture"),
        ("udp6", "udp6-aaaa.fixture", 28, "2001:db8:1234:5678::99"),
        ("tcp", "fragment-tcp.fixture"), ("tcp6", "extra-tcp6.fixture", 28),
        ("tcp", "end-header-tcp.fixture"), ("tcp", "end-body-tcp.fixture"),
        ("udp", "malformed-udp.fixture"), ("tcp", "malformed-tcp.fixture"),
        ("udp", "timeout-udp.fixture"), ("tcp", "timeout-tcp.fixture"),
        ("tls", "tls-hostname.fixture"), ("tls", "tls-ip.fixture", 28),
        ("tls", "fragment-tls.fixture"), ("tls", "end-body-tls.fixture"), ("tls", "timeout-tls.fixture"),
        ("udp", "fallback.fixture"), ("tcp", "fallback.fixture"),
        ("tls", "tls-cache.fixture"), ("tcp", "injected-native.fixture"),
    ]
    expected = []
    for transport, domain, *options in queries:
        packet = query_bytes(domain, *options)
        expected.append({"transport": transport, "domain": domain, "packet": list(packet),
                         "frame": [] if transport.startswith("udp") else list(len(packet).to_bytes(2, "big"))})
    return expected


def expected_dns_connections():
    return [{"transport": transport, "handshake": handshake, "server_name": server_name} for transport, handshake, server_name in [
        ("tcp", None, None), ("tcp6", None, None), *[("tcp", None, None)] * 4,
        ("tls", False, "localhost"), ("tls", False, "localhost"), ("tls", True, "localhost"),
        *[("tls", True, None)] * 4, ("tcp", None, None),
        ("tls", True, "localhost"), ("tls", False, "localhost"), ("tcp", None, None),
    ]]


def check_native_dns(requests, connections, errors):
    check_json(errors, [])
    expected = expected_native_dns_requests()
    check_json([{key: item[key] for key in ("transport", "domain", "packet", "frame")} for item in requests], expected)
    check_json([{key: item[key] for key in ("transport", "handshake", "server_name")} for item in connections], expected_dns_connections())
    check_json([item["closed"] for item in connections if item["handshake"] is not False], [True] * 14)
    for item in requests:
        if item["domain"].startswith("fragment-"):
            size = len(response_bytes(bytes(item["packet"]), "good")) + 2
            check_json(item["response_chunks"], [1, 1, 3, 3, 3, size - 11])
            if type(item["response_elapsed"]) not in (float, int) or item["response_elapsed"] < 0.28:
                raise AssertionError("Fragment fixture must outlast the 200 ms inactivity timeout")


def check_host_requests(requests):
    text = list("订阅🙂\r\nsecond\n".encode("utf-8"))
    expected = []

    def add(method, path, body=None, accept="*/*", probe=None, authorization=None):
        expected.append({"method": method, "path": path, "body_bytes": body or [],
                         "headers": {"accept": accept, "x-probe": probe, "authorization": authorization}})

    for method in ("GET", "POST", "PUT", "DELETE", "HEAD", "OPTIONS", "PATCH"):
        add(method, "/host/echo", [] if method in ("GET", "HEAD") else text, probe=method)
    add("GET", "/host/binary")
    add("GET", "/host/text")
    for _ in range(2):
        add("POST", "/host/echo", [0, 255, 13, 10], accept=None)
    add("GET", "/host/echo", accept="application/json", authorization="Basic dXNlckBuYW1lOnBAc3M6JTIw")
    add("GET", "/host/echo", probe="hook")
    add("GET", "/host/echo")
    add("GET", "/host/delay")
    add("GET", "/host/echo")
    add("GET", "/host/status")
    for count in (2, 1):
        add("GET", f"/host/redirect/{count}")
    add("GET", "/host/text")
    for count in (3, 2, 1):
        add("GET", f"/host/redirect/{count}")
    add("GET", "/host/text")
    for count in (4, 3, 2, 1):
        add("GET", f"/host/redirect/{count}")
    actual = [{"method": item["method"], "path": item["path"], "body_bytes": item["body_bytes"],
               "headers": {key: item["headers"].get(key) for key in ("accept", "x-probe", "authorization")}}
              for item in requests]
    check_json(actual, expected)


def run_container(image, state, upstream, mode, environment, output, name):
    container = f"sub-store-{output.name}-{name}".lower()
    argv = ["docker", "run", "--name", container, "--network", "host", "--read-only",
            "--user", f"{os.getuid()}:{os.getgid()}",
            "--mount", f"type=bind,source={state},target=/state",
            "--env", "SUB_STORE_COMPAT_STATE_DIR=/state", "--env", "SUB_STORE_DATA_BASE_PATH=/state",
            "--env", "SUB_STORE_COMPAT_DNS_SERVERS=" + environment.get("SUB_STORE_COMPAT_DNS_SERVERS", "{}"),
            "--env", f"SUB_STORE_COMPAT_UPSTREAM={upstream}", "--env", "TZ=UTC"]
    for kind in ("COUNTRY", "ASN"):
        key = f"SUB_STORE_MMDB_{kind}_PATH"
        if key in environment:
            argv.extend(["--env", key + "=/state/" + Path(environment[key]).name])
    argv.extend([image, *mode])
    try:
        return execute(argv, ROOT, environment, output, name, timeout=60)
    finally:
        # Killing the Docker CLI alone leaves an evaluated container running.
        removed = execute(["docker", "rm", "--force", container], ROOT, environment, output, name + "-cleanup", timeout=30)
        if removed.returncode:
            raise RuntimeError(f"{name}: could not remove owned container; inspect preserved logs")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--node", type=Path, required=True, help="Pinned development-only Node 24.15.0 oracle")
    parser.add_argument("--docker", action="store_true", help="Build and run the release probe in native Linux Docker containers")
    args = parser.parse_args()
    node = args.node.resolve(strict=True)
    output = HERE / "evidence" / (("docker-probe-" if args.docker else "rust-probe-") + datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%S.%fZ"))
    output.mkdir(parents=True)
    report = {"scope": "M1 script feasibility experiment", "production_ready": False, "full_host_parity": False,
              "os": platform.system(), "machine": platform.machine(), "execution": "docker" if args.docker else "native",
              "exit_code": 1, "checks": []}
    image = None
    try:
        environment = {key: value for key, value in os.environ.items()
                       if not key.startswith("SUB_STORE_") and key.lower() not in ("http_proxy", "https_proxy", "all_proxy", "no_proxy")}
        for variable in ("CARGO_BUILD_TARGET", "RUSTFLAGS", "CARGO_ENCODED_RUSTFLAGS"):
            environment.pop(variable, None)
        environment["CARGO_TARGET_DIR"] = str(ROOT / "rust/target")
        environment["PATH"] = str(node.parent) + os.pathsep + environment.get("PATH", "")
        report.update(source_provenance(environment, output))
        version = execute([node, "--version"], ROOT, environment, output, "node-toolchain")
        if version.returncode or version.stdout.strip() != "v24.15.0":
            raise RuntimeError("The shared host oracle requires Node v24.15.0")
        if args.docker and platform.system() != "Linux":
            raise RuntimeError("Container acceptance requires a native Linux runner")
        executable = ROOT / "rust/target/debug/examples" / ("script_probe.exe" if os.name == "nt" else "script_probe")
        mmdb_manifest = verify_mmdb_fixtures()
        report["mmdb_fixture_commit"] = mmdb_manifest["commit"]
        mmdb_files = [str((MMDB_FIXTURES / item["file"]).relative_to(ROOT)).replace("\\", "/") for item in mmdb_manifest["files"]]
        for relative in ("rust/Cargo.toml", "rust/Cargo.lock", "rust/examples/script_probe.rs", "rust/examples/support/script_host.rs", "rust/examples/support/dns.rs", "rust/examples/support/mmdb.rs",
                         "tests/compat/probe.py", "tests/compat/reference.py", "tests/compat/upstream.py", "tests/compat/register.cjs",
                         "tests/compat/host-reference.cjs", "scripts/ip-flag-node.js", "tests/compat/fixtures/script-host-probe.js",
                         "tests/compat/fixtures/script-assertions.js", "tests/compat/fixtures/script-host.js",
                         "tests/compat/fixtures/script-host-contracts.js", "tests/compat/.generated/script-helpers.js",
                         "tests/compat/build-helpers.cjs", "tests/compat/processor-reference.cjs",
                         "tests/compat/fixtures/script-processor-clock.js", "tests/compat/fixtures/script-processor-contracts.js",
                         "tests/compat/fixtures/script-helper-contracts.js", "tests/compat/fixtures/script-dns-contracts.js",
                         "tests/compat/fixtures/script-native-dns-contracts.js", "tests/compat/fixtures/dns-cert.pem", "tests/compat/fixtures/dns-key.pem",
                         "tests/compat/dns_fixtures.py", "tests/compat/mmdb_fixtures.py", "tests/compat/fixtures/script-mmdb-contracts.js",
                         "tests/compat/fixtures/mmdb/provenance.json", "tests/compat/Probe.Dockerfile",
                         "tests/compat/Probe.Dockerfile.dockerignore", *mmdb_files):
            report.setdefault("source_sha256", {})[relative] = hashlib.sha256((ROOT / relative).read_bytes()).hexdigest()
        native_checks = (("toolchain", ["rustc", "--version", "--verbose"]),
                         ("fmt", ["cargo", "fmt", "--manifest-path", "rust/Cargo.toml", "--", "--check"]),
                         ("clippy", ["cargo", "clippy", "--locked", "--manifest-path", "rust/Cargo.toml", "--all-targets", "--", "-D", "warnings"]),
                         ("probe-build", ["cargo", "build", "--locked", "--manifest-path", "rust/Cargo.toml", "--example", "script_probe"]))
        container_tag = "sub-store-compat:" + output.name.lower()
        container_checks = (("docker-version", ["docker", "version"]),
                            ("container-build", ["docker", "build", "--progress=plain", "--file", "tests/compat/Probe.Dockerfile",
                                                 "--tag", container_tag, "."]))
        for name, argv in container_checks if args.docker else native_checks:
            result = execute(argv, ROOT, environment, output, name)
            report["checks"].append({"name": name, "exit_code": result.returncode})
            if result.returncode:
                raise RuntimeError(f"{name} failed; inspect preserved logs")
        if args.docker:
            image = container_tag
            inspected = execute(["docker", "image", "inspect", image], ROOT, environment, output, "container-image")
            if inspected.returncode:
                raise RuntimeError("Could not inspect the built image")
            report["container_image"] = json.loads(inspected.stdout)[0]
            check_json(report["container_image"]["Os"], "linux")
            check_json(report["container_image"]["Architecture"], {"x86_64": "amd64", "aarch64": "arm64"}[platform.machine().lower()])
        else:
            report["binary_sha256"] = hashlib.sha256(executable.read_bytes()).hexdigest()
        with ControlledUpstream() as upstream, ControlledDns() as dns:
            report["dns_fixture_urls"] = dns.urls
            for suite in ("host", "processor"):
                observed = {}
                filename = "sub-store-probe.json" if suite == "host" else "sub-store.json"
                root_seed = {} if suite == "host" else {key: "{}" for key in (
                    "sub-store-cached-resource", "sub-store-cached-headers-resource", "sub-store-cached-script-resource")}
                for kind in ("node", "rust"):
                    state = output / (f"{kind}-state" if suite == "host" else f"{kind}-processor-state")
                    state.mkdir()
                    (state / filename).write_text('{"settings":{"logsMaxCount":0}}\n', encoding="utf-8")
                    (state / "root.json").write_text(json.dumps(root_seed) + "\n", encoding="utf-8")
                    child_env = {**environment, "SUB_STORE_DATA_BASE_PATH": str(state), "SUB_STORE_COMPAT_STATE_DIR": str(state),
                                 "SUB_STORE_COMPAT_UPSTREAM": upstream.url, "SUB_STORE_COMPAT_DNS_SERVERS": json.dumps(dns.urls)}
                    if suite == "processor":
                        child_env.update(prepare_mmdb(state))
                        child_env["TZ"] = "UTC"
                        if kind == "node":
                            child_env["NODE_DEBUG"] = "tls"
                    argv = [node, "--require", HERE / "register.cjs", HERE / f"{suite}-reference.cjs"] if kind == "node" else [executable]
                    for resume in (False, True):
                        name = f"{kind}-{suite}" + ("-resume" if resume else "")
                        mode = (["--resume"] if resume else []) if kind == "node" else (
                            (["--host-resume"] if resume else []) if suite == "host" else
                            ["--processors-resume" if resume else "--processors"])
                        offset = len(upstream.requests)
                        dns_offset, connection_offset, error_offset = len(dns.requests), len(dns.connections), len(dns.errors)
                        # Babel's default dependency exclusion is relative to cwd, as in the real unit suite.
                        cwd = ROOT / "backend" if kind == "node" and suite == "processor" else ROOT
                        if kind == "rust" and args.docker:
                            result = run_container(image, state, upstream.url, mode, child_env, output, name)
                        else:
                            result = execute([*argv, *mode], cwd, child_env, output, name, timeout=60,
                                direct_reason="Direct native process ownership allows the watchdog to kill the evaluated host on Windows")
                        dns.wait_for_idle()
                        requests = upstream.requests[offset:]
                        runs = report.setdefault(f"{suite}_runs", [])
                        runs.append({"name": name, "exit_code": result.returncode, "upstream_requests": requests,
                                     "dns_requests": dns.requests[dns_offset:], "dns_connections": dns.connections[connection_offset:],
                                     "dns_errors": dns.errors[error_offset:]})
                        if suite == "processor":
                            runs[-1]["progress"] = [json.loads(line.removeprefix("COMPAT_PROCESSOR "))
                                for line in result.stderr.splitlines() if line.startswith("COMPAT_PROCESSOR ")]
                        if result.returncode:
                            raise RuntimeError(f"{name} failed; inspect preserved logs")
                        if suite == "processor":
                            check_processor_progress(runs[-1]["progress"], resume)
                        lines = result.stdout.splitlines()
                        prefix = f"COMPAT_{suite.upper()}_RESULT "
                        value = json.loads(next(line.removeprefix(prefix) for line in lines if line.startswith(prefix))
                                           if kind == "node" else lines[-1])
                        if kind == "rust" and suite == "host" and not resume:
                            report["probe"] = value
                            expected_arch = {"amd64": "x86_64", "x86_64": "x86_64", "aarch64": "aarch64", "arm64": "aarch64"}.get(platform.machine().lower())
                            if value["os"] != platform.system().lower() or value["arch"] != expected_arch:
                                raise RuntimeError("Probe binary does not match the native runner's OS/architecture")
                            value = value["host"]
                        runs[-1]["result"] = value
                        persisted = {key: json.loads((state / file).read_text(encoding="utf-8"))
                                     for key, file in (("cache", filename), ("root", "root.json"))}
                        runs[-1]["persisted"] = persisted
                        if suite == "processor":
                            runs[-1]["mmdb_state_sha256"] = check_mmdb_state(state)
                        check_json(value["storage"], persisted)
                        if resume:
                            expected = {"storage": observed[kind]["storage"]}
                            if suite == "processor":
                                expected["resourceCache"] = observed[kind]["resourceCache"]
                                expected["headersCache"] = observed[kind]["headersCache"]
                                expected["dnsCache"] = observed[kind]["dnsCache"]
                            check_json(value, expected)
                            check_json(requests, [])
                            for key in ("dns_requests", "dns_connections", "dns_errors"):
                                check_json(runs[-1][key], [])
                        else:
                            observed[kind] = value
                            (check_host if suite == "host" else check_processors)(value, observed["node"])
                            (check_host_requests if suite == "host" else check_processor_requests)(requests)
                            if suite == "processor":
                                check_native_dns(runs[-1]["dns_requests"], runs[-1]["dns_connections"], runs[-1]["dns_errors"])
                            else:
                                for key in ("dns_requests", "dns_connections", "dns_errors"):
                                    check_json(runs[-1][key], [])
        report["exit_code"] = 0
    except Exception as error:
        report["error"] = f"{type(error).__name__}: {error}"
    finally:
        if image:
            try:
                removed = execute(["docker", "image", "rm", image], ROOT, environment, output, "container-image-cleanup", timeout=30)
                if removed.returncode:
                    raise RuntimeError("Could not remove the owned image; inspect preserved logs")
            except Exception as error:
                report.update(exit_code=1, cleanup_error=f"{type(error).__name__}: {error}")
    (output / "summary.json").write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({"scope": report["scope"], "exit_code": report["exit_code"], "os": report["os"], "machine": report["machine"], "evidence": str(output)}))
    return report["exit_code"]


if __name__ == "__main__":
    if not __debug__:
        raise SystemExit("Script feasibility checks require enabled assertions")
    raise SystemExit(main())
