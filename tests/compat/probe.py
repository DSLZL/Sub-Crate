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
]


def check_host(actual, expected):
    check_json(actual["passed"], HOST_CASES)
    check_json(actual, expected)


def check_processors(actual, expected):
    check_json(actual["passed"], PROCESSOR_CASES)
    check_json(actual, expected)


def check_processor_requests(requests):
    expected = [
        {"method": "GET", "path": "/host/echo?name=zero", "body_bytes": [], "probe": "processor-shortcut", "accept": "*/*"},
        {"method": "GET", "path": "/host/echo?name=false", "body_bytes": [], "probe": "processor-shortcut", "accept": "*/*"},
        {"method": "POST", "path": "/host/echo", "body_bytes": list("订阅🙂\r\n".encode()), "probe": "processor-response", "accept": "*/*"},
        {"method": "POST", "path": "/host/echo", "body_bytes": list("嵌套🙂\r\n".encode()), "probe": "processor-nested", "accept": "*/*"},
    ]
    actual = [{"method": item["method"], "path": item["path"], "body_bytes": item["body_bytes"],
               "probe": item["headers"].get("x-probe"), "accept": item["headers"].get("accept")}
              for item in requests]
    check_json(actual, expected)


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


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--node", type=Path, required=True, help="Pinned development-only Node 24.15.0 oracle")
    args = parser.parse_args()
    node = args.node.resolve(strict=True)
    output = HERE / "evidence" / ("rust-probe-" + datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%S.%fZ"))
    output.mkdir(parents=True)
    report = {"scope": "M1 script feasibility experiment", "production_ready": False, "full_host_parity": False,
              "os": platform.system(), "machine": platform.machine(), "exit_code": 1, "checks": []}
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
        executable = ROOT / "rust/target/debug/examples" / ("script_probe.exe" if os.name == "nt" else "script_probe")
        for relative in ("rust/Cargo.toml", "rust/Cargo.lock", "rust/examples/script_probe.rs", "rust/examples/support/script_host.rs",
                         "tests/compat/probe.py", "tests/compat/reference.py", "tests/compat/upstream.py", "tests/compat/register.cjs",
                         "tests/compat/host-reference.cjs", "scripts/ip-flag-node.js", "tests/compat/fixtures/script-host-probe.js",
                         "tests/compat/fixtures/script-assertions.js", "tests/compat/fixtures/script-host.js",
                         "tests/compat/fixtures/script-host-contracts.js", "tests/compat/.generated/script-helpers.js",
                         "tests/compat/build-helpers.cjs", "tests/compat/processor-reference.cjs",
                         "tests/compat/fixtures/script-processor-clock.js", "tests/compat/fixtures/script-processor-contracts.js"):
            report.setdefault("source_sha256", {})[relative] = hashlib.sha256((ROOT / relative).read_bytes()).hexdigest()
        for name, argv in (("toolchain", ["rustc", "--version", "--verbose"]),
                           ("fmt", ["cargo", "fmt", "--manifest-path", "rust/Cargo.toml", "--", "--check"]),
                           ("clippy", ["cargo", "clippy", "--locked", "--manifest-path", "rust/Cargo.toml", "--all-targets", "--", "-D", "warnings"]),
                           ("probe-build", ["cargo", "build", "--locked", "--manifest-path", "rust/Cargo.toml", "--example", "script_probe"])):
            result = execute(argv, ROOT, environment, output, name)
            report["checks"].append({"name": name, "exit_code": result.returncode})
            if result.returncode:
                raise RuntimeError(f"{name} failed; inspect preserved logs")
        report["binary_sha256"] = hashlib.sha256(executable.read_bytes()).hexdigest()
        with ControlledUpstream() as upstream:
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
                                 "SUB_STORE_COMPAT_UPSTREAM": upstream.url}
                    argv = [node, "--require", HERE / "register.cjs", HERE / f"{suite}-reference.cjs"] if kind == "node" else [executable]
                    for resume in (False, True):
                        name = f"{kind}-{suite}" + ("-resume" if resume else "")
                        mode = (["--resume"] if resume else []) if kind == "node" else (
                            (["--host-resume"] if resume else []) if suite == "host" else
                            ["--processors-resume" if resume else "--processors"])
                        offset = len(upstream.requests)
                        # Babel's default dependency exclusion is relative to cwd, as in the real unit suite.
                        cwd = ROOT / "backend" if kind == "node" and suite == "processor" else ROOT
                        result = execute([*argv, *mode], cwd, child_env, output, name, timeout=60,
                            direct_reason="Direct native process ownership allows the watchdog to kill the evaluated host on Windows")
                        requests = upstream.requests[offset:]
                        runs = report.setdefault(f"{suite}_runs", [])
                        runs.append({"name": name, "exit_code": result.returncode, "upstream_requests": requests})
                        if result.returncode:
                            raise RuntimeError(f"{name} failed; inspect preserved logs")
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
                        check_json(value["storage"], persisted)
                        if resume:
                            expected = {"storage": observed[kind]["storage"]}
                            if suite == "processor":
                                expected["resourceCache"] = observed[kind]["resourceCache"]
                            check_json(value, expected)
                            check_json(requests, [])
                        else:
                            observed[kind] = value
                            (check_host if suite == "host" else check_processors)(value, observed["node"])
                            (check_host_requests if suite == "host" else check_processor_requests)(requests)
        report["exit_code"] = 0
    except Exception as error:
        report["error"] = f"{type(error).__name__}: {error}"
    (output / "summary.json").write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({"scope": report["scope"], "exit_code": report["exit_code"], "os": report["os"], "machine": report["machine"], "evidence": str(output)}))
    return report["exit_code"]


if __name__ == "__main__":
    raise SystemExit(main())
