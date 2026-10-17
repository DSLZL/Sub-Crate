"""CI acceptance for the actual Rust HTTP listener, separate from the JS probe."""
import argparse
import base64
from contextlib import contextmanager
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import platform
import shutil
import socket
import subprocess
import time

from reference import BACKEND, HERE, ROOT, execute, oracle, source_provenance
from runner import check_json, load_cases, request, run_cases


@contextmanager
def listener(binary, environment, output, report, log_prefix="rust", image=None):
    with socket.socket() as reserved:
        reserved.bind(("127.0.0.1", 0))
        port = reserved.getsockname()[1]
    environment = {**environment, "SUB_STORE_BACKEND_API_PORT": str(port)}
    origin = f"http://127.0.0.1:{port}"
    if environment.get("SUB_STORE_FRONTEND_PATH"):
        environment.setdefault("SUB_STORE_CORS_ALLOWED_ORIGINS", origin)
        report["fixture_cors_origin"] = environment["SUB_STORE_CORS_ALLOWED_ORIGINS"]
    url = origin + environment.get("SUB_STORE_FRONTEND_BACKEND_PATH", "/").rstrip("/")
    name = f"m2-{os.getpid()}-{port}"
    if image:
        argv = ["docker", "run", "--rm", "--name", name, "--read-only", "--cap-drop=ALL",
                "--security-opt=no-new-privileges", "--user", f"{os.getuid()}:{os.getgid()}",
                "-p", f"127.0.0.1:{port}:3000",
                "--mount", f"type=bind,source={environment['SUB_STORE_DATA_BASE_PATH']},target=/data"]
        for key, value in environment.items():
            if key.startswith("SUB_STORE_"):
                value = {"SUB_STORE_DATA_BASE_PATH": "/data", "SUB_STORE_BACKEND_API_PORT": "3000",
                         "SUB_STORE_BACKEND_API_HOST": "0.0.0.0"}.get(key, value)
                argv.extend(["--env", f"{key}={value}"])
        argv.append(image)
    else:
        argv = [str(binary)]
        # No executable search path is available to the production Rust process.
        environment = {**environment, "PATH": ""}
    record = {"argv": argv, "pid": None, "url": url,
              "rtk_exception": "Direct ownership is required to stop the listener and preserve native exit status"}
    report.setdefault("processes", []).append(record)
    with (output / f"{log_prefix}.stdout.log").open("w", encoding="utf-8") as stdout, (output / f"{log_prefix}.stderr.log").open("w", encoding="utf-8") as stderr:
        child = subprocess.Popen(argv, cwd=ROOT, env=environment, stdout=stdout, stderr=stderr)
        record["pid"] = child.pid
        report.update(server_url=url, server_origin=origin, server_argv=argv, server_pid=child.pid)
        try:
            deadline = time.monotonic() + 30
            while True:
                if child.poll() is not None:
                    raise RuntimeError(f"Rust listener exited before readiness: {child.returncode}")
                try:
                    if request(url, {"path": "/api/utils/env"})["status"] == 200:
                        break
                except OSError:
                    pass
                if time.monotonic() >= deadline:
                    raise RuntimeError("Rust listener readiness timeout")
                time.sleep(0.1)
            yield url
        finally:
            if image:
                execute(["docker", "rm", "--force", name], ROOT, os.environ.copy(), output, f"{log_prefix}-remove", timeout=30)
            if child.poll() is None:
                child.terminate()
            try:
                record["exit_code"] = child.wait(timeout=5)
            except subprocess.TimeoutExpired:
                child.kill()
                record["exit_code"] = child.wait(timeout=5)


def check_cases(url, state, cases, output, name):
    results = run_cases(url, state, cases)
    (output / f"{name}.json").write_text(json.dumps(results, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    assert len(results) == len(cases) and all(case["status"] == "passed" for case in results), f"{name}: {[(case['id'], case.get('error')) for case in results if case['status'] != 'passed']}"
    return results


def state_edges(url, state, keys, output, name):
    """Compare actual HTTP values and persisted state without response normalization."""
    observations = []

    def send(method, path, body, status):
        response = request(url, {"method": method, "path": path, "body": body})
        observation = {"method": method, "path": path, "body": body, "response": response,
                       "state": json.loads((state / "sub-store.json").read_text(encoding="utf-8"))}
        observations.append(observation)
        with (output / f"{name}.jsonl").open("a", encoding="utf-8") as evidence:
            evidence.write(json.dumps(observation, ensure_ascii=False) + "\n")
        assert response["status"] == status, f"{name} {method} {path}: {response}"
        observation["response"] = {"status": response["status"], "json": json.loads(response["body"])}
        return observation["response"]["json"]

    numeric_fields = ("defaultTimeout", "githubApiTimeout", "artifactSyncBatchSize", "cacheThreshold",
                      "resourceCacheTtl", "headersCacheTtl", "scriptCacheTtl", "logsMaxCount",
                      "backendRequestConcurrency", "backendRequestConcurrencyWaitTime")
    values = [None, False, True, 0, -1, 0.5, 2, 2.5, "", " ", "1", "1.5", "0x20", "0X20",
              "0b10", "0B10", "0o10", "0O10", "+0x10", "-0b1", "0x", "Infinity", "NaN",
              "\ufeff2\ufeff", "\u00852", "\u20032\u2028", [], [None], [False], [True], ["2"],
              [[["2"]]], ["2", "3"], [{}], {}, ["0x20"], [[None]], [-0.0], [[[]]]]
    for value in values:
        send("PATCH", "/api/settings", dict.fromkeys(numeric_fields, value), 200)
    send("PATCH", "/api/settings", dict.fromkeys(numeric_fields), 200)
    for plural, singular in (("subs", "sub"), ("collections", "collection")):
        for index, key in enumerate(keys["valid"]):
            item = {"name": f"key-{index}", "age-public-key": f"\ufeff \n{key}\r ",
                    "editorLanguage": "invalid", "previewLanguage": "yaml",
                    "process": [{"type": "Script Operator", "args": {"content": "// 保留🙂\r\n", "editorLanguage": "bad"}}],
                    "custom": {"null": None, "false": False, "zero": 0}}
            created = send("POST", f"/api/{plural}", item, 201)["data"]
            assert created["age-public-key"] == key and "editorLanguage" not in created
            assert created["process"][0]["args"] == {"content": "// 保留🙂\r\n"}
            path = f"/api/{singular}/key-{index}"
            for blank in (None, "", "\ufeff \n", [], [None], [[[]]]):
                assert "age-public-key" not in send("PATCH", path, {"age-public-key": blank}, 200)["data"]
            assert send("PATCH", path, {"age-public-key": [[key]]}, 200)["data"]["age-public-key"] == key
            for invalid in [*keys["invalid"], "age1invalid", "age1tag1invalid", "age1tagpq1invalid", 0, False, {}, [key, ""], "\u0085" + key]:
                before = (state / "sub-store.json").read_bytes()
                rejected = send("PATCH", path, {"age-public-key": invalid}, 500)
                assert rejected["error"]["code"] == "INVALID_AGE_PUBLIC_KEY"
                assert (state / "sub-store.json").read_bytes() == before, "Invalid recipient changed durable state"
            rejected = send("POST", f"/api/{plural}", {"name": "bad/name", "age-public-key": "age1invalid"}, 500)
            assert rejected["error"]["code"] == "INVALID_AGE_PUBLIC_KEY", "Recipient validation order changed"
        before = (state / "sub-store.json").read_bytes()
        rejected = send("PUT", f"/api/{plural}", [{"name": "valid", "age-public-key": keys["valid"][0]},
                                                   {"name": "invalid", "age-public-key": keys["invalid"][0]}], 500)
        assert rejected["error"]["code"] == "INVALID_AGE_PUBLIC_KEY"
        assert (state / "sub-store.json").read_bytes() == before, "Rejected bulk replacement changed state"
        send("PUT", f"/api/{plural}", [{"name": f"bulk-{index}", "age-public-key": [key], "previewLanguage": "bad"}
                                       for index, key in enumerate(keys["valid"])], 200)
    (output / f"{name}.json").write_text(json.dumps(observations, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    return observations


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--node", required=True, type=Path)
    parser.add_argument("--binary", type=Path, default=ROOT / "rust/target/release" / ("sub-store-rust.exe" if os.name == "nt" else "sub-store-rust"))
    parser.add_argument("--docker", action="store_true")
    args = parser.parse_args()
    if os.environ.get("GITHUB_ACTIONS") != "true":
        raise SystemExit("Build and execute service acceptance in GitHub Actions only")
    output = HERE / "evidence" / ("service-" + datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%S.%fZ"))
    output.mkdir(parents=True)
    environment = {key: value for key, value in os.environ.items() if not key.startswith("SUB_STORE_")}
    environment.update(SUB_STORE_BACKEND_API_HOST="127.0.0.1", SUB_STORE_BACKEND_PREFIX="true", SUB_STORE_FRONTEND_BACKEND_PATH="/backend")
    report = {"kind": "rust-service-docker" if args.docker else "rust-service-native", "exit_code": 1,
              "full_coverage": False, "production_ready": False, "os": platform.system(), "machine": platform.machine(), "phases": []}
    image = None
    try:
        report.update(source_provenance(environment, output))
        report["sources"] = {str(path.relative_to(ROOT)): hashlib.sha256(path.read_bytes()).hexdigest()
                             for path in [ROOT / "rust/Cargo.toml", ROOT / "rust/Cargo.lock", *sorted((ROOT / "rust/src").glob("*.rs"))]}
        version = execute([args.node, "--version"], ROOT, environment, output, "node-version")
        assert version.returncode == 0 and version.stdout.strip() == "v24.16.0", "Pinned Node required"
        bundled = execute([args.node, "bundle-esbuild.js"], BACKEND, environment, output, "reference-bundle", timeout=120)
        assert bundled.returncode == 0, "Could not build the Node reference"
        if args.docker:
            image = f"sub-store-m2:{os.environ['GITHUB_RUN_ID']}"
            build = execute(["docker", "build", "-f", "rust/Dockerfile", "-t", image, "."], ROOT, environment, output, "docker-build", timeout=1800)
            assert build.returncode == 0, "Rust service container build failed"
            architecture = execute(["docker", "image", "inspect", image, "--format", "{{.Architecture}}"], ROOT, environment, output, "docker-architecture")
            expected = {"x86_64": "amd64", "aarch64": "arm64"}[platform.machine()]
            assert architecture.returncode == 0 and architecture.stdout.strip() == expected, "Container did not build natively"
            report["docker_architecture"] = expected
        else:
            args.binary = args.binary.resolve(strict=True)
            report["binary_sha256"] = hashlib.sha256(args.binary.read_bytes()).hexdigest()
        _, cases = load_cases(HERE / "manifest.json", HERE / "fixtures/http-cases.json", "service-state")
        _, restart_cases = load_cases(HERE / "manifest.json", HERE / "fixtures/http-cases.json", "restart")
        final = {}
        for kind in ("node", "rust"):
            state = output / f"{kind}-state"
            state.mkdir()
            env = {**environment, "SUB_STORE_DATA_BASE_PATH": str(state)}
            manager = oracle(args.node, env, output, report, "node") if kind == "node" else listener(args.binary, env, output, report, image=image)
            with manager as url:
                results = check_cases(url, state, cases, output, f"{kind}-contracts")
                report["phases"].append({"name": f"{kind}-contracts", "cases": len(results), "requests": sum(len(case["steps"]) for case in results)})
                final[kind] = json.loads((state / "sub-store.json").read_text(encoding="utf-8"))
            manager = oracle(args.node, env, output, report, "node-restart") if kind == "node" else listener(args.binary, env, output, report, "rust-restart", image)
            with manager as url:
                check_cases(url, state, restart_cases, output, f"{kind}-restart")
                report["phases"].append({"name": f"{kind}-restart", "cases": len(restart_cases)})
                if kind == "rust":
                    previous = request(url, {"path": "/api/settings"})
                    saved = state / "saved-state.json"
                    (state / "sub-store.json").rename(saved)
                    (state / "sub-store.json").mkdir()
                    try:
                        failed = request(url, {"path": "/api/settings", "method": "PATCH", "body": {"mustNotCommit": True}})
                        unchanged = request(url, {"path": "/api/settings"})
                        (output / "write-failure.json").write_text(json.dumps({"before": previous, "failure": failed, "after": unchanged}, indent=2) + "\n", encoding="utf-8")
                        assert failed["status"] == 500 and json.loads(failed["body"])["error"]["code"] == "STATE_WRITE_FAILED"
                        check_json(json.loads(unchanged["body"]), json.loads(previous["body"]))
                    finally:
                        (state / "sub-store.json").rmdir()
                        saved.rename(state / "sub-store.json")
                    report["write_failure_preserved_state"] = True
                    denied = request(url, {"path": "/api/settings", "method": "PATCH", "headers": {"Origin": "https://denied.invalid"}, "body": {"mustNotCommit": True}})
                    assert denied["status"] == 403 and "access-control-allow-origin" not in denied["headers"]
                    assert request(url, {"path": "/api/storage", "origin": True})["status"] == 404
                    assert request(url, {"path": "/api/settings", "method": "OPTIONS", "headers": {"Origin": "https://sub-store.vercel.app"}})["status"] == 200
                    report["cors_and_prefix_checked"] = True
        check_json(final["rust"], final["node"])
        report["full_document_equal_after_shared_suite"] = True
        legacy = {"settings": {}, "subs": {"z-last": {"name": "z-last", "display-name": "旧数据🙂", "process": [{"type": "Useless Filter"}, {"type": "Set Property Operator", "args": {"key": "udp", "value": False}}, {"type": "Flag Operator", "args": False}, {"type": "Script Operator", "args": {"content": "require('node:fs')\r\n"}}]}, "a-first": {"name": "a-first"}}, "collections": {"c": {"name": "c", "ua": "old", "subscriptions": ["z-last"], "extra": False}}, "unknown": {"nil": None, "zero": 0, "flag": False}}
        for key in ("10", "2", "01", "0", "4294967295", "4294967294"):
            legacy["subs"][key] = {"name": "index-" + key}
        original = json.dumps(legacy, ensure_ascii=False).encode("utf-8")
        root = {"opaque": {"text": "cache🙂", "zero": 0, "flag": False}, "sub-store-cached-resource": "{}"}
        migrated = {}
        for kind in ("node", "rust"):
            state = output / f"{kind}-migration"
            state.mkdir()
            (state / "sub-store.json").write_bytes(base64.b64encode(original))
            (state / "root.json").write_text(json.dumps(root, ensure_ascii=False), encoding="utf-8")
            env = {**environment, "SUB_STORE_DATA_BASE_PATH": str(state)}
            manager = oracle(args.node, env, output, report, "node-migrate") if kind == "node" else listener(args.binary, env, output, report, "rust-migrate", image)
            with manager as url:
                response = request(url, {"path": "/api/storage"})
                assert response["status"] == 200
                migrated[kind] = json.loads(response["body"])
                (output / f"{kind}-migration.json").write_text(json.dumps(migrated[kind], ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
                if kind == "rust":
                    assert (state / "sub-store.pre-rust.json").read_bytes() == base64.b64encode(original)
                    check_json(json.loads((state / "root.json").read_text(encoding="utf-8")), root)
            for key in ("subs", "collections", "artifacts", "rules", "files", "tokens", "schemaVersion", "unknown"):
                if kind == "rust":
                    check_json(migrated[kind][key], migrated["node"][key])
        report["legacy_base64_migration_and_snapshots_checked"] = True
        report["legacy_numeric_key_order_checked"] = True
        generated = execute([args.node, HERE / "fixtures/service-recipient-keys.mjs"], BACKEND, environment, output, "recipient-oracle", timeout=30)
        assert generated.returncode == 0, "Could not generate pinned recipient fixtures"
        keys = json.loads(generated.stdout)
        report["recipient_fixture_sha256"] = hashlib.sha256((HERE / "fixtures/service-recipient-keys.mjs").read_bytes()).hexdigest()
        edges = {}
        for kind in ("node", "rust"):
            state = output / f"{kind}-edges"
            state.mkdir()
            env = {**environment, "SUB_STORE_DATA_BASE_PATH": str(state)}
            manager = oracle(args.node, env, output, report, "node-edges") if kind == "node" else listener(args.binary, env, output, report, "rust-edges", image)
            with manager as url:
                edges[kind] = state_edges(url, state, keys, output, f"{kind}-edges")
                report["phases"].append({"name": f"{kind}-edges", "requests": len(edges[kind])})
        check_json(edges["rust"], edges["node"])
        report["settings_and_recipient_http_state_equal"] = True
        rollback = output / "node-rollback"
        rollback.mkdir()
        shutil.copyfile(output / "rust-state/sub-store.json", rollback / "sub-store.json")
        with oracle(args.node, {**environment, "SUB_STORE_DATA_BASE_PATH": str(rollback)}, output, report, "node-rollback") as url:
            response = request(url, {"path": "/api/storage"})
            check_json(json.loads(response["body"]), final["rust"])
        report["node_reads_rust_written_state"] = True
        if not image:
            corrupt = output / "corrupt-state"
            corrupt.mkdir()
            (corrupt / "sub-store.json").write_bytes(b"invalid backup \xff")
            result = execute([args.binary], ROOT, {**environment, "SUB_STORE_DATA_BASE_PATH": str(corrupt)}, output, "corrupt-startup", timeout=10)
            assert result.returncode != 0 and (corrupt / "sub-store.json").read_bytes() == b"invalid backup \xff"
            report["corrupt_state_not_reset"] = True
        report["exit_code"] = 0
    except Exception as error:
        report["error"] = f"{type(error).__name__}: {error}"
    finally:
        (output / "summary.json").write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({"kind": report["kind"], "exit_code": report["exit_code"], "error": report.get("error"), "evidence": str(output)}))
    return report["exit_code"]


if __name__ == "__main__":
    if not __debug__:
        raise SystemExit("Service acceptance requires enabled assertions")
    raise SystemExit(main())
