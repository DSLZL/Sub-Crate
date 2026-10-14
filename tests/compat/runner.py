"""Explicit-URL contract checks. A scoped Node baseline is never Rust parity."""
import argparse
from datetime import datetime, timezone
import hashlib
import json
from pathlib import Path
import sys
from urllib.error import HTTPError
from urllib.parse import urlsplit
from urllib.request import HTTPRedirectHandler, ProxyHandler, Request, build_opener
from upstream import ControlledUpstream, substitute

HERE = Path(__file__).resolve().parent
if not __debug__:
    raise SystemExit("Compatibility runner cannot run under Python -O")


class NoRedirect(HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        return None


def check_json(actual, expected, subset=False, path="$"):
    if isinstance(expected, dict):
        assert isinstance(actual, dict), f"{path}: expected object, got {actual!r}"
        if not subset:
            assert actual.keys() == expected.keys(), f"{path}: keys {list(actual)} != {list(expected)}"
        for key, value in expected.items():
            assert key in actual, f"{path}.{key}: missing required field"
            check_json(actual[key], value, subset, f"{path}.{key}")
    elif isinstance(expected, list):
        assert isinstance(actual, list) and len(actual) == len(expected), f"{path}: array length/type mismatch"
        for index, value in enumerate(expected):
            check_json(actual[index], value, subset, f"{path}[{index}]")
    else:
        # JSON booleans must not compare equal to 0/1 through Python coercion.
        numeric = type(actual) in (int, float) and type(expected) in (int, float)
        assert (numeric or type(actual) is type(expected)) and actual == expected, f"{path}: {actual!r} != {expected!r}"


def load_cases(manifest_path, fixtures_path, suite):
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    fixtures = json.loads(fixtures_path.read_text(encoding="utf-8"))
    required = manifest["suites"].get(suite)
    if not required or len(required) != len(set(required)):
        raise ValueError(f"Unknown, empty or duplicate required suite: {suite}")
    by_id = {}
    for case in fixtures:
        if case["id"] in by_id:
            raise ValueError(f"Duplicate fixture: {case['id']}")
        if not case.get("steps") or not case.get("source"):
            raise ValueError(f"Fixture has no steps/source: {case['id']}")
        by_id[case["id"]] = case
    missing = set(required) - by_id.keys()
    if missing:
        raise ValueError(f"Required scenarios have no executable fixture: {sorted(missing)}")
    return manifest, [by_id[case_id] for case_id in required]


def request(base_url, step):
    data = json.dumps(step["body"], ensure_ascii=False).encode("utf-8") if "body" in step else None
    headers = {"Content-Type": "application/json", **step.get("headers", {})}
    req = Request(base_url.rstrip("/") + step["path"], data=data, headers=headers, method=step.get("method", "GET"))
    opener = build_opener(ProxyHandler({}), NoRedirect())
    try:
        response = opener.open(req, timeout=5)
    except HTTPError as error:
        response = error
    with response:
        return {"status": response.code, "headers": dict((k.lower(), v) for k, v in response.headers.items()),
                "body": response.read().decode("utf-8")}


def check_response(response, expected):
    assert response["status"] == expected["status"], f"HTTP {response['status']} != {expected['status']}"
    for key, value in expected.get("headers", {}).items():
        assert response["headers"].get(key.lower()) == value, f"header {key}: {response['headers'].get(key.lower())!r} != {value!r}"
    for key, prefix in expected.get("header_prefixes", {}).items():
        assert response["headers"].get(key.lower(), "").startswith(prefix), f"header {key}: expected prefix {prefix!r}"
    if "json" in expected:
        check_json(json.loads(response["body"]), expected["json"])
    if "json_contains" in expected:
        check_json(json.loads(response["body"]), expected["json_contains"], subset=True)
    if "body" in expected:
        assert response["body"] == expected["body"], "raw response body mismatch"


def run_cases(base_url, state_dir, cases, upstream=None):
    results = []
    for case in cases:
        record = {"id": case["id"], "source": case["source"], "status": "passed", "steps": []}
        try:
            for original in case["steps"]:
                step = substitute(original, upstream.url) if upstream else original
                upstream_offset = len(upstream.requests) if upstream else 0
                response = request(base_url, step)
                observed = {"method": step.get("method", "GET"), "path": step["path"], "response": response}
                record["steps"].append(observed)
                check_response(response, step["expect"])
                if "state" in step:
                    if state_dir is None:
                        raise ValueError("Persisted state assertion requires --state-dir")
                    state = json.loads((state_dir / "sub-store.json").read_text(encoding="utf-8"))
                    check_json(state, step["state"], subset=True)
                outgoing = upstream.requests[upstream_offset:] if upstream else []
                observed["upstream_requests"] = outgoing
                check_json(outgoing, step.get("upstream", []), subset=True, path="upstream")
        except Exception as error:
            record.update(status="failed", error=f"{type(error).__name__}: {error}")
        results.append(record)
        # Cases form an ordered state transition; continuing would contaminate evidence.
        if record["status"] == "failed":
            break
    for case in cases[len(results):]:
        results.append({"id": case["id"], "status": "not_run", "reason": "Earlier scenario failed"})
    return results


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--url", required=True)
    parser.add_argument("--kind", choices=("node-baseline", "rust-contract"), required=True)
    parser.add_argument("--suite", default="smoke")
    parser.add_argument("--state-dir", type=Path)
    parser.add_argument("--manifest", type=Path, default=HERE / "manifest.json")
    parser.add_argument("--fixtures", type=Path, default=HERE / "fixtures/http-cases.json")
    parser.add_argument("--evidence", type=Path, required=True)
    args = parser.parse_args(argv)
    report = {"timestamp": datetime.now(timezone.utc).isoformat(), "argv": sys.argv if argv is None else argv,
              "kind": args.kind, "suite": args.suite, "full_coverage": False, "rust_parity": False, "cases": [], "exit_code": 1}
    try:
        parsed = urlsplit(args.url)
        if parsed.scheme not in ("http", "https") or not parsed.hostname or parsed.username or parsed.password or parsed.query or parsed.fragment:
            raise ValueError("--url must be an explicit HTTP(S) backend base URL without credentials/query/fragment")
        manifest, cases = load_cases(args.manifest, args.fixtures, args.suite)
        report.update(backend_commit=manifest["backend_commit"], frontend_commit=manifest["frontend_commit"],
                      fixture_sha256=hashlib.sha256(args.fixtures.read_bytes()).hexdigest(),
                      manifest_sha256=hashlib.sha256(args.manifest.read_bytes()).hexdigest())
        with ControlledUpstream() as upstream:
            report["cases"] = run_cases(args.url, args.state_dir, cases, upstream)
        report["exit_code"] = 0 if all(case["status"] == "passed" for case in report["cases"]) else 1
    except Exception as error:
        report["error"] = f"{type(error).__name__}: {error}"
    args.evidence.parent.mkdir(parents=True, exist_ok=True)
    # Never overwrite a failed or interrupted run with newer evidence.
    with args.evidence.open("x", encoding="utf-8") as output:
        json.dump(report, output, indent=2, ensure_ascii=False)
        output.write("\n")
    print(json.dumps({"kind": args.kind, "suite": args.suite, "passed": sum(case["status"] == "passed" for case in report["cases"]),
                      "required": len(report["cases"]), "exit_code": report["exit_code"], "full_coverage": False,
                      "evidence": str(args.evidence)}, ensure_ascii=False))
    return report["exit_code"]


if __name__ == "__main__":
    raise SystemExit(main())
