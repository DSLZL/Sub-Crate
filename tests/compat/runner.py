"""Explicit-URL contract checks. A scoped Node baseline is never Rust parity."""
import argparse
import base64
import copy
from datetime import datetime, timezone
import hashlib
from http.client import HTTPConnection
import json
from pathlib import Path
import re
import sys
from urllib.error import HTTPError
from urllib.parse import urlsplit
from urllib.request import HTTPHandler, HTTPRedirectHandler, ProxyHandler, Request, build_opener
from upstream import ControlledUpstream, substitute

HERE = Path(__file__).resolve().parent
if not __debug__:
    raise SystemExit("Compatibility runner cannot run under Python -O")


class NoRedirect(HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        return None


class BufferedHTTPConnection(HTTPConnection):
    def request(self, *args, **kwargs):
        self.segments = []
        try:
            super().request(*args, **kwargs)
            # Early CORS rejection can close Windows TCP between separate header/body writes.
            super().send(b''.join(self.segments))
        finally:
            del self.segments

    def send(self, data):
        self.segments.append(data)


class BufferedHTTPHandler(HTTPHandler):
    def http_open(self, req):
        return self.do_open(BufferedHTTPConnection, req)


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
    if step.get("origin"):
        parsed = urlsplit(step["origin"] if isinstance(step["origin"], str) else base_url)
        base_url = f"{parsed.scheme}://{parsed.netloc}"
    req = Request(base_url.rstrip("/") + step["path"], data=data, headers=headers, method=step.get("method", "GET"))
    opener = build_opener(ProxyHandler({}), NoRedirect(), BufferedHTTPHandler())
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
    for key in expected.get("absent_headers", []):
        assert key.lower() not in response["headers"], f"header {key}: must be absent"
    for key, prefix in expected.get("header_prefixes", {}).items():
        assert response["headers"].get(key.lower(), "").startswith(prefix), f"header {key}: expected prefix {prefix!r}"
    if "json" in expected:
        check_json(json.loads(response["body"]), expected["json"])
    if "json_contains" in expected:
        check_json(json.loads(response["body"]), expected["json_contains"], subset=True)
    if "body" in expected:
        assert response["body"] == expected["body"], "raw response body mismatch"


def capture_values(response, specifications):
    body = json.loads(response["body"])
    values = {}
    for name, specification in specifications.items():
        assert re.fullmatch(r"[A-Z][A-Z0-9_]*", name), f"Invalid capture name: {name}"
        pointer = specification["pointer"]
        assert pointer.startswith("/"), f"Invalid JSON pointer: {pointer}"
        value = body
        for component in pointer[1:].split("/"):
            key = component.replace("~1", "/").replace("~0", "~")
            if isinstance(value, list):
                assert key.isdecimal(), f"Invalid array index: {key}"
                value = value[int(key)]
            else:
                value = value[key]
        kind = specification["type"]
        assert type(value) is {"string": str, "integer": int}[kind], f"{name}: expected {kind}"
        if kind == "integer":
            assert value > 0, f"{name}: expected positive integer"
        else:
            assert re.fullmatch(specification["pattern"], value), f"{name}: invalid captured string"
        values[name] = value
    return values


def interpolate(value, captures):
    if isinstance(value, str):
        exact = re.fullmatch(r"\{\{([A-Z][A-Z0-9_]*)\}\}", value)
        if exact:
            return captures[exact[1]]
        return re.sub(r"\{\{([A-Z][A-Z0-9_]*)\}\}", lambda match: str(captures[match[1]]), value)
    if isinstance(value, list):
        return [interpolate(item, captures) for item in value]
    if isinstance(value, dict):
        return {key: interpolate(item, captures) for key, item in value.items()}
    return value


def project_captured_scalars(response, expected, captures):
    """Only fixture-declared scalar paths may differ between the two live clocks."""
    targets = {}

    def declared(value, path=()):
        if isinstance(value, str) and re.search(r"\{\{[A-Z][A-Z0-9_]*\}\}", value):
            assert path not in targets or targets[path] == value, "Conflicting capture paths"
            targets[path] = value
        elif isinstance(value, dict):
            for key, item in value.items():
                declared(item, path + (key,))
        elif isinstance(value, list):
            for index, item in enumerate(value):
                declared(item, path + (index,))

    for key in ("json", "json_contains"):
        if key in expected:
            declared(expected[key])
    if not targets:
        return None
    body = response["body"]
    raw = body.encode("utf-8")
    assert response["headers"]["content-length"] == str(len(raw)), "Captured body length mismatch"
    digest = base64.b64encode(hashlib.sha1(raw).digest()).decode().rstrip("=")
    assert response["headers"]["etag"] == f'W/"{len(raw):x}-{digest}"', "Captured body ETag mismatch"
    decoder, replacements, visited = json.JSONDecoder(), [], set()

    def whitespace(offset):
        while offset < len(body) and body[offset] in " \r\n\t":
            offset += 1
        return offset

    def scan(offset, path):
        offset = whitespace(offset)
        if path in targets:
            value, end = decoder.raw_decode(body, offset)
            wanted = interpolate(targets[path], captures)
            assert not isinstance(value, (dict, list)), "Capture projection must be a scalar"
            assert type(value) is type(wanted) and value == wanted, "Capture projection value mismatch"
            replacements.append((offset, end, json.dumps(targets[path], ensure_ascii=False)))
            visited.add(path)
            return end
        if body[offset] not in "[{":
            return decoder.raw_decode(body, offset)[1]
        mapping = body[offset] == "{"
        end_char = "}" if mapping else "]"
        offset, index = whitespace(offset + 1), 0
        while body[offset] != end_char:
            if mapping:
                key, offset = decoder.raw_decode(body, offset)
                offset = whitespace(offset)
                assert body[offset] == ":", "Invalid JSON object"
                offset += 1
            else:
                key = index
            offset = whitespace(scan(offset, path + (key,)))
            index += 1
            if body[offset] == ",":
                offset = whitespace(offset + 1)
            else:
                assert body[offset] == end_char, "Invalid JSON container"
        return offset + 1

    assert whitespace(scan(0, ())) == len(body), "Trailing JSON content"
    assert visited == targets.keys(), "Missing capture projection path"
    for start, end, replacement in reversed(replacements):
        body = body[:start] + replacement + body[end:]
    return {"body": body, "paths": [{"path": list(path), "template": value} for path, value in targets.items()]}


def run_cases(base_url, state_dir, cases, upstream=None):
    results = []
    captures = {}
    for case in cases:
        record = {"id": case["id"], "source": case["source"], "status": "passed", "steps": []}
        try:
            if "upstream_gists" in case:
                if upstream is None:
                    raise ValueError("Gist fixture requires a controlled upstream")
                upstream.gists = copy.deepcopy(substitute(case["upstream_gists"], upstream.url))
            for original in case["steps"]:
                step = substitute(original, upstream.url) if upstream else original
                upstream_offset = len(upstream.requests) if upstream else 0
                sent = interpolate({key: step[key] for key in ("path", "method", "headers", "body", "origin") if key in step}, captures)
                response = request(base_url, sent)
                outgoing = upstream.requests[upstream_offset:] if upstream else []
                observed = {"method": sent.get("method", "GET"), "path": sent["path"], "origin": sent.get("origin", False), "response": response, "upstream_requests": outgoing}
                record["steps"].append(observed)
                if "state" in step:
                    if state_dir is None:
                        raise ValueError("Persisted state assertion requires --state-dir")
                    observed["state"] = json.loads((state_dir / "sub-store.json").read_text(encoding="utf-8"))
                if "upstream_state" in step:
                    if upstream is None:
                        raise ValueError("Provider state assertion requires a controlled upstream")
                    observed["upstream_state"] = copy.deepcopy(upstream.gists)
                if "base64_backup" in step:
                    if upstream is None:
                        raise ValueError("Backup assertion requires a controlled upstream")
                    backup = step["base64_backup"]
                    content = upstream.gists[backup["gist"]]["files"][backup["file"]]["content"]
                    observed["base64_backup"] = json.loads(base64.b64decode(content, validate=True))
                if "capture" in step:
                    captured = capture_values(response, step["capture"])
                    assert not captures.keys() & captured.keys(), "Capture names must be unique"
                    captures.update(captured)
                    observed["captured"] = captured
                projection = project_captured_scalars(response, step["expect"], captures)
                if projection:
                    observed["capture_projection"] = projection
                step = interpolate(step, captures)
                check_response(response, step["expect"])
                if "state" in step:
                    check_json(observed["state"], step["state"], subset=True)
                if "upstream_state" in step:
                    check_json(observed["upstream_state"], step["upstream_state"])
                if "base64_backup" in step:
                    check_json(observed["base64_backup"], step["base64_backup"]["json"])
                if step.get("gist_contracts"):
                    from gist_fixtures import check_gist_requests, check_gist_state
                    check_gist_requests(outgoing)
                    observed["gist_state"] = upstream.script_gist.snapshot()
                    check_gist_state(observed["gist_state"])
                else:
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
    parser.add_argument("--upstream-port", type=int, default=0, help="Loopback fixture port; match the backend's HTTP proxy for node-info")
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
        report.update(backend_base_commit=manifest["backend_commit"], frontend_commit=manifest["frontend_commit"],
                      fixture_sha256=hashlib.sha256(args.fixtures.read_bytes()).hexdigest(),
                      manifest_sha256=hashlib.sha256(args.manifest.read_bytes()).hexdigest())
        with ControlledUpstream(args.upstream_port) as upstream:
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
