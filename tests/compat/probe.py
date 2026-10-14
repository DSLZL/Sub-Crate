"""Capture native execution of the contained rquickjs experiment."""
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import platform

from reference import execute, HERE, ROOT


def main():
    output = HERE / "evidence" / ("rust-probe-" + datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%S.%fZ"))
    output.mkdir(parents=True)
    report = {"scope": "M1 script feasibility experiment", "production_ready": False, "full_host_parity": False,
              "os": platform.system(), "machine": platform.machine(), "exit_code": 1, "checks": []}
    try:
        environment = dict(os.environ)
        for variable in ("CARGO_BUILD_TARGET", "RUSTFLAGS", "CARGO_ENCODED_RUSTFLAGS"):
            environment.pop(variable, None)
        for relative in ("rust/Cargo.lock", "rust/examples/script_probe.rs", "scripts/ip-flag-node.js",
                         "tests/compat/fixtures/script-host-probe.js", "tests/compat/fixtures/script-assertions.js",
                         "tests/compat/.generated/script-helpers.js"):
            report.setdefault("source_sha256", {})[relative] = hashlib.sha256((ROOT / relative).read_bytes()).hexdigest()
        for name, argv in (("toolchain", ["rustc", "--version", "--verbose"]),
                           ("fmt", ["cargo", "fmt", "--manifest-path", "rust/Cargo.toml", "--", "--check"]),
                           ("clippy", ["cargo", "clippy", "--locked", "--manifest-path", "rust/Cargo.toml", "--all-targets", "--", "-D", "warnings"]),
                           ("native-probe", ["cargo", "run", "--locked", "--manifest-path", "rust/Cargo.toml", "--example", "script_probe"])):
            result = execute(argv, ROOT, environment, output, name)
            report["checks"].append({"name": name, "exit_code": result.returncode})
            if result.returncode:
                raise RuntimeError(f"{name} failed; inspect preserved logs")
            if name == "native-probe":
                report["probe"] = json.loads(result.stdout.strip().splitlines()[-1])
                expected_arch = {"amd64": "x86_64", "x86_64": "x86_64", "aarch64": "aarch64", "arm64": "aarch64"}.get(platform.machine().lower())
                if report["probe"]["os"] != platform.system().lower() or report["probe"]["arch"] != expected_arch:
                    raise RuntimeError("Probe binary does not match the native runner's OS/architecture")
        report["exit_code"] = 0
    except Exception as error:
        report["error"] = f"{type(error).__name__}: {error}"
    (output / "summary.json").write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({"scope": report["scope"], "exit_code": report["exit_code"], "os": report["os"], "machine": report["machine"], "evidence": str(output)}))
    return report["exit_code"]


if __name__ == "__main__":
    raise SystemExit(main())
