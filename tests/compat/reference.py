"""Run the frozen Node oracle with pinned tools and fresh task-owned state."""
import argparse
from contextlib import contextmanager
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import socket
import subprocess
import sys
import time

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent.parent
BACKEND = ROOT / "backend"


def execute(argv, directory, environment, output_dir, name, timeout=None, direct_reason=None):
    started = datetime.now(timezone.utc).isoformat()
    command = [*map(str, argv)]
    rtk = None if direct_reason else shutil.which("rtk", path=environment.get("PATH"))
    if rtk:
        command = [rtk, "proxy", *command]
    timed_out = False
    try:
        process = subprocess.run(command, cwd=directory, env=environment, timeout=timeout,
                                 capture_output=True, text=True, encoding="utf-8", errors="replace")
    except subprocess.TimeoutExpired as error:
        timed_out = True
        stdout = error.stdout.decode("utf-8", errors="replace") if isinstance(error.stdout, bytes) else error.stdout or ""
        stderr = error.stderr.decode("utf-8", errors="replace") if isinstance(error.stderr, bytes) else error.stderr or ""
        process = subprocess.CompletedProcess(command, 124, stdout, stderr + f"\nCommand exceeded {timeout}s timeout\n")
    (output_dir / f"{name}.stdout.log").write_text(process.stdout, encoding="utf-8", newline="")
    (output_dir / f"{name}.stderr.log").write_text(process.stderr, encoding="utf-8", newline="")
    result = {"argv": command, "rtk_exception": direct_reason or (None if rtk else "RTK unavailable on this runner"),
              "cwd": str(directory), "started": started,
              "finished": datetime.now(timezone.utc).isoformat(), "exit_code": process.returncode, "timed_out": timed_out}
    (output_dir / f"{name}.command.json").write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")
    return process


@contextmanager
def oracle(node, environment, output_dir, summary, log_prefix="server"):
    with socket.socket() as reserved:
        reserved.bind(("127.0.0.1", 0))
        port = reserved.getsockname()[1]
    environment = {**environment, "SUB_STORE_BACKEND_API_PORT": str(port)}
    origin = f"http://127.0.0.1:{port}"
    if environment.get("SUB_STORE_FRONTEND_PATH"):
        environment.setdefault("SUB_STORE_CORS_ALLOWED_ORIGINS", origin)
        summary["fixture_cors_origin"] = environment["SUB_STORE_CORS_ALLOWED_ORIGINS"]
    backend_url = origin + environment["SUB_STORE_FRONTEND_BACKEND_PATH"].rstrip("/")
    bundle = BACKEND / "dist/sub-store.bundle.js"
    from runner import request
    with (output_dir / f"{log_prefix}.stdout.log").open("w", encoding="utf-8") as stdout, (output_dir / f"{log_prefix}.stderr.log").open("w", encoding="utf-8") as stderr:
        # RTK's wrapper leaves Node alive on Windows after terminate(); own the child directly.
        child = subprocess.Popen([str(node), str(bundle)], cwd=BACKEND, env=environment, stdout=stdout, stderr=stderr)
        summary.update(server_argv=[str(node), str(bundle)], server_pid=child.pid, server_url=backend_url,
                       server_origin=origin, rtk_exception="Direct listener ownership required for Windows process cleanup; short commands use RTK where available")
        try:
            deadline = time.monotonic() + 30
            while True:
                if child.poll() is not None:
                    raise RuntimeError("Node oracle exited before readiness; inspect preserved server logs")
                try:
                    if request(backend_url, {"path": "/api/utils/env"})["status"] == 200:
                        break
                except OSError:
                    pass
                if time.monotonic() >= deadline:
                    raise RuntimeError("Node oracle readiness timeout")
                time.sleep(0.1)
            yield backend_url
        finally:
            child.terminate()
            try:
                summary["server_exit_code"] = child.wait(timeout=5)
            except subprocess.TimeoutExpired:
                child.kill()
                summary["server_exit_code"] = child.wait(timeout=5)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--node", required=True, type=Path, help="Pinned Node 24.15.0 executable")
    parser.add_argument("--mode", choices=("unit", "unit-windows", "contracts", "helpers"), required=True)
    parser.add_argument("--suite", default="smoke")
    args = parser.parse_args()
    node = args.node.resolve(strict=True)
    run_id = datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%S.%fZ")
    output_dir = HERE / "evidence" / f"node-{args.mode}-{run_id}"
    output_dir.mkdir(parents=True)
    state_dir = output_dir / "state"
    state_dir.mkdir()
    environment = {key: value for key, value in os.environ.items() if not key.startswith("SUB_STORE_")}
    environment["PATH"] = str(node.parent) + os.pathsep + environment.get("PATH", "")
    environment.update(SUB_STORE_DATA_BASE_PATH=str(state_dir), SUB_STORE_FRONTEND_BACKEND_PATH="/", SUB_STORE_BACKEND_API_HOST="127.0.0.1")
    summary = {"kind": "node-baseline", "production_ready": False, "rust_parity": False, "full_coverage": False,
               "backend_commit": "9cc6b5fac966993364ae33da8797f446bbf7f571", "mode": args.mode, "exit_code": 1,
               "state_dir": str(state_dir), "lock_sha256": hashlib.sha256((BACKEND / "pnpm-lock.yaml").read_bytes()).hexdigest()}
    try:
        version = execute([node, "--version"], ROOT, environment, output_dir, "toolchain")
        if version.returncode or version.stdout.strip() != "v24.15.0":
            raise RuntimeError(f"Expected Node v24.15.0; observed {version.stdout.strip()!r}")
        if args.mode in ("unit", "unit-windows"):
            loader = str(HERE / "register.cjs") if args.mode == "unit-windows" else "@babel/register"
            summary["loader"] = loader
            summary["loader_adjustment"] = "Same Babel preset, native absolute @/ aliases instead of pinned plugin's broken Windows filename handling; no source edits" if args.mode == "unit-windows" else None
            result = execute([node, BACKEND / "node_modules/mocha/bin/_mocha", "src/test/**/*.spec.js",
                              "--require", loader, "--recursive", "--reporter", "json"], BACKEND, environment, output_dir, "mocha")
            match = re.search(r'\{\s*"stats"\s*:', result.stdout)
            if not match:
                raise RuntimeError("Mocha JSON result was not produced; inspect preserved logs")
            report, _ = json.JSONDecoder().raw_decode(result.stdout[match.start():])
            summary["stats"] = report["stats"]
            summary["pending"] = [test["fullTitle"] for test in report["pending"]]
            summary["failures"] = [{"name": test["fullTitle"], "error": test["err"]} for test in report["failures"]]
            summary["exit_code"] = 0 if result.returncode == 0 and report["stats"]["tests"] > 0 and not report["pending"] and not report["failures"] else 1
        elif args.mode == "helpers":
            result = execute([node, HERE / "build-helpers.cjs"], ROOT, environment, output_dir, "helpers")
            summary["exit_code"] = result.returncode
        else:
            result = execute([node, "bundle-esbuild.js"], BACKEND, environment, output_dir, "bundle")
            if result.returncode:
                raise RuntimeError("Pinned backend bundling failed; inspect preserved logs")
            bundle = BACKEND / "dist/sub-store.bundle.js"
            summary["bundle_sha256"] = hashlib.sha256(bundle.read_bytes()).hexdigest()
            phases = [(args.suite, "contracts", {})]
            if args.suite == "smoke":
                phases.extend([("restart", "contracts-restart", {}), ("merged-share", "contracts-merged-share",
                    {"SUB_STORE_BACKEND_MERGE": "true", "SUB_STORE_FRONTEND_BACKEND_PATH": "/backend"})])
            elif args.suite == "merged-share":
                phases[0] = (args.suite, "contracts", {"SUB_STORE_BACKEND_MERGE": "true", "SUB_STORE_FRONTEND_BACKEND_PATH": "/backend"})
            for suite, phase, overrides in phases:
                listener = {"suite": suite}
                summary.setdefault("listeners", []).append(listener)
                with oracle(node, {**environment, **overrides}, output_dir, listener, log_prefix=phase + "-server") as backend_url:
                    result = execute([sys.executable, HERE / "runner.py", "--url", backend_url, "--kind", "node-baseline", "--suite", suite,
                                      "--state-dir", state_dir, "--evidence", output_dir / f"{phase}.json"], ROOT, environment, output_dir, phase)
                summary["exit_code"] = result.returncode
                if result.returncode:
                    break
    except Exception as error:
        summary["error"] = f"{type(error).__name__}: {error}"
    finally:
        (output_dir / "summary.json").write_text(json.dumps(summary, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
    print(json.dumps({"mode": args.mode, "exit_code": summary["exit_code"], "evidence": str(output_dir), **({"stats": summary["stats"]} if "stats" in summary else {})}, ensure_ascii=False))
    return summary["exit_code"]


if __name__ == "__main__":
    raise SystemExit(main())
