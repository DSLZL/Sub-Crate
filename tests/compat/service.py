"""CI acceptance for the actual Rust HTTP listener, separate from the JS probe."""
import argparse
import base64
from concurrent.futures import ThreadPoolExecutor
from contextlib import contextmanager
from datetime import datetime, timedelta, timezone
from html import unescape as unescape_html
import gzip
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import shutil
import socket
import subprocess
import time
from urllib.parse import quote, unquote, urlsplit
import zlib

from reference import BACKEND, HERE, ROOT, execute, oracle, source_provenance
from runner import BufferedHTTPConnection, check_json, load_cases, request, run_cases
from upstream import ControlledUpstream


def wall_time():
    if os.name != "nt":
        return datetime.now(timezone.utc)
    # Some Windows Python builds sample a coarse clock; export bounds need the precise OS clock.
    import ctypes
    from ctypes.wintypes import FILETIME
    stamp = FILETIME()
    precise = ctypes.windll.kernel32.GetSystemTimePreciseAsFileTime
    precise.argtypes = (ctypes.POINTER(FILETIME),)
    precise.restype = None
    precise(ctypes.byref(stamp))
    ticks = (stamp.dwHighDateTime << 32) | stamp.dwLowDateTime
    return datetime(1601, 1, 1, tzinfo=timezone.utc) + timedelta(microseconds=ticks // 10)


@contextmanager
def listener(binary, environment, output, report, log_prefix="rust", image=None, host_network=False):
    port = int(environment.get("SUB_STORE_BACKEND_API_PORT", "0"))
    if not port:
        with socket.socket() as reserved:
            reserved.bind(("127.0.0.1", 0))
            port = reserved.getsockname()[1]
    environment = {**environment, "SUB_STORE_BACKEND_API_PORT": str(port)}
    origin = f"http://127.0.0.1:{port}"
    if environment.get("SUB_STORE_FRONTEND_PATH"):
        environment.setdefault("SUB_STORE_CORS_ALLOWED_ORIGINS", origin)
        report["fixture_cors_origin"] = environment["SUB_STORE_CORS_ALLOWED_ORIGINS"]
    prefixed = environment.get("SUB_STORE_BACKEND_MERGE") or environment.get("SUB_STORE_BACKEND_PREFIX")
    url = origin + (environment.get("SUB_STORE_FRONTEND_BACKEND_PATH", "") if prefixed else "").rstrip("/")
    name = f"m2-{os.getpid()}-{port}"
    if image:
        argv = ["docker", "run", "--rm", "--name", name, "--read-only", "--cap-drop=ALL",
                "--security-opt=no-new-privileges", "--user", f"{os.getuid()}:{os.getgid()}",
                "--mount", f"type=bind,source={environment['SUB_STORE_DATA_BASE_PATH']},target=/data"]
        # Conversion callbacks contact the controlled loopback upstream in CI.
        argv.extend(["--network=host"] if host_network else ["-p", f"127.0.0.1:{port}:3000"])
        if environment.get("SUB_STORE_FRONTEND_PATH"):
            argv.extend(["--mount", f"type=bind,source={environment['SUB_STORE_FRONTEND_PATH']},target=/frontend,readonly"])
            if not environment.get("SUB_STORE_BACKEND_MERGE") and not host_network:
                argv.extend(["-p", f"127.0.0.1:{environment['SUB_STORE_FRONTEND_PORT']}:3001"])
        for key, value in environment.items():
            if key.startswith("SUB_STORE_") or key in ("TZ", "NODE_ENV"):
                value = {"SUB_STORE_DATA_BASE_PATH": "/data", "SUB_STORE_BACKEND_API_PORT": str(port) if host_network else "3000",
                         "SUB_STORE_BACKEND_API_HOST": value if host_network else "0.0.0.0", "SUB_STORE_FRONTEND_PATH": "/frontend",
                         "SUB_STORE_FRONTEND_HOST": value if host_network else "0.0.0.0", "SUB_STORE_FRONTEND_PORT": value if host_network else "3001"}.get(key, value)
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
                    frontend_ready = not environment.get("SUB_STORE_FRONTEND_PATH") or environment.get("SUB_STORE_BACKEND_MERGE") or request(
                        f"http://127.0.0.1:{environment.get('SUB_STORE_FRONTEND_PORT', '3001')}", {"path": "/"})["status"] == 200
                    if frontend_ready and request(url, {"path": "/api/utils/env"})["status"] == 200:
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


def check_cases(url, state, cases, output, name, upstream=None):
    results = run_cases(url, state, cases, upstream)
    (output / f"{name}.json").write_text(json.dumps(results, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    assert len(results) == len(cases) and all(case["status"] == "passed" for case in results), f"{name}: {[(case['id'], case.get('error')) for case in results if case['status'] != 'passed']}"
    return results


def script_concurrency(url, state, output, name):
    """A pending script must observe and preserve a concurrent ordinary write."""
    step = {"method": "POST", "path": "/api/preview/sub", "body": {
        "name": "concurrent-script", "source": "local",
        "content": "ss://YWVzLTEyOC1nY206cGFzcw==@127.0.0.1:8388#concurrent",
        "process": [{"type": "Script Operator", "args": {"content": """
async function operator(proxies) {
  $substore.write({started: true}, 'm3-script-await');
  const deadline = Date.now() + 3000;
  while (!$substore.read('settings').m3Concurrent) {
    if (Date.now() > deadline) throw new Error('Concurrent write was blocked');
    await $substore.wait(10);
  }
  $substore.write({completed: true}, 'm3-script-await');
  return proxies;
}
"""}}]}}
    with ThreadPoolExecutor(max_workers=1) as pool:
        pending = pool.submit(request, url, step)
        deadline = time.monotonic() + 4
        while True:
            durable = json.loads((state / "sub-store.json").read_text(encoding="utf-8"))
            if durable.get("m3-script-await", {}).get("started"):
                break
            assert not pending.done() and time.monotonic() < deadline, "Script did not publish the await barrier"
            time.sleep(0.01)
        patch = request(url, {"method": "PATCH", "path": "/api/settings", "body": {"m3Concurrent": True}})
        response = pending.result(timeout=5)
    assert patch["status"] == response["status"] == 200, (patch, response)
    durable = json.loads((state / "sub-store.json").read_text(encoding="utf-8"))
    assert durable["settings"]["m3Concurrent"] is True and durable["m3-script-await"] == {"completed": True}
    observation = {"patch": {key: patch[key] for key in ("status", "body")},
                   "script": {key: response[key] for key in ("status", "body")}}
    (output / f"{name}-script-concurrency.json").write_text(json.dumps(observation, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    return observation


def state_edges(url, state, keys, output, name):
    """Compare actual HTTP values and persisted state without response normalization."""
    observations = []

    def send(method, path, body, status, html=False):
        response = request(url, {"method": method, "path": path, "body": body})
        observation = {"method": method, "path": path, "body": body, "response": response,
                       "state": json.loads((state / "sub-store.json").read_text(encoding="utf-8"))}
        observations.append(observation)
        with (output / f"{name}.jsonl").open("a", encoding="utf-8") as evidence:
            evidence.write(json.dumps(observation, ensure_ascii=False) + "\n")
        assert response["status"] == status, f"{name} {method} {path}: {response}"
        if html:
            assert response["headers"]["content-type"] == "text/html; charset=utf-8"
            assert response["headers"]["content-security-policy"] == "default-src 'none'"
            assert response["headers"]["x-content-type-options"] == "nosniff"
            observation["response"] = {"status": response["status"], "html": response["body"]}
            return response["body"]
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
    for field in numeric_fields:
        for value in ({"toString": None}, [{"toString": "not callable"}], [[{"toString": False}]]):
            before = {key: (state / key).read_bytes() for key in ("sub-store.json", "root.json")}
            error = send("PATCH", "/api/settings", {field: value}, 500)["error"]
            check_json(error, {"code": "FAILED_TO_UPDATE_SETTINGS", "type": "InternalServerError",
                               "message": "Failed to update settings", "details": "Reason: Cannot convert object to primitive value"})
            assert all((state / key).read_bytes() == content for key, content in before.items())
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
                send("PATCH", path, {"age-public-key": invalid}, 500, html=True)
                rejected = send("POST", f"/api/{plural}", {"name": "invalid", "age-public-key": invalid}, 500)
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


def crud_edges(url, state, output, name):
    """Subscription/collection CRUD with exact typed values and persisted state."""
    observations = []

    def send(method, path, body=..., status=200, html=None):
        step = {"method": method, "path": path}
        if body is not ...:
            step["body"] = body
        response = request(url, step)
        observation = {**step, "response": response,
                       "state": json.loads((state / "sub-store.json").read_text(encoding="utf-8"))}
        with (output / f"{name}.jsonl").open("a", encoding="utf-8") as evidence:
            evidence.write(json.dumps(observation, ensure_ascii=False) + "\n")
        assert response["status"] == status, f"{name} {method} {path}: {response}"
        content_type = response["headers"]["content-type"]
        if html is not None:
            assert content_type == "text/html; charset=utf-8"
            assert response["headers"]["content-security-policy"] == "default-src 'none'"
            assert response["headers"]["x-content-type-options"] == "nosniff"
            assert response["body"] == ('<!DOCTYPE html>\n<html lang="en">\n<head>\n<meta charset="utf-8">\n'
                                        '<title>Error</title>\n</head>\n<body>\n<pre>' + html + '</pre>\n</body>\n</html>\n')
            assert int(response["headers"]["content-length"]) == len(response["body"].encode("utf-8"))
            observation["response"] = {"status": status, "content_type": content_type, "body": response["body"]}
            observations.append(observation)
            return response["body"]
        expected_type = "application/json; charset=utf-8" if path == "/api/storage" and method == "GET" else "text/plain; charset=utf-8"
        assert content_type == expected_type, f"{name} {method} {path}: Content-Type {content_type!r} != {expected_type!r}"
        observation["response"] = {"status": response["status"], "content_type": content_type, "json": json.loads(response["body"])}
        observations.append(observation)
        return observation["response"]["json"]

    for old, renamed in (("1", "renamed🙂"), ("0", "zero-renamed")):
        patched = send("PATCH", f"/api/sub/{old}", {"name": renamed, "subscriptions": ["ignored"]})["data"]
        assert patched["subscriptions"] == ["reserved"], "PATCH removed the existing reserved subscription field"
        assert patched["unknown"] == {"zero": 0, "flag": False, "text": "保留🙂\r\n"}
    send("PATCH", "/api/collection/1", {"name": "collection-renamed"})
    document = send("GET", "/api/storage")["subs"]
    assert [item["name"] for item in document] == ["renamed🙂", "zero-renamed"]
    stored = json.loads((state / "sub-store.json").read_text(encoding="utf-8"))
    assert stored["collections"][0]["subscriptions"] == ["renamed🙂", "1", "zero-renamed", "0", 1, 0]
    expected = ["renamed🙂", "01", "renamed🙂", "renamed🙂", "zero-renamed", "zero-renamed", None,
                "renamed🙂", "renamed🙂", "zero-renamed", [], {}]
    check_json([item["source"] for item in stored["artifacts"] if item["type"] == "subscription"], expected)
    check_json([item["sourceName"] for item in stored["files"] if item["sourceType"] == "subscription"], expected)
    check_json([item["source"] for item in stored["artifacts"] if item["type"] == "collection"],
               ["collection-renamed", 1, True, [1]])
    before = (state / "sub-store.json").read_bytes()
    assert "subscriptions" not in send("GET", "/api/sub/" + quote("renamed🙂", safe=""))["data"]
    assert "subscriptions" not in send("GET", "/api/subs")["data"][0]
    assert (state / "sub-store.json").read_bytes() == before, "GET persisted a live-object cleanup prematurely"
    send("PATCH", "/api/sub/" + quote("renamed🙂", safe=""), {})
    assert "subscriptions" not in json.loads((state / "sub-store.json").read_text(encoding="utf-8"))["subs"][0]
    send("DELETE", "/api/sub/" + quote("renamed🙂", safe=""))
    send("DELETE", "/api/sub/zero-renamed?mode=permanent")
    send("DELETE", "/api/collection/collection-renamed?mode=")
    stored = json.loads((state / "sub-store.json").read_text(encoding="utf-8"))
    assert stored["collections"][0]["subscriptions"] == ["1", "0", 1, 0]
    assert stored["artifacts"][0]["source"] == "renamed🙂", "Permanent delete unexpectedly rewrote artifact references"

    for plural, singular in (("subs", "sub"), ("collections", "collection")):
        path = f"/api/{singular}/missing"
        before = (state / "sub-store.json").read_bytes()
        for mode in ("", "?mode=", "?mode=permanent", "?mode=invalid"):
            error = send("DELETE", path + mode, status=500)["error"]
            assert error["code"] == ("INVALID_DELETE_MODE" if mode.endswith("invalid") else "RESOURCE_NOT_FOUND")
        send("PATCH", path, {}, 404)
        if plural == "subs":
            send("GET", path, status=500, html="Internal Server Error")
        else:
            error = send("GET", path, status=500)["error"]
            check_json(error, {"code": "SUBSCRIPTION_NOT_FOUND", "type": "ResourceNotFoundError",
                               "message": "Collection missing does not exist", "details": 404})
        assert (state / "sub-store.json").read_bytes() == before, "Missing-resource requests changed state"
        for item in ({"name": ""}, {"name": None}, {}, {"name": False}, {"name": 0}, {"name": 1.5}):
            send("POST", f"/api/{plural}", item, 201)
            before = (state / "sub-store.json").read_bytes()
            assert send("POST", f"/api/{plural}", item, 500)["error"]["code"] == "DUPLICATE_KEY"
            assert (state / "sub-store.json").read_bytes() == before
        for item in ({"name": ["alias"]}, {"name": {"alias": True}}):
            send("POST", f"/api/{plural}", item, 201)
            send("POST", f"/api/{plural}", item, 201)
        for value in (1.0, 1e-6, 1e-7, 1e20, 1e21):
            send("POST", f"/api/{plural}", {"name": value}, 201)
            assert send("POST", f"/api/{plural}", {"name": value}, 500)["error"]["code"] == "DUPLICATE_KEY"
        for field in ("name", "age-public-key"):
            before = {key: (state / key).read_bytes() for key in ("sub-store.json", "root.json")}
            error = send("POST", f"/api/{plural}", {field: {"toString": "not callable"}}, 500)["error"]
            check_json(error, {"message": "Cannot convert object to primitive value"})
            assert all((state / key).read_bytes() == content for key, content in before.items())
        before = (state / "sub-store.json").read_bytes()
        for value in ("bad/name", ["bad/name"]):
            assert send("POST", f"/api/{plural}", {"name": value}, 500)["error"]["code"] == "INVALID_NAME"
        assert (state / "sub-store.json").read_bytes() == before
        for position in ("top", "bottom"):
            send("PATCH", "/api/settings", {"appearanceSetting": {"createItemPosition": position}})
            send("POST", f"/api/{plural}", {"name": position, "subscriptions": ["keep-only-on-collections"]}, 201)
            items = send("GET", f"/api/{plural}")["data"]
            assert items[0 if position == "top" else -1]["name"] == position
        items = [{"name": "editable", "subscriptions": ["replace-keeps-this"], "unknown": {"nil": None},
                  "editorLanguage": "yaml", "previewLanguage": "bad",
                  "process": [None, False, {"type": "Script Operator", "args": {"content": "require('node:fs')\r\n🙂", "editorLanguage": "plaintext", "previewLanguage": "bad"}}]},
                 {"name": "duplicate", "subscriptions": []}, {"name": "duplicate", "subscriptions": []}]
        send("PUT", f"/api/{plural}", items)
        patch_path = f"/api/{singular}/editable"
        for value in (None, False, 0, ""):
            changed = send("PATCH", patch_path, {"name": value, "subscriptions": ["patch-input"], "opaque": value})["data"]
            assert changed["name"] == "editable" and changed["unknown"] == {"nil": None}
            assert changed["subscriptions"] == (["replace-keeps-this"] if plural == "subs" else ["patch-input"])
            assert changed["process"] == [None, False, {"type": "Script Operator", "args": {"content": "require('node:fs')\r\n🙂", "editorLanguage": "plaintext"}}]
        before = (state / "sub-store.json").read_bytes()
        assert send("DELETE", patch_path + "?mode=invalid", status=500)["error"]["code"] == "INVALID_DELETE_MODE"
        assert (state / "sub-store.json").read_bytes() == before
        send("PATCH", patch_path, {"name": "duplicate"})
        send("DELETE", f"/api/{singular}/duplicate?mode=permanent")
        remaining = send("GET", f"/api/{plural}")["data"]
        check_json(remaining, [{"name": "duplicate", "subscriptions": []}, {"name": "duplicate", "subscriptions": []}])

    for plural, singular in (("subs", "sub"), ("collections", "collection")):
        path = f"/api/{plural}"
        item_path = f"/api/{singular}/typed"
        before = (state / "sub-store.json").read_bytes()
        for value in (None, False, 0, "scalar🙂"):
            for method, target in (("POST", path), ("PUT", path), ("PATCH", item_path)):
                send(method, target, value, 400, html="Bad Request")
                assert (state / "sub-store.json").read_bytes() == before, "Rejected scalar JSON changed state"
        for value in ([], [None, False, {"opaque": "数组🙂\r\n"}]):
            send("PUT", path, [])
            created = send("POST", path, value, 201)["data"]
            check_json(created, dict((str(index), item) for index, item in enumerate(value)) if plural == "subs" else value)
            before = (state / "sub-store.json").read_bytes()
            assert send("POST", path, {}, 500)["error"]["code"] == "DUPLICATE_KEY"
            assert (state / "sub-store.json").read_bytes() == before, "Duplicate unnamed record changed state"
        target = {"name": "typed", "opaque": {"zero": 0, "flag": False}}
        send("PUT", path, [target])
        before = {key: (state / key).read_bytes() for key in ("sub-store.json", "root.json")}
        error = send("PUT", path, [target, {"age-public-key": {"toString": None}}], 500)["error"]
        check_json(error, {"message": "Cannot convert object to primitive value"})
        send("PATCH", item_path, {"age-public-key": {"toString": None}}, 500, html="Internal Server Error")
        assert all((state / key).read_bytes() == content for key, content in before.items())
        for value in ([], [None, False, {"opaque": "PATCH🙂\r\n"}]):
            changed = send("PATCH", item_path, value)["data"]
            target.update((str(index), item) for index, item in enumerate(value))
            check_json(changed, target)
        mixed = [False, 0, "text🙂\r\n", [], [None, {"name": "nested"}], target, None]
        send("PUT", path, mixed)
        check_json(send("GET", path)["data"], mixed)
        check_json(send("GET", item_path)["data"], target)
        check_json(send("PATCH", item_path, ["after-scalars"])["data"], {**target, "0": "after-scalars"})
        mixed = [None, *mixed[:-1]]
        send("PUT", path, mixed)
        before = (state / "sub-store.json").read_bytes()
        send("GET", item_path, status=500, html="Internal Server Error")
        send("PATCH", item_path, [], 500, html="Internal Server Error")
        for method, target_path, value in (("DELETE", item_path, ...), ("POST", path, {"name": "new"})):
            error = send(method, target_path, value, 500)["error"]
            check_json(error, {"message": "Cannot read properties of null (reading 'name')"})
        assert send("DELETE", item_path + "?mode=invalid", status=500)["error"]["code"] == "INVALID_DELETE_MODE"
        check_json(send("GET", path)["data"], mixed)
        assert (state / "sub-store.json").read_bytes() == before, "Null-record lookup failure changed state"
    original = json.loads((state / "sub-store.json").read_text(encoding="utf-8"))
    reference_cases = [
        ("null-record", None), ("false-record", False), ("number-record", 3),
        ("string-record", "record"), ("array-record", []), ("missing", {}),
        ("null-names", {"subscriptions": None}), ("false-names", {"subscriptions": False}),
        ("number-names", {"subscriptions": 3}), ("object-names", {"subscriptions": {}}),
        ("empty-string", {"subscriptions": ""}), ("matching-string", {"subscriptions": "1"}),
        ("other-string", {"subscriptions": "other"}),
        ("valid", {"subscriptions": ["1", 1, "1", False, None]}),
        ("stored-buffer", {"type": "Buffer", "data": [0, 65]}),
        ("stored-empty-buffer", {"type": "Buffer", "data": []}),
        ("live-buffer", b"\x00A"), ("live-empty-buffer", b""),
        ("artifact-coercion", {"subscriptions": ["1"]}),
        ("file-coercion", {"subscriptions": ["1"]}),
    ]
    for case_id, collection in reference_cases:
        for method in ("PATCH", "DELETE"):
            document = {**original, "subs": [{"name": "1", "opaque": {"keep": True}}],
                        "collections": [{"name": "first", "subscriptions": ["1", "1", 1]}, collection,
                                        {"name": "last", "subscriptions": ["1"]}]}
            if case_id.startswith("stored-"):
                document["collections"] = collection
            elif isinstance(collection, bytes):
                document["collections"] = []
            if case_id == "artifact-coercion":
                document["artifacts"] = [{"type": "subscription", "source": {"toString": None}}]
            elif case_id == "file-coercion":
                document["files"] = [{"sourceType": "subscription", "sourceName": {"toString": None}}]
            send("POST", "/api/storage", {"content": json.dumps(document, ensure_ascii=False)})
            if isinstance(collection, bytes):
                endpoint = urlsplit(url)
                connection = BufferedHTTPConnection(endpoint.hostname, endpoint.port, timeout=5)
                try:
                    connection.request("PUT", endpoint.path.rstrip("/") + "/api/collections",
                                       body=collection, headers={"Content-Type": "application/octet-stream"})
                    response = connection.getresponse()
                    content = response.read()
                    assert response.status == 200, f"Reference Buffer setup failed: {content!r}"
                finally:
                    connection.close()
            before = {key: (state / key).read_bytes() for key in ("sub-store.json", "root.json")}
            published_before = request(url, {"path": "/api/storage"})
            step = {"method": method, "path": "/api/sub/1?mode=permanent"}
            if method == "PATCH":
                step["body"] = {"name": "renamed"}
            response = request(url, step)
            after = {key: (state / key).read_bytes() for key in before}
            published_after = request(url, {"path": "/api/storage"})
            record = {"id": case_id, "request": step, "response": response,
                      "raw_setup_base64": base64.b64encode(collection).decode() if isinstance(collection, bytes) else None,
                      "published_before": published_before, "published_after": published_after,
                      "state": json.loads(after["sub-store.json"]), "root": json.loads(after["root.json"]),
                      "files_before_base64": {key: base64.b64encode(value).decode() for key, value in before.items()},
                      "files_after_base64": {key: base64.b64encode(value).decode() for key, value in after.items()}}
            with (output / f"{name}-reference-graphs.jsonl").open("a", encoding="utf-8") as evidence:
                evidence.write(json.dumps(record, ensure_ascii=False) + "\n")
            expected_status = 200 if case_id in ("valid", "live-empty-buffer") or (method == "PATCH" and case_id in ("empty-string", "matching-string", "other-string")) or (method == "DELETE" and case_id.endswith("-coercion")) else 500
            assert response["status"] == expected_status, f"{name} {case_id} {method}: {response}"
            assert after["root.json"] == before["root.json"], "Reference mutation changed root backup"
            if expected_status == 500:
                if name.startswith("rust"):
                    assert before == after, "Rejected reference mutation changed durable files"
                    check_json(json.loads(published_after["body"]), json.loads(published_before["body"]))
                elif method == "PATCH":
                    assert before == after, "Node rename unexpectedly persisted before its reference error"
                    if not case_id.startswith(("stored-", "live-")):
                        check_json(json.loads(published_after["body"])["collections"][0]["subscriptions"], ["renamed", "1", 1])
                else:
                    check_json(json.loads(after["sub-store.json"])["subs"], [])
            else:
                stored = json.loads(after["sub-store.json"])
                if case_id == "live-empty-buffer":
                    check_json(stored["collections"], {"type": "Buffer", "data": []})
                else:
                    check_json(stored["collections"][0]["subscriptions"], ["renamed", "1", 1] if method == "PATCH" else [1])
                check_json(stored["subs"], [{"name": "renamed", "opaque": {"keep": True}}] if method == "PATCH" else [])
            observations.append({"id": f"reference-{case_id}-{method}",
                                 "response": {key: response[key] for key in ("status", "body")},
                                 "headers": {key: response["headers"].get(key) for key in
                                             ("content-type", "content-length", "etag", "content-security-policy", "x-content-type-options")}})
    restored = original
    for key in ("subs", "collections"):
        restored[key] = [None, False, 0, "恢复🙂\r\n", [], [None, {"name": "nested"}],
                         {"0": False, "1": None, "name": "restored", "opaque": {"zero": 0}}]
    send("POST", "/api/storage", {"content": json.dumps(restored, ensure_ascii=False)})
    check_json(json.loads((state / "root.json").read_text(encoding="utf-8"))["sub-store"], json.dumps(restored, ensure_ascii=False, indent=2))
    check_json(send("GET", "/api/storage"), restored)
    return observations


def backup_encodings(document):
    raw = json.dumps(document, ensure_ascii=False).encode("utf-8")
    padded = raw + b" " * ((1 - len(raw)) % 3)
    encoded = base64.b64encode(padded).decode("ascii")
    urlsafe = base64.urlsafe_b64encode(padded).decode("ascii")
    variants = {"urlsafe-padded": urlsafe, "urlsafe-unpadded": urlsafe.rstrip("="),
                "separated-urlsafe": "\r\n!☃=".join(urlsafe[index:index + 17] for index in range(0, len(urlsafe), 17)),
                "interior-padding": "==".join(encoded[index:index + 11] for index in range(0, len(encoded), 11)),
                "orphan-sextet": base64.b64encode(raw + b" " * (-len(raw) % 3)).decode("ascii") + "Z"}
    alphabet = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/"
    for remainder, unused in ((1, 15), (2, 3)):
        payload = raw + b" " * ((remainder - len(raw)) % 3)
        canonical = base64.b64encode(payload).decode("ascii").rstrip("=")
        last = alphabet[alphabet.index(canonical[-1]) | unused]
        assert last != canonical[-1], "Trailing-bit fixture is canonical"
        variants[f"trailing-bits-{remainder}"] = canonical[:-1] + last
    return variants


def backup_edges(url, state, output, name, offset=None):
    observations = []
    local_zone = datetime.now().astimezone().tzinfo if offset is None else timezone(timedelta(minutes=offset))

    def send(method, path, body=None, status=200, filename=None):
        previous = {key: (state / key).read_bytes() for key in ("sub-store.json", "root.json")} if status >= 400 else None
        before = wall_time()
        response = request(url, {"method": method, "path": path, **({"body": body} if body is not None else {})})
        after = wall_time()
        stored = json.loads((state / "sub-store.json").read_text(encoding="utf-8"))
        root = json.loads((state / "root.json").read_text(encoding="utf-8"))
        record = {"method": method, "path": path, "body": body, "response": response,
                  "before": before.isoformat(), "after": after.isoformat(), "state": stored, "root": root}
        if previous is not None:
            record["files_before_base64"] = {key: base64.b64encode(value).decode("ascii") for key, value in previous.items()}
            record["files_after_base64"] = {key: base64.b64encode((state / key).read_bytes()).decode("ascii") for key in previous}
        with (output / f"{name}.jsonl").open("a", encoding="utf-8") as evidence:
            evidence.write(json.dumps(record, ensure_ascii=False) + "\n")
        assert response["status"] == status, f"{name} {path}: {response}"
        parsed = json.loads(response["body"])
        assert response["headers"]["content-type"] == ("application/json; charset=utf-8" if filename is not None else "text/plain; charset=utf-8")
        assert root["opaque"] == {"text": "根缓存🙂\r\n", "zero": 0, "flag": False, "nil": None}
        if filename is not None:
            assert response["headers"]["access-control-expose-headers"] == "content-disposition"
            disposition = response["headers"]["content-disposition"]
            match = re.fullmatch(r'attachment; filename="([^"\r\n]+)"', disposition)
            assert match, f"Invalid disposition: {disposition}"
            decoded = unquote(match[1])
            assert quote(decoded, safe="-_.!~*'()") == match[1], "Filename is not encodeURIComponent encoded"
            stamp = re.fullmatch(re.escape(filename) + r"_(\d{4}-\d{2}-\d{2}_\d{2}-\d{2}-\d{2})\.json", decoded)
            assert stamp, f"Invalid calendar filename: {decoded}"
            instant = datetime.strptime(stamp[1], "%Y-%m-%d_%H-%M-%S").replace(tzinfo=local_zone)
            assert before - timedelta(seconds=1) <= instant <= after, "Filename does not use server local time"
        observations.append({"method": method, "path": path, "status": status, "json": parsed, "state": stored,
                             "root": {key: root.get(key) for key in ("opaque", "sub-store", "sub-store-cached-resource",
                                                                     "sub-store-cached-headers-resource", "sub-store-cached-script-resource")}})
        return parsed

    resource = "导出 name!()'*🙂"
    for plural, singular in (("subs", "sub"), ("collections", "collection")):
        item = {"name": resource, "custom": {"zero": 0, "false": False, "nil": None}, "content": "保留🙂\r\nrequire('node:fs')"}
        created = send("POST", f"/api/{plural}", item, 201)["data"]
        exported = send("GET", f"/api/{singular}/{quote(resource, safe='')}?raw=1", filename=f"sub-store_{'subscription' if singular == 'sub' else 'collection'}_{resource}")
        check_json(exported, created)
    before = {name: (state / name).read_bytes() for name in ("sub-store.json", "root.json")}
    check_json(send("GET", "/api/storage", filename="sub-store_data"), json.loads(before["sub-store.json"]))
    assert all((state / name).read_bytes() == content for name, content in before.items()), "Export changed persisted files"
    for content in ("invalid backup", json.dumps({"unknown": "missing settings"}), json.dumps({"settings": None}),
                    "!W!==", "!W10=", backup_encodings({"settings": None})["separated-urlsafe"]):
        result = send("POST", "/api/storage", {"content": content}, 500)
        assert result["error"]["code"] == "INVALID_BACKUP_DATA"
        assert all((state / name).read_bytes() == value for name, value in before.items()), "Invalid import changed persisted files"
    backup = json.loads((HERE / "fixtures/frontend-backup.json").read_text(encoding="utf-8"))
    legacy = {**backup, "subs": {"10": {"name": "ten"}, "2": {"name": "two", "display-name": "旧名字🙂"}},
              "collections": {"c": {"name": "legacy-col", "subscriptions": ["two"], "ua": "old"}},
              "files": {}, "artifacts": {}, "tokens": {}}
    del legacy["schemaVersion"]
    array_settings = [None, False, 0, "设置🙂\r\n", [None, {"nested": True}],
                      {"script": "require('node:fs')\r\n", "opaque": {"zero": 0, "nil": None}}]
    array_backup = {**backup, "settings": array_settings}
    empty_legacy = {**backup, "settings": [], "subs": False, "collections": 0,
                    "artifacts": "", "rules": False, "files": None, "tokens": 0}
    del empty_legacy["schemaVersion"]
    for original in (backup, legacy, {**backup, "settings": []}, array_backup, empty_legacy):
        for encoded in (False, True):
            content = json.dumps(original, ensure_ascii=False)
            if encoded:
                content = base64.b64encode(content.encode("utf-8")).decode("ascii")
            send("POST", "/api/storage", {"content": content})
            root = json.loads((state / "root.json").read_text(encoding="utf-8"))
            check_json(json.loads(root["sub-store"]), original)
            exported = send("GET", "/api/storage", filename="sub-store_data")
            check_json(exported, json.loads((state / "sub-store.json").read_text(encoding="utf-8")))
            assert "opaque" not in exported and "sub-store" not in exported, "Export included root cache"
            if isinstance(original["settings"], list):
                check_json(send("GET", "/api/settings")["data"], original["settings"])
                expected = {str(index): value for index, value in enumerate(original["settings"])}
                expected.update({"defaultTimeout": "1500", "appearanceSetting": {"createItemPosition": "top"}})
                check_json(send("PATCH", "/api/settings", {"defaultTimeout": "1500", "appearanceSetting": {"createItemPosition": "top"}})["data"], expected)
                expected.update({"0": "覆盖🙂\r\n", "1": False})
                check_json(send("PATCH", "/api/settings", ["覆盖🙂\r\n", False])["data"], expected)
                check_json(json.loads((state / "sub-store.json").read_text(encoding="utf-8"))["settings"], expected)
            if original is empty_legacy:
                assert all(exported[key] == [] for key in ("subs", "collections", "artifacts", "rules", "files", "tokens"))
    scalar_settings = [True, False, 0, 1.25, "", "设置🙂\r\n"]
    for value, encoded in [(value, False) for value in scalar_settings] + [(scalar_settings[-1], True)]:
        original = {**backup, "settings": value}
        content = json.dumps(original, ensure_ascii=False)
        if encoded:
            content = base64.b64encode(content.encode("utf-8")).decode("ascii")
        send("POST", "/api/storage", {"content": content})
        check_json(json.loads(json.loads((state / "root.json").read_text(encoding="utf-8"))["sub-store"]), original)
        check_json(send("GET", "/api/storage", filename="sub-store_data")["settings"], value)
        previous = {key: (state / key).read_bytes() for key in ("sub-store.json", "root.json")}
        expected = value if value else {}
        check_json(send("GET", "/api/settings")["data"], expected)
        check_json(json.loads((state / "sub-store.json").read_text(encoding="utf-8"))["settings"], expected)
        assert (state / "root.json").read_bytes() == previous["root.json"], "Settings initialization changed root cache"
        if value:
            assert (state / "sub-store.json").read_bytes() == previous["sub-store.json"], "Truthy settings read rewrote data"
        if value is True or type(value) is float:
            patch = {"defaultTimeout": "1500", "opaque": {"nil": None, "flag": False, "text": "保留🙂\r\n"}}
            check_json(send("PATCH", "/api/settings", patch)["data"], patch)
    codec_backup = {**backup, "codecText": "ÿÿÿ🙂\r\n", "codecOpaque": {"nil": None, "flag": False, "zero": 0}}
    encodings = backup_encodings(codec_backup)
    assert any(char in encodings["urlsafe-padded"] for char in "-_"), "URL-safe fixture has no alternate symbols"
    lossy_backup = {**codec_backup, "codecText": "\ufffd\ufffd(\ufffd"}
    lossy_bytes = json.dumps({**codec_backup, "codecText": "INVALID_UTF8"}, ensure_ascii=False).encode("utf-8")
    encodings["lossy-utf8"] = base64.b64encode(lossy_bytes.replace(b"INVALID_UTF8", b"\xff\xc3(\xf0\x9f")).decode("ascii")
    for encoding, content in encodings.items():
        expected = lossy_backup if encoding == "lossy-utf8" else codec_backup
        send("POST", "/api/storage", {"content": content})
        check_json(json.loads(json.loads((state / "root.json").read_text(encoding="utf-8"))["sub-store"]), expected)
        check_json(send("GET", "/api/storage", filename="sub-store_data"), expected)
        check_json(send("GET", "/api/settings")["data"], expected["settings"])
    send("POST", "/api/storage", {"content": json.dumps(array_backup, ensure_ascii=False)})
    check_json(send("GET", "/api/settings")["data"], array_settings)
    prior_data = (state / "sub-store.json").read_bytes()
    send("GET", "/api/utils/refresh")
    root = json.loads((state / "root.json").read_text(encoding="utf-8"))
    assert all(root[key] == "{}" for key in ("sub-store-cached-resource", "sub-store-cached-headers-resource", "sub-store-cached-script-resource"))
    assert (state / "sub-store.json").read_bytes() == prior_data, "No-Gist refresh changed main state"
    (output / f"{name}.json").write_text(json.dumps(observations, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    return observations


def settings_reads(url, state, output, name, expected):
    observations = []
    for path in ("/api/storage", "/api/settings"):
        before = {key: (state / key).read_bytes() for key in ("sub-store.json", "root.json")}
        response = request(url, {"path": path})
        after = {key: (state / key).read_bytes() for key in before}
        stored = json.loads(after["sub-store.json"])
        record = {"path": path, "response": response, "state": stored,
                  "root": json.loads(after["root.json"]),
                  "files_before_base64": {key: base64.b64encode(value).decode("ascii") for key, value in before.items()},
                  "files_after_base64": {key: base64.b64encode(value).decode("ascii") for key, value in after.items()}}
        with (output / f"{name}.jsonl").open("a", encoding="utf-8") as evidence:
            evidence.write(json.dumps(record, ensure_ascii=False) + "\n")
        assert response["status"] == 200, f"{name} {path}: {response}"
        assert before == after, "Initialized settings read changed durable files"
        parsed = json.loads(response["body"])
        check_json(stored["settings"], expected)
        check_json(parsed if path == "/api/storage" else parsed["data"], stored if path == "/api/storage" else expected)
        assert response["headers"]["content-type"] == ("application/json; charset=utf-8" if path == "/api/storage" else "text/plain; charset=utf-8")
        observations.append({"path": path, "json": parsed, "state": stored, "root": record["root"]})
    return observations


def unicode_state(url, state, output, name, mutate=False):
    """Keep user code units intact across HTTP, both files and Node rollback."""
    parsed = urlsplit(url)
    observations = []

    def send(method, path, body=None, status=200, utf16=False):
        payload = json.dumps(body, ensure_ascii=not utf16) if body is not None else None
        encoded = payload.encode("utf-16le", errors="surrogatepass") if utf16 else payload.encode() if payload is not None else None
        before = {key: (state / key).read_bytes() for key in ("sub-store.json", "root.json")}
        connection = BufferedHTTPConnection(parsed.hostname, parsed.port, timeout=30)
        headers = {"Content-Type": "application/json; charset=" + ("utf-16le" if utf16 else "utf-8")} if encoded is not None else {}
        try:
            connection.request(method, parsed.path.rstrip("/") + path, body=encoded, headers=headers)
            response = connection.getresponse()
            content = response.read()
            fields = dict(response.getheaders())
            actual_status = response.status
        finally:
            connection.close()
        after = {key: (state / key).read_bytes() for key in before}
        record = {"method": method, "path": path, "status": actual_status, "headers": fields,
                  "request_base64": base64.b64encode(encoded).decode() if encoded is not None else None,
                  "body_base64": base64.b64encode(content).decode(), "state": json.loads(after["sub-store.json"]),
                  "root": json.loads(after["root.json"]),
                  "files_before_base64": {key: base64.b64encode(value).decode() for key, value in before.items()},
                  "files_after_base64": {key: base64.b64encode(value).decode() for key, value in after.items()}}
        with (output / f"{name}.jsonl").open("a", encoding="utf-8") as evidence:
            evidence.write(json.dumps(record, ensure_ascii=True) + "\n")
        assert actual_status == status, f"{name} {method} {path}: {actual_status} {content!r}"
        if method == "GET" or status >= 400:
            assert before == after, "Read/rejected Unicode request changed durable files"
        fields = {key.lower(): value for key, value in fields.items()}
        assert int(fields["content-length"]) == len(content)
        if "etag" in fields:
            digest = base64.b64encode(hashlib.sha1(content).digest()).decode().rstrip("=")
            assert fields["etag"] == f'W/"{len(content):x}-{digest}"'
        observations.append({key: record[key] for key in ("method", "path", "status", "body_base64", "state", "root")})
        return json.loads(content)

    original = json.loads((state / "sub-store.json").read_text(encoding="utf-8"))
    check_json(send("GET", "/api/storage"), original)
    check_json(send("GET", "/api/settings")["data"], original["settings"])
    if mutate:
        words = original["settings"].encode("utf-16le", errors="surrogatepass")
        expected = {str(index // 2): chr(int.from_bytes(words[index:index + 2], "little")) for index in range(0, len(words), 2)}
        check_json(send("PATCH", "/api/settings", {})["data"], expected)
        values = {"high": "\ud800", "low": "\udfff", "pair": "🙂", "literal": "\\ud800",
                  "marker": "\ue000d800", "markers": "\ue000\ue000", "\ud800": "\udc00", "nil": None}
        expected["unicode"] = values
        for utf16 in (False, True):
            check_json(send("PATCH", "/api/settings", {"unicode": values}, utf16=utf16)["data"], expected)
        item = {"name": "unicode-\ue000d800-🙂", "unicode": values}
        check_json(send("POST", "/api/subs", item, status=201)["data"], item)
        path = "/api/sub/" + quote(item["name"], safe="")
        check_json(send("GET", path)["data"], item)
        check_json(send("GET", path + "?raw=1"), item)
        send("POST", "/api/collections", {**item, "subscriptions": [item["name"]]}, status=201)
        restored = json.loads((state / "sub-store.json").read_text(encoding="utf-8"))
        restored["unknown"] = values
        for content in (json.dumps(restored, ensure_ascii=True),
                        base64.b64encode(json.dumps(restored, ensure_ascii=True).encode()).decode()):
            send("POST", "/api/storage", {"content": content})
            check_json(send("GET", "/api/storage"), restored)
            check_json(json.loads(json.loads((state / "root.json").read_text(encoding="utf-8"))["sub-store"]), restored)
        send("POST", "/api/storage", {"content": json.dumps({**restored, "settings": None}, ensure_ascii=True)}, status=500)
    for plural in ("subs", "collections"):
        check_json(send("GET", "/api/" + plural)["data"], json.loads((state / "sub-store.json").read_text(encoding="utf-8"))[plural])
    check_json(send("GET", "/api/settings")["data"], json.loads((state / "sub-store.json").read_text(encoding="utf-8"))["settings"])
    (output / f"{name}.json").write_text(json.dumps(observations, ensure_ascii=True, indent=2) + "\n", encoding="utf-8")
    return observations


def request_bodies(url, state, output, name, origin=None):
    """Exercise the production parser, durable CRUD, and the standalone forwarding path."""
    parsed = urlsplit(url)
    observations = []
    original = json.loads((state / "sub-store.json").read_text(encoding="utf-8"))
    compared_headers = ("content-type", "content-length", "etag", "content-security-policy", "x-content-type-options")

    def send(path="/api/settings", body=b"{}", media="application/json", status=200, method="PATCH", headers=None, parser_error=False, chunk_size=None, case_id=None):
        headers = {**({"Origin": origin} if origin else {}), **(headers or {})}
        if media is not None:
            headers["Content-Type"] = media
        if chunk_size is not None:
            headers["Transfer-Encoding"] = "chunked"
        chunks = [body[i:i + chunk_size] for i in range(0, len(body), chunk_size)] if chunk_size else None
        before = {filename: (state / filename).read_bytes() for filename in ("sub-store.json", "root.json")}
        connection = BufferedHTTPConnection(parsed.hostname, parsed.port, timeout=5)
        try:
            connection.request(method, parsed.path.rstrip("/") + path, body=chunks if chunks is not None else body,
                               headers=headers, encode_chunked=chunks is not None)
            response = connection.getresponse()
            content = response.read()
            raw = {"method": method, "path": path, "request_headers": headers,
                   "request_body_base64": base64.b64encode(body).decode("ascii"), "status": response.status,
                   "request_chunk_sizes": [len(chunk) for chunk in chunks] if chunks is not None else None,
                   "headers": {key.lower(): value for key, value in response.getheaders()},
                   "body_base64": base64.b64encode(content).decode("ascii"),
                   "state": json.loads((state / "sub-store.json").read_text(encoding="utf-8"))}
        finally:
            connection.close()
        if case_id is not None:
            raw["case_id"] = case_id
        if (method == "PUT" and status == 500) or case_id is not None:
            raw["files_before_base64"] = {filename: base64.b64encode(data).decode("ascii") for filename, data in before.items()}
            raw["files_after_base64"] = {filename: base64.b64encode((state / filename).read_bytes()).decode("ascii") for filename in before}
        with (output / f"{name}.jsonl").open("a", encoding="utf-8") as evidence:
            evidence.write(json.dumps(raw, ensure_ascii=False) + "\n")
        assert raw["status"] == status, f"{name} {method} {path}: expected {status}, got {raw['status']} {content!r}"
        actual = raw["headers"]
        if status in (400, 413, 415) or parser_error:
            message = {400: "Bad Request", 403: "Forbidden", 413: "Payload Too Large", 415: "Unsupported Media Type"}[status]
            assert content == (f'<!DOCTYPE html>\n<html lang="en">\n<head>\n<meta charset="utf-8">\n<title>Error</title>\n</head>\n<body>\n<pre>{message}</pre>\n</body>\n</html>\n').encode()
            assert actual["content-security-policy"] == "default-src 'none'" and actual["x-content-type-options"] == "nosniff"
            assert "etag" not in actual
        if status >= 400 or method == "GET" or method == "OPTIONS":
            assert all((state / filename).read_bytes() == data for filename, data in before.items()), "Parser/read/rejection mutated durable state"
        observations.append({"method": method, "path": path, "status": status,
                             "headers": {key: actual[key] for key in compared_headers if key in actual},
                             "body_base64": raw["body_base64"], "state": raw["state"]})
        return json.loads(content) if status < 400 and content else None

    for plural in ("subs", "collections"):
        send("/api/" + plural, b"[]", method="PUT")
        send("/api/" + plural, json.dumps({"name": "parser-" + plural}).encode(), status=201, method="POST")
        rejected = ((b"{}", "application/json"),
                    (b'{"forEach":0,"age-public-key":"invalid","0":{"name":"must-not-replace"}}', "application/json"),
                    (b'{"type":"Buffer","data":[65,0,255]}', "application/json"),
                    (b"name=must-not-replace&forEach[]=one&forEach[]=two", "application/x-www-form-urlencoded"),
                    (b"not-json", None), (b"not-json", "invalid"),
                    (b"", "application/json"), (b"", "application/x-www-form-urlencoded"))
        for body, media in rejected:
            send("/api/" + plural, body, media, status=500, method="PUT")
            actual = observations[-1]
            content = base64.b64decode(actual["body_base64"])
            check_json(json.loads(content),
                       {"status": "failed", "error": {"message": "r.forEach is not a function"}})
            assert actual["headers"]["content-type"] == "text/plain; charset=utf-8"
            assert actual["headers"]["content-length"] == str(len(content))
            tag = base64.b64encode(hashlib.sha1(content).digest()).decode("ascii").rstrip("=")
            assert actual["headers"]["etag"] == f'W/"{len(content):x}-{tag}"'
    for body, media in ((b"", "application/json"), (b"", "text/plain"),
                        (b"not-json", None), (b"not-json", "invalid"),
                        (b"not-json", "application/json; broken")):
        send(body=body, media=media)
    for charset, encoded in (("utf-8", b'\xef\xbb\xbf{"parserUnicode":"\xe4\xb8\xad"}'),
                             ("UTF-16LE", '{"parserUnicode":"UTF16🙂"}'.encode("utf-16le")),
                             ("utf-8", b'{"parserLossy":"\xff"}')):
        data = send(body=encoded, media=f'Application/JSON; charset="{charset}"')["data"]
        assert data.get("parserLossy") == "�" or data["parserUnicode"] in ("中", "UTF16🙂")
    for charset in ("utf-16be", "utf-16", "utf-7", "utf-7imap"):
        send(media=f"application/json; charset={charset}", status=403, parser_error=True)
        send(body=b"", media=f"application/json; charset={charset}")
    for encoded in (b"{", b"null", b"false", b"0", b'"text"', b" \t\r\n"):
        send(body=encoded, status=400)
    for media in ("application/json; charset=latin1", "application/json; charset=utf-99",
                  "application/json; charset=utf-32",
                  "application/x-www-form-urlencoded; charset=iso-8859-1"):
        send(media=media, status=415)
    send(headers={"Content-Encoding": "br"}, status=415)
    for path in ("/api/utils/env", "/api/missing-parser-route"):
        send(path, b"{", status=400, method="GET")
    if origin is None:
        send("/", b"{", status=400, method="GET")
    for media in ("application/json", "text/plain", "application/x-www-form-urlencoded"):
        send(body=b"x" * (102400 + 1), media=media, status=413, method="GET")
    send(body=json.dumps({"large": "x" * 32768}).encode(), status=413)
    for media in ("application/octet-stream", "application/x-www-form-urlencoded"):
        send(body=b"large=" + b"x" * 32768, media=media, method="GET")
    send(body=b"x=1&" * 1000 + b"y=2", media="application/x-www-form-urlencoded", status=413)
    send(body=b"deep" + b"[x]" * 33 + b"=1", media="application/x-www-form-urlencoded", status=400)
    send(body=b"{", status=403, headers={"Origin": "https://denied.invalid"})
    allowed = origin or "https://sub-store.vercel.app"
    send(body=b"{", method="OPTIONS", headers={"Origin": allowed})
    for media in ("text/plain", "application/octet-stream; charset=unrecognized", "application/problem+json"):
        data = send(body=b"A\x00\xff", media=media)["data"]
        assert [data[str(i)] for i in range(3)] == [65, 0, 255]
    data = send(body=b'["array",{"opaque":false}]')["data"]
    assert data["0"] == "array" and data["1"] == {"opaque": False}
    for plural, singular in (("subs", "sub"), ("collections", "collection")):
        data = send("/api/" + plural, b"A\x00\xff", "text/plain", 201, "POST")["data"]
        assert data == ({"0": 65, "1": 0, "2": 255} if plural == "subs" else {"type": "Buffer", "data": [65, 0, 255]})
        data = send(f"/api/{singular}/parser-{plural}", b"B\x00\xff", "text/plain")["data"]
        assert data["name"] == "parser-" + plural and data["0"] == 66
        body = b"name=form+%E4%B8%AD&subscriptions[]=one&subscriptions[]=two&custom[text]=a%2Bb%0D%0A"
        data = send("/api/" + plural, body, "application/x-www-form-urlencoded", 201, "POST")["data"]
        assert data["name"] == "form 中" and data["custom"] == {"text": "a+b\r\n"}
        assert ("subscriptions" not in data) if plural == "subs" else data["subscriptions"] == ["one", "two"]
    forms = (b"parserForm[a][b]=nested&parserRepeat=one&parserRepeat=two&parserEmpty&literal.dot=keep",
             b"parserSparse[2]=c&parserSparse[0]=a&parserObjects[0][a]=1&parserObjects[0][b]=2",
             b"parserConflict[x]=nested&parserConflict=scalar&parserNumeric[100]=object",
             b"parserMalformed=%E0%A4%A&parserPlus=a+b&parserComma=a,b&parserEquals[x=y]=z",
             b"__proto__[polluted]=yes&parserSafe[__proto__][polluted]=yes&constructor[prototype][opaque]=keep",
             b"parserOverflow[100]=one&parserOverflow=tail",
             b"parserOverflow=lead&parserOverflow[100]=one",
             b"parserOverflow[100]=one&parserOverflow[2]=two&parserOverflow=tail",
             b"parserOverflow[]=lead&parserOverflow[100]=one&parserOverflow[]=tail",
             b"parserOverflow[100][x]=one&parserOverflow=tail",
             b"parserOverflow[100]=one&parserOverflow[word]=value&parserOverflow=tail",
             b"parserOverflow[word]=one&parserOverflow[100]=two&parserOverflow=tail",
             b"parserOverflow=lead&parserOverflow[100][x]=one&parserOverflow[01]=two",
             b"parserOverflow%5B100%5D=one&parserOverflow%5B%5D=two",
             b"parserOverflow[18446744073709551615]=one&parserOverflow=tail",
             b"parserOverflow[100000000000000000000]=one&parserOverflow=tail",
             b"parserOverflow[1000000000000000000000]=one&parserOverflow=tail",
             b"parserConflict[x]=nested&parserConflict=&parserConflict[y]=last",
             b"parserConflict=first&parserConflict[x]=last&parserConflict=",
             b"%EE%80%80overflow=literal&parserMarker[%EE%80%80overflow]=keep")
    results = [send(body=body, media="application/x-www-form-urlencoded")["data"] for body in forms]
    assert results[0]["parserForm"] == {"a": {"b": "nested"}} and results[0]["parserRepeat"] == ["one", "two"]
    assert results[1]["parserSparse"] == ["a", "c"] and results[1]["parserObjects"] == [{"a": "1", "b": "2"}]
    assert results[3]["parserMalformed"] == "%E0%A4%A" and results[3]["parserPlus"] == "a b"
    assert "polluted" not in results[4] and "__proto__" not in results[4]
    assert results[-1]["\ue000overflow"] == "literal" and results[-1]["parserMarker"]["\ue000overflow"] == "keep"
    for coding in ("gzip", "deflate"):
        compress = (lambda data: gzip.compress(data, mtime=0)) if coding == "gzip" else zlib.compress
        headers = {"Content-Encoding": coding}
        data = send(body=compress(json.dumps({"parserCompressed": coding + "中🙂"}, ensure_ascii=False).encode()),
                    headers={"Content-Encoding": coding.upper()})["data"]
        assert data["parserCompressed"] == coding + "中🙂"
        data = send(body=compress(b"parserInflated[a][]=one&parserInflated[a][]=two"),
                    media="application/x-www-form-urlencoded", headers=headers)["data"]
        assert data["parserInflated"] == {"a": ["one", "two"]}
        data = send(body=compress(b"C\x00\xff"), media="application/octet-stream", headers=headers)["data"]
        assert [data[str(i)] for i in range(3)] == [67, 0, 255]
        for media in ("application/json", "text/plain", "application/x-www-form-urlencoded"):
            send(body=compress(b""), media=media, headers=headers)
            expanded_limit = 32768 if media == "application/json" else 102400
            exact = b'{"padding":"' + b"x" * (expanded_limit - 14) + b'"}' if media == "application/json" else b"x" * expanded_limit
            assert len(exact) == expanded_limit
            send(body=compress(exact), media=media, headers=headers, method="GET")
            send(body=compress(exact + b" "), media=media, headers=headers, status=413)
        for plain in (b"{", b"null"):
            send(body=compress(plain), headers=headers, status=400)
        send(body=compress('{"parserInflatedUtf16":"中🙂"}'.encode("utf-16le")),
             media="application/json; charset=utf-16le", headers=headers)
        send(body=compress(b"{}"), media="application/json; charset=utf-16be", headers=headers,
             status=403, parser_error=True)
        encoded = compress(b"{}")
        corrupted = bytearray(encoded)
        corrupted[-1] ^= 1
        for invalid in (b"", b"bad-header", encoded[:1], encoded[:-1], bytes(corrupted)):
            send(body=invalid, headers=headers, status=400)
        send(body=encoded, headers=headers, chunk_size=1)
        send(body=encoded[:-1], headers=headers, chunk_size=1, status=400)
        send(body=encoded, headers={**headers, "Origin": "https://denied.invalid"}, status=403)
        send(body=b"bad-header", headers={**headers, "Origin": allowed}, method="OPTIONS")
        send(body=b"bad-header", media=None, headers=headers)
    member = lambda data: gzip.compress(data, mtime=0)
    members = member(b'{"parserMembers":') + member(b'"all"}')
    assert send(body=members, headers={"Content-Encoding": "gzip"}, chunk_size=3)["data"]["parserMembers"] == "all"
    send(body=member(b"") + member(b"{}"), headers={"Content-Encoding": "gzip"})
    send(body=member(b"{}") + b"\x00" * 8, headers={"Content-Encoding": "gzip"})
    send(body=member(b"{}") + b"\x00ignored", headers={"Content-Encoding": "gzip"})
    send(body=member(b"{}") + b"garbage", headers={"Content-Encoding": "gzip"}, status=400)
    send(body=member(b"{}") + member(b"")[:-1], headers={"Content-Encoding": "gzip"}, status=400)
    send(body=zlib.compress(b"{}") + b"ignored", headers={"Content-Encoding": "deflate"})
    send(body=zlib.compress(b"{}")[2:-4], headers={"Content-Encoding": "deflate"}, status=400)
    for coding in ("br", "x-gzip", "gzip, deflate"):
        send(headers={"Content-Encoding": coding}, status=415)
    send(headers={"Content-Encoding": ""})
    json_media = ('Application/JSON ; charset = "UTF-8"',
                  'application/json; note="a;b\\"c\\\\d"; charset="utf\\-8"',
                  'application/json; note="café;ok"; charset="utf-8"',
                  'application/json; charset=latin1; CHARSET=utf-8',
                  'application/json; charset=latin1; charset=""',
                  'application/json; note=""; charset=utf-8',
                  'application/json+',
                  'application/json; extension*="a=b;c"; charset=utf-8')
    for index, media in enumerate(json_media):
        value = {"label": f"json-{index}", "text": "参数🙂\r\n", "nil": None, "flag": False}
        data = send(body=json.dumps({"parserMedia": value}, ensure_ascii=False).encode(), media=media,
                    case_id=f"media-json-{index}")["data"]
        check_json(data["parserMedia"], value)
    form_media = ('application/x-www-form-urlencoded; charset = "UTF-8"; note="a;b"',
                  'application/x-www-form-urlencoded; charset=""',
                  'application/x-www-form-urlencoded; charset=latin1; charset="utf\\-8"')
    for index, media in enumerate(form_media):
        value = f"form-{index}中🙂\r\n"
        data = send(body=("parserMediaForm=" + quote(value, safe="")).encode(), media=media,
                    case_id=f"media-form-{index}")["data"]
        assert data["parserMediaForm"] == value
    raw_media = ('application/problem+json; note="a;b"',
                 'text/plain; note="café"; charset=unrecognized',
                 "a" * 127 + "/" + "b" * 127,
                 'application/json!; note="a;b"',
                 'application/json+other')
    for index, media in enumerate(raw_media):
        body = bytes([index, 0, 255])
        data = send(body=body, media=media, case_id=f"media-raw-{index}")["data"]
        assert [data[str(i)] for i in range(3)] == list(body)
    invalid_media = ('application/json;', 'application/json; charset=',
                     'application/json; note="unterminated',
                     'application/json; \tcharset=utf-8', 'application/json; charset\t=utf-8',
                     'application/json; note="tab\tvalue"',
                     'application/json; charset="utf-8" garbage',
                     'application/json; note="escape\\é"', 'application/json; =missing',
                     'application/json; note=(x)', 'application/jso%n', 'app.lication/json',
                     'application/json+bad.suffix', 'application/json++',
                     "application/" + "a" * 128, "a" * 128 + "/json",
                     'application/json\t; charset=utf-8', 'application/json; charset=UTF8;')
    for index, media in enumerate(invalid_media):
        send(body=b"{", media=media, method="GET", headers={"Content-Encoding": "br"},
             case_id=f"media-invalid-{index}")
    aliases = ("utf--8", "utf-8_", "utf-8:1987", "utf-16-le", "utf-16-be", "utf-7-imap", "utf-7:1987")
    for index, charset in enumerate(aliases):
        media = f'application/json; charset="{charset}"'
        send(media=media, status=403, parser_error=True, case_id=f"charset-alias-{index}-nonempty")
        send(body=b"", media=media, case_id=f"charset-alias-{index}-empty")
    unsupported = ("utf8", "ucs-2", "unicode-1-1-utf-8", "utf-32le", "utf-99", "utf-8:198", "utf-8:19870")
    for index, charset in enumerate(unsupported):
        for label, body in (("nonempty", b"{}"), ("empty", b"")):
            send(body=body, media=f'application/json; charset="{charset}"', status=415,
                 case_id=f"charset-unsupported-{index}-{label}")
    for index, charset in enumerate(("utf8", "utf--8", "utf-8:1987")):
        send(media=f'application/x-www-form-urlencoded; charset="{charset}"', status=415,
             case_id=f"charset-form-{index}")
    media = 'application/json; charset="utf--8"'
    for label, body, status in (("nonempty", member(b"{}"), 403), ("empty", member(b""), 200),
                                ("invalid", b"bad-header", 400), ("limit", member(b"x" * 32769), 413)):
        send(body=body, media=media, headers={"Content-Encoding": "gzip"}, status=status,
             parser_error=status == 403, case_id=f"charset-gzip-{label}")
    for plural in ("subs", "collections"):
        current = json.loads((state / "sub-store.json").read_text(encoding="utf-8"))[plural]
        send("/api/" + plural, json.dumps(original[plural] + current, ensure_ascii=False).encode(), method="PUT")
    (output / f"{name}.json").write_text(json.dumps(observations, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    return observations


def bulk_buffers(url, state, output, name, phase, origin=None):
    """Keep live raw-byte lists distinct from their JSON after restore/restart."""
    parsed = urlsplit(url)
    observations = []

    def send(path, method="GET", body=None, status=200, error=None):
        encoded = body if isinstance(body, bytes) else json.dumps(body, ensure_ascii=False).encode() if body is not None else None
        headers = {"Content-Type": "application/octet-stream" if isinstance(body, bytes) else "application/json"}
        if origin:
            headers["Origin"] = origin
        before = {filename: (state / filename).read_bytes() for filename in ("sub-store.json", "root.json")}
        connection = BufferedHTTPConnection(parsed.hostname, parsed.port, timeout=5)
        try:
            connection.request(method, parsed.path.rstrip("/") + path, body=encoded, headers=headers)
            response = connection.getresponse()
            content = response.read()
            raw = {"method": method, "path": path, "phase": phase, "status": response.status,
                   "request_headers": headers, "request_body_base64": base64.b64encode(encoded or b"").decode(),
                   "headers": {key.lower(): value for key, value in response.getheaders()},
                   "body_base64": base64.b64encode(content).decode(),
                   "state": json.loads((state / "sub-store.json").read_text(encoding="utf-8")),
                   "root": json.loads((state / "root.json").read_text(encoding="utf-8")),
                   "files_before_base64": {key: base64.b64encode(value).decode() for key, value in before.items()},
                   "files_after_base64": {key: base64.b64encode((state / key).read_bytes()).decode() for key in before}}
        finally:
            connection.close()
        with (output / f"{name}.jsonl").open("a", encoding="utf-8") as evidence:
            evidence.write(json.dumps(raw, ensure_ascii=False) + "\n")
        assert raw["status"] == status, f"{name} {method} {path}: expected {status}, got {raw['status']} {content!r}"
        if method == "GET" or status >= 400:
            assert raw["files_before_base64"] == raw["files_after_base64"], "Buffer read/error changed files"
        if error == "html":
            assert content == b'<!DOCTYPE html>\n<html lang="en">\n<head>\n<meta charset="utf-8">\n<title>Error</title>\n</head>\n<body>\n<pre>Internal Server Error</pre>\n</body>\n</html>\n'
            assert raw["headers"]["content-security-policy"] == "default-src 'none'"
            assert raw["headers"]["x-content-type-options"] == "nosniff" and "etag" not in raw["headers"]
        else:
            if error:
                check_json(json.loads(content)["error"], error)
            tag = base64.b64encode(hashlib.sha1(content).digest()).decode().rstrip("=")
            assert raw["headers"]["etag"] == f'W/"{len(content):x}-{tag}"'
            assert raw["headers"]["content-length"] == str(len(content))
        observations.append({key: raw[key] for key in ("method", "path", "phase", "status", "body_base64", "state", "root")}
                             | {"headers": {key: value for key, value in raw["headers"].items()
                                             if key in ("content-type", "content-length", "etag", "content-security-policy", "x-content-type-options")}})
        return json.loads(content) if error != "html" else None

    if phase == "restore":
        original = json.loads((state / "sub-store.json").read_text(encoding="utf-8"))
        send("/api/storage", "POST", {"content": json.dumps(original, ensure_ascii=False)})
        check_json(json.loads(json.loads((state / "root.json").read_text(encoding="utf-8"))["sub-store"]), original)
    for plural, singular, label in (("subs", "sub", "Subscription"), ("collections", "collection", "Collection")):
        live = phase in ("live", "live-before-restart")
        if phase == "live-before-restart":
            send("/api/" + plural, "PUT", [{"name": "buffer-array"}])
            check_json(send(f"/api/{singular}/buffer-array")["data"], {"name": "buffer-array"})
        vectors = (b"A\x00\xff", b"", b"\x00A") if phase == "live" else (b"\x00A",) if live else (None,)
        for data in vectors:
            path = "/api/" + plural
            item_path = f"/api/{singular}/buffer-missing"
            if live:
                send(path, "PUT", data)
            expected = {"type": "Buffer", "data": list(data)} if live else json.loads((state / "sub-store.json").read_text(encoding="utf-8"))[plural]
            check_json(send(path)["data"], expected)
            missing_get = "html" if not live or plural == "subs" else {
                "code": "SUBSCRIPTION_NOT_FOUND", "type": "ResourceNotFoundError",
                "message": "Collection buffer-missing does not exist", "details": 404}
            send(item_path, status=500, error=missing_get)
            missing = {"code": "RESOURCE_NOT_FOUND", "type": "ResourceNotFoundError",
                       "message": f"{label} buffer-missing does not exist!"}
            send(item_path, "PATCH", {}, 404 if live else 500, missing if live else "html")
            send(item_path + "?mode=invalid", "DELETE", status=500,
                 error={"code": "INVALID_DELETE_MODE", "type": "RequestInvalidError", "message": "Unsupported delete mode: invalid"})
            send(item_path + "?mode=permanent", "DELETE", status=500,
                 error=missing if live else {"message": "e.find is not a function"})
            send(path, "POST", {"name": "buffer-new"}, 500,
                 {"message": "e.push is not a function" if live else "e.find is not a function"})
            duplicate = {"code": "DUPLICATE_KEY", "type": "RequestInvalidError",
                         "message": f"{label} undefined already exists."} if live and data and data[0] else {
                             "message": "e.push is not a function" if live else "e.find is not a function"}
            send(path, "POST", {}, 500, duplicate)
            send(path, "PUT", expected, 500, {"message": "r.forEach is not a function"})
            send(item_path, status=500, error=missing_get)
            if live:
                send("/api/settings", "PATCH", {"appearanceSetting": {"createItemPosition": "top"}})
                send(path, "POST", {"name": "buffer-new"}, 500, {"message": "e.unshift is not a function"})
                send("/api/settings", "PATCH", {"appearanceSetting": {"createItemPosition": "bottom"}})
                send(item_path, status=500, error=missing_get)
    exported = send("/api/storage")
    check_json(exported, json.loads((state / "sub-store.json").read_text(encoding="utf-8")))
    (output / f"{name}.json").write_text(json.dumps(observations, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    return observations


def api_entities(url, state, output, name, origin=None):
    """Pin send/end response boundaries on direct and forwarded business APIs."""
    parsed = urlsplit(url)
    observations = []
    compared_headers = ("content-type", "content-length", "etag", "last-modified", "content-encoding",
                        "cache-control", "vary", "content-security-policy", "x-content-type-options", "allow")

    def send(path, headers=None, method="GET", status=200, body=None, tagged=True, expected=None):
        headers = {**({"Origin": origin} if origin else {}), **(headers or {})}
        headers = {key: value for key, value in headers.items() if value is not None}
        if body is not None:
            headers.setdefault("Content-Type", "application/json")
        connection = BufferedHTTPConnection(parsed.hostname, parsed.port, timeout=5)
        before = wall_time().astimezone()
        try:
            connection.request(method, parsed.path.rstrip("/") + path, body=body, headers=headers)
            response = connection.getresponse()
            content = response.read()
            raw = {"method": method, "path": path, "request_headers": headers,
                   "request_body_base64": base64.b64encode(body or b"").decode("ascii"),
                   "status": response.status, "headers": {key.lower(): value for key, value in response.getheaders()},
                   "body_base64": base64.b64encode(content).decode("ascii"), "before": before.isoformat(),
                   "after": wall_time().astimezone().isoformat(),
                   "state": json.loads((state / "sub-store.json").read_text(encoding="utf-8"))}
        finally:
            connection.close()
        with (output / f"{name}.jsonl").open("a", encoding="utf-8") as evidence:
            evidence.write(json.dumps(raw, ensure_ascii=False) + "\n")
        assert raw["status"] == status, f"{name} {method} {path}: {raw}"
        actual = raw["headers"]
        if expected is not None:
            assert content == expected, f"{name} {method} {path}: body mismatch"
        if tagged:
            assert "etag" in actual, f"{name} {method} {path}: missing entity ETag"
            if method != "HEAD" and status != 304:
                digest = base64.b64encode(hashlib.sha1(content).digest()).decode("ascii")[:27]
                assert actual["etag"] == f'W/"{len(content):x}-{digest}"'
                assert actual["content-length"] == str(len(content))
        else:
            assert "etag" not in actual, "end/finalhandler response gained an entity tag"
        if status == 304:
            assert not content and all(key not in actual for key in ("content-type", "content-length", "transfer-encoding"))
        if "content-disposition" in actual:
            attachment = re.fullmatch(r'attachment; filename=".*_(\d{4}-\d{2}-\d{2}_\d{2}-\d{2}-\d{2})\.json"', actual["content-disposition"])
            assert attachment and actual["access-control-expose-headers"] == "content-disposition"
            instant = datetime.strptime(attachment[1], "%Y-%m-%d_%H-%M-%S").replace(tzinfo=before.tzinfo)
            assert before - timedelta(seconds=1) <= instant <= datetime.fromisoformat(raw["after"]), f"Export timestamp {instant.isoformat()} outside request bounds {before.isoformat()}..{raw['after']}"
        if headers.get("Origin") and status != 403:
            assert actual["access-control-allow-origin"] == headers["Origin"]
        observations.append({"method": method, "path": path, "status": status,
                             "headers": {key: actual[key] for key in compared_headers if key in actual},
                             "body_base64": raw["body_base64"], "state": raw["state"]})
        return actual, content

    payload = json.dumps({"name": "entity-保留🙂", "text": "Unicode🙂\r\n", "zero": 0, "nil": None}, ensure_ascii=False).encode("utf-8")
    send("/api/collections", method="POST", status=201, body=payload)
    item = "/api/collection/" + quote("entity-保留🙂", safe="")
    send("/api/settings", method="PATCH", body=b'{"entityValidator":"before"}')
    before_reads = {filename: (state / filename).read_bytes() for filename in ("sub-store.json", "root.json")}
    # Windows V8 interpolates a coarse wall-clock sample; keep Node off that tick.
    boundary_offset = 0.05 if os.name == "nt" and name.startswith("node-") else 0
    for _ in range(3):
        time.sleep(1 - time.time() % 1 + boundary_offset)
        send("/api/storage")
    for path in ("/api/settings", "/api/subs", "/api/collections", item, item + "?raw=1", "/api/storage"):
        normal, content = send(path)
        head, _ = send(path, method="HEAD", expected=b"")
        assert all(head.get(key) == normal.get(key) for key in compared_headers)
        for match in (normal["etag"], normal["etag"][2:], '"miss", ' + normal["etag"], "*"):
            cached, _ = send(path, {"If-None-Match": match}, status=304, expected=b"")
            assert cached["etag"] == normal["etag"]
        send(path, {"If-None-Match": normal["etag"]}, method="HEAD", status=304, expected=b"")
        for headers in ({"If-None-Match": '"miss"'},
                        {"If-None-Match": normal["etag"], "Cache-Control": "max-age=0, no-cache"},
                        {"If-None-Match": normal["etag"], "If-Modified-Since": "Fri, 01 Jan 2100 00:00:00 GMT"},
                        {"If-None-Match": normal["etag"], "If-Modified-Since": "invalid"},
                        {"If-Modified-Since": "Fri, 01 Jan 2100 00:00:00 GMT"},
                        {"If-None-Match": '"miss",\t' + normal["etag"]},
                        {"If-Match": '"miss"', "If-Unmodified-Since": "Fri, 01 Jan 1999 00:00:00 GMT", "Range": "bytes=0-1"}):
            fresh, _ = send(path, headers, expected=content)
            assert fresh["etag"] == normal["etag"] and "last-modified" not in fresh
    assert all((state / filename).read_bytes() == data for filename, data in before_reads.items()), "API revalidation changed durable state"
    previous, _ = send("/api/settings")
    changed, _ = send("/api/settings", {"If-None-Match": "*", "If-Match": '"miss"'},
                      method="PATCH", body=b'{"entityValidator":"after"}')
    assert changed["etag"] != previous["etag"]
    assert observations[-1]["state"]["settings"]["entityValidator"] == "after"
    send("/api/settings", {"If-None-Match": previous["etag"]})
    send("/api/settings", {"If-None-Match": changed["etag"]}, status=304, expected=b"")
    send("/api/collections", {"If-None-Match": "*"}, method="POST", body=payload, status=500)
    send(item + "?mode=permanent", {"If-None-Match": "*"}, method="DELETE")
    send(item, {"If-None-Match": "*"}, status=500)
    send(item, method="HEAD", status=500, expected=b"")
    send("/api/sub/missing-entity", {"If-None-Match": "*"}, status=500, tagged=False)
    send("/api/sub/missing-entity", method="HEAD", status=500, tagged=False, expected=b"")
    send("/api/settings", method="PATCH", body=b"{", status=400, tagged=False)
    send("/api/not-a-resource", {"If-None-Match": "*"}, status=404, tagged=False, expected=b"")
    send("/api/settings", {"Origin": "https://denied.invalid", "If-None-Match": "*"}, status=403,
         tagged=False, expected=b"CORS origin not allowed")
    allowed = origin or f"{parsed.scheme}://{parsed.netloc}"
    send("/api/settings", {"Origin": allowed, "If-None-Match": "*"}, method="OPTIONS", tagged=False, expected=b"")
    for resource in ("SETTINGS", "SuBs", "COLLECTIONS", "wholeFiles", "FILES", "MODULES", "TOKENS", "ARCHIVES"):
        send(f"/api/{resource}/")
    normal, content = send("/api/SeTTings/")
    head, _ = send("/api/SETTINGS/", method="HEAD", expected=b"")
    assert all(head.get(key) == normal.get(key) for key in compared_headers)
    send("/api/SeTTings/", {"If-None-Match": normal["etag"]}, status=304, expected=b"")
    for plural, singular in (("subs", "sub"), ("collections", "collection")):
        names = ["RouteCase🙂", "routecase🙂", "route/part", "route%2Fpart", ""]
        items = [{"name": value, "marker": index} for index, value in enumerate(names)]
        created = {"name": "route-created", "marker": "created"}
        if plural == "collections":
            for value in items + [created]:
                value["subscriptions"] = []
        send(f"/api/{plural.upper()}/", method="PUT", body=json.dumps(items, ensure_ascii=False).encode("utf-8"))
        send(f"/api/{plural.upper()}/", method="POST", status=201, body=json.dumps(created).encode("utf-8"))
        before_routes = {filename: (state / filename).read_bytes() for filename in ("sub-store.json", "root.json")}
        for item in items[:-1] + [created]:
            _, content = send(f"/api/{singular.upper()}/{quote(item['name'], safe='')}/")
            assert json.loads(content)["data"] == item
        _, content = send(f"/api/{singular}/route%2fpart")
        assert json.loads(content)["data"]["name"] == "route/part"
        for path in (f"/api/{singular}/", f"/api/{singular}/route/part"):
            send(path, status=404, tagged=False, expected=b"")
        send(f"/api/{singular}/route-created//", status=200 if origin else 404,
             tagged=bool(origin), expected=None if origin else b"")
        assert all((state / filename).read_bytes() == data for filename, data in before_routes.items())
        target = f"/api/{singular.upper()}/{quote(names[0], safe='')}/"
        send(target, method="PATCH", body=b'{"marker":"patched"}')
        _, content = send(f"/api/{singular}/{quote(names[1], safe='')}")
        assert json.loads(content)["data"]["marker"] == 1, "Case folding changed another record"
        send(target + "?mode=permanent", method="DELETE")
        _, content = send(f"/api/{plural.upper()}/")
        assert [item["name"] for item in json.loads(content)["data"]] == names[1:] + [created["name"]]
    before_routes = {filename: (state / filename).read_bytes() for filename in ("sub-store.json", "root.json")}
    for path in ("/api/%73ettings", "/api/utils/%65nv"):
        send(path, status=404, tagged=False, expected=b"")
    send("/api/settings//", status=200 if origin else 404, tagged=bool(origin), expected=None if origin else b"")
    for escape in ("%", "%0", "%GG", "%FF", "%E0%A4"):
        for method in ("GET", "PATCH"):
            headers, content = send(f"/api/sub/{escape}", method=method, status=400, tagged=False)
            assert headers["content-type"] == "text/html; charset=utf-8" and b"Bad Request" in content
            assert headers["content-security-policy"] == "default-src 'none'" and headers["x-content-type-options"] == "nosniff"
    assert all((state / filename).read_bytes() == data for filename, data in before_routes.items()), "Rejected route changed durable state"
    before_methods = {filename: (state / filename).read_bytes() for filename in ("sub-store.json", "root.json")}
    for method in ("POST", "PUT", "PATCH", "DELETE"):
        headers, content = send("/api/method-&'quote?ignored=method-query", method=method, status=404,
                                body=b'{"mustNotWrite":true}', tagged=False)
        assert headers["content-type"] == "text/html; charset=utf-8"
        assert f"Cannot {method} ".encode() in content and b"/api/method-&amp;&#39;quote</pre>" in content
        assert b"method-query" not in content
    for path, method in (("/api/settings", "POST"), ("/api/settings", "PUT"), ("/api/settings", "DELETE"),
                         ("/api/subs", "PATCH"), ("/api/collections", "PATCH"),
                         ("/api/storage", "PATCH"), ("/api/storage", "DELETE"), ("/api/not-a-route/extra", "POST")):
        headers, content = send(path, method=method, status=404, body=b'{"mustNotWrite":true}', tagged=False)
        assert headers["content-type"] == "text/html; charset=utf-8" and f"Cannot {method} ".encode() in content
    for path, methods in (("/api/SeTTings/", "GET,PATCH,HEAD"), ("/api/storage", "GET,POST,HEAD"),
                          ("/api/subs", "GET,POST,PUT,HEAD"), ("/api/collections", "GET,POST,PUT,HEAD"),
                          ("/api/sub/route-created", "GET,PATCH,DELETE,HEAD"),
                          ("/api/collection/route-created", "GET,PATCH,DELETE,HEAD"),
                          ("/api/utils/env", "GET,HEAD"), ("/api/utils/refresh", "GET,HEAD"),
                          ("/api/not-a-resource", "GET,HEAD"), ("/api/sub/route/part", "GET,HEAD")):
        headers, _ = send(path, {"Origin": None}, method="OPTIONS", expected=methods.encode())
        assert headers["allow"] == methods and headers["content-type"] == "text/plain; charset=utf-8"
    headers, _ = send("/api/settings", {"Origin": ""}, method="OPTIONS", expected=b"GET,PATCH,HEAD")
    assert headers["allow"] == "GET,PATCH,HEAD"
    for value, status, expected in ((None, 400, None), ("", 400, None), (allowed, 200, b""),
                                    ("https://denied.invalid", 403, b"CORS origin not allowed")):
        headers, content = send("/api/settings", {"Origin": value}, method="OPTIONS", body=b"{",
                                status=status, tagged=False, expected=expected)
        assert "allow" not in headers
        if status == 400:
            assert headers["content-type"] == "text/html; charset=utf-8" and b"Bad Request" in content
    send("/api/sub/%FF", {"Origin": None}, method="OPTIONS", status=400, tagged=False)
    for singular in ("sub", "collection"):
        _, content = send(f"/api/{singular}/missing-method-record", method="PATCH", body=b"{}", status=404)
        assert json.loads(content)["error"]["type"] == "ResourceNotFoundError"
    if origin is None:
        for method in ("POST", "PUT", "PATCH", "DELETE", "OPTIONS"):
            send("/", method=method, expected="Hello from sub-store, made with ❤️ by Peng-YM".encode())
        send("/", method="POST", body=b"{", status=400, tagged=False)
    assert all((state / filename).read_bytes() == data for filename, data in before_methods.items()), "HTTP method handling changed durable state"
    before_queries = {filename: (state / filename).read_bytes() for filename in ("sub-store.json", "root.json")}
    for singular in ("sub", "collection"):
        target = f"/api/{singular}/route-created"
        expected = {"name": "route-created", "marker": "created"}
        if singular == "collection":
            expected["subscriptions"] = []
        for query, exported in (("raw", False), ("raw=", False), ("raw=0", True), ("raw=false", True),
                                ("raw=&raw=", True), ("raw=1&raw=", True), ("raw[]", True),
                                ("raw[x]=", True), ("r%61w%5B%5D=", True), ("raw%3Dx=1", False),
                                ("raw[x][y][z][u][v][w]=", True), ("raw[__proto__]=", True)):
            headers, content = send(target + "?" + query)
            assert ("content-disposition" in headers) == exported
            value = json.loads(content)
            assert (value if exported else value["data"]) == expected
        for query, mode in (("mode=invalid", "invalid"), ("mode=0", "0"),
                            ("mode=archive&mode=permanent", "archive,permanent"),
                            ("mode=permanent&mode=", "permanent,"), ("mode=&mode=", ","),
                            ("mode[]=permanent", "permanent"), ("m%6Fde%5B%5D=permanent", "permanent"),
                            ("mode[0]=permanent&mode[2]=", "permanent,"),
                            ("mode[19]=permanent", "permanent"), ("mode[20]=permanent", "[object Object]"),
                            ("mode[x]=permanent", "[object Object]"),
                            ("mode[x]=archive&mode=permanent", "[object Object]"),
                            ("mode=lead&mode[20]=tail", "[object Object]"),
                            ("mode[20]=tail&mode=toString", "[object Object]"),
                            ("mode[x]=one&mode[20]=last&mode=toString", "[object Object]"),
                            ("mode[20]=tail&mode[x]=one&mode=more", "[object Object]"),
                            ("mode[x][y][z][u][v][w]=permanent", "[object Object]"),
                            ("mode[__proto__]=permanent", "[object Object]"),
                            ("mode=+permanent+", " permanent "), ("mode=%2570ermanent", "%70ermanent"),
                            ("mode=%FF%20permanent", "%FF%20permanent"),
                            ("&".join(["mode=permanent"] * 20), ",".join(["permanent"] * 20)),
                            ("&".join(["mode=permanent"] * 21), "[object Object]"),
                            ("mode=invalid&" + "x=&" * 999 + "mode=permanent", "invalid")):
            _, content = send(target + "?" + query, method="DELETE", status=500)
            assert json.loads(content) == {"status": "failed", "error": {"code": "INVALID_DELETE_MODE",
                "type": "RequestInvalidError", "message": "Unsupported delete mode: " + mode}}
        for query in ("mode[toString]=not-callable", "mode[x]=one&mode=toString", "mode[][toString]=not-callable"):
            _, content = send(target + "?" + query, method="DELETE", status=500)
            assert json.loads(content) == {"status": "failed", "error": {"message": "Cannot convert object to primitive value"}}
        for query, exported in (("x=&" * 999 + "raw=1", True), ("x=&" * 1000 + "raw=1", False)):
            headers, _ = send(target + "?" + query)
            assert ("content-disposition" in headers) == exported
    assert all((state / filename).read_bytes() == data for filename, data in before_queries.items()), "CRUD query flags changed durable state"
    for plural, singular in (("subs", "sub"), ("collections", "collection")):
        for query in ("", "?mode=", "?mode=%70ermanent"):
            send(f"/api/{plural}", method="POST", status=201, body=b'{"name":"query-delete"}')
            send(f"/api/{singular}/query-delete" + query, method="DELETE")
            assert all(item.get("name") != "query-delete" for item in observations[-1]["state"][plural])
    (output / f"{name}.json").write_text(json.dumps(observations, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    return observations


def static_fixture(frontend):
    files = {"index.html": b"<main>unchanged frontend entry</main>",
             "chunks/main-84da2e86.js": b'console.log("original chunk")',
             "chunks/multi-deadbeef.locale-extra.js": b"hashed suffix",
             "chunks/multi-deadbeef.-extra.js": b"hashed dash suffix",
             "css/main.css": b"body{}", "sw.js": b"worker", "empty.txt": b"",
             "release-deadbeef.dir/index.html": b"nested entry",
             "text 中文.txt": "Unicode中文🙂\r\n".encode("utf-8"),
             ".env": b"must never expose dotfiles"}
    for name, data in list(files.items()):
        if name in ("index.html", "chunks/main-84da2e86.js", "css/main.css"):
            files[name + ".gz"] = gzip.compress(data, mtime=0)
    files["chunks/only-deadbeef.mjs.gz"] = gzip.compress(b"compressed-only module", mtime=0)
    for name, data in files.items():
        path = frontend / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(data)
        os.utime(path, (1609459200, 1609459200))
    (frontend.parent / "secret.txt").write_bytes(b"must never expose parent files")


def static_conditions(send, frontend):
    """The same range/precondition contract runs through both hosting owners."""
    path = "/chunks/main-84da2e86.js"
    plain = (frontend / path.lstrip("/")).read_bytes()
    compressed = (frontend / (path.lstrip("/") + ".gz")).read_bytes()
    normal = send(path + "?conditional=plain", expected=plain)
    encoded = send(path + "?conditional=gzip", {"Accept-Encoding": "gzip"}, expected=compressed)
    errors = {status: (f'<!DOCTYPE html>\n<html lang="en">\n<head>\n<meta charset="utf-8">\n<title>Error</title>\n</head>\n<body>\n<pre>{message}</pre>\n</body>\n</html>\n').encode()
              for status, message in ((412, "Precondition Failed"), (416, "Range Not Satisfiable"))}

    def case(label, headers, status, expected, method="GET", content_range=None):
        result = send(path + "?conditional=" + label, headers, method, status, b"" if method == "HEAD" else expected)
        assert result.get("content-range") == content_range, f"{label}: incorrect Content-Range"
        if status != 304:
            assert result["content-length"] == str(len(expected)), f"{label}: incorrect representation length"
        if status >= 400:
            assert result["content-type"] == "text/html; charset=utf-8"
            assert result["content-security-policy"] == "default-src 'none'" and result["x-content-type-options"] == "nosniff"
            assert "content-encoding" not in result
        return result

    length = len(plain)
    for label, value, start, end in (
            ("first", "bytes=0-3", 0, 3), ("open", "bytes=4-", 4, length - 1),
            ("suffix", "bytes=-4", length - 4, length - 1),
            ("clamped", "bytes=2-99999999999999999999999", 2, length - 1),
            ("whole-suffix", f"bytes=-{length}", 0, length - 1),
            ("adjacent", "bytes=3-5,0-2", 0, 5), ("overlap", "bytes=2-5,0-3", 0, 5),
            ("duplicate", "bytes=0-3,0-3", 0, 3), ("one-valid", "bytes=99999-,1-4", 1, 4),
            ("numeric-prefix", "bytes=1garbage-4suffix", 1, 4),
            ("suffix-coercion", "bytes=invalid-4", length - 4, length - 1),
            ("spaces", "bytes= 1 - 4 ", 1, 4)):
        case(label, {"Range": value}, 206, plain[start:end + 1], content_range=f"bytes {start}-{end}/{length}")
    for label, value in (("disjoint", "bytes=0-1,4-5"), ("other-unit", "items=0-3"),
                         ("uppercase", "Bytes=0-3"), ("no-equals", "bytes"), ("space-before-equals", "bytes =0-3")):
        case(label, {"Range": value}, 200, plain)
    for label, value in (("past-end", f"bytes={length}-"), ("reversed", "bytes=5-2"),
                         ("zero-suffix", "bytes=-0"), ("large-suffix", f"bytes=-{length + 1}"),
                         ("empty-range", "bytes="), ("invalid-range", "bytes=garbage")):
        case(label, {"Range": value}, 416, errors[416], content_range=f"bytes */{length}")
    case("head-range", {"Range": "bytes=0-3"}, 206, plain[:4], "HEAD", f"bytes 0-3/{length}")
    case("head-unsatisfied", {"Range": "bytes=99999-"}, 416, errors[416], "HEAD", f"bytes */{length}")
    for label, value, fresh in (("weak", normal["etag"], True), ("strong", normal["etag"][2:], False),
                                ("missing", '"miss"', False), ("date", normal["last-modified"], True),
                                ("future", "Sat, 02 Jan 2021 00:00:00 GMT", True),
                                ("old", "Thu, 31 Dec 2020 23:59:59 GMT", False), ("invalid", "invalid", False)):
        case("if-range-" + label, {"Range": "bytes=0-3", "If-Range": value}, 206 if fresh else 200,
             plain[:4] if fresh else plain, content_range=f"bytes 0-3/{length}" if fresh else None)
    case("stale-unsatisfied", {"Range": "bytes=99999-", "If-Range": '"miss"'}, 200, plain)
    for label, value in (("weak", normal["etag"]), ("strong", normal["etag"][2:]),
                         ("list", '"miss", ' + normal["etag"]), ("wildcard", "*")):
        case("if-match-" + label, {"If-Match": value}, 200, plain)
    case("if-match-miss", {"If-Match": '"miss"'}, 412, errors[412])
    case("head-precondition", {"If-Match": '"miss"'}, 412, errors[412], "HEAD")
    for label, value in (("equal", normal["last-modified"]), ("future", "Sat, 02 Jan 2021 00:00:00 GMT"), ("invalid", "invalid")):
        case("unmodified-" + label, {"If-Unmodified-Since": value}, 200, plain)
    old = "Thu, 31 Dec 2020 23:59:59 GMT"
    case("unmodified-old", {"If-Unmodified-Since": old}, 412, errors[412])
    case("match-over-date", {"If-Match": normal["etag"], "If-Unmodified-Since": old}, 200, plain)
    case("precondition-before-fresh", {"If-Match": '"miss"', "If-None-Match": normal["etag"]}, 412, errors[412])
    case("precondition-before-range", {"If-Match": '"miss"', "Range": "bytes=0-3"}, 412, errors[412])
    case("fresh-before-range", {"If-None-Match": normal["etag"], "Range": "bytes=99999-"}, 304, b"")
    case("matched-range", {"If-Match": normal["etag"], "Range": "bytes=0-3"}, 206, plain[:4], content_range=f"bytes 0-3/{length}")
    gzip_headers = {"Accept-Encoding": "gzip", "Range": "bytes=0-3"}
    case("gzip-range", gzip_headers, 206, compressed[:4], content_range=f"bytes 0-3/{len(compressed)}")
    case("gzip-range-head", gzip_headers, 206, compressed[:4], "HEAD", f"bytes 0-3/{len(compressed)}")
    case("gzip-if-range", {**gzip_headers, "If-Range": encoded["etag"]}, 206, compressed[:4], content_range=f"bytes 0-3/{len(compressed)}")
    case("gzip-stale-if-range", {**gzip_headers, "If-Range": normal["etag"]}, 200, compressed)
    case("gzip-precondition", {**gzip_headers, "If-Match": normal["etag"]}, 412, errors[412])
    case("gzip-unsatisfied", {"Accept-Encoding": "gzip", "Range": "bytes=99999-"}, 416, errors[416], content_range=f"bytes */{len(compressed)}")
    send("/empty.txt?conditional=empty", expected=b"")
    empty = send("/empty.txt?conditional=range", {"Range": "bytes=0-"}, status=416, expected=errors[416])
    assert empty["content-range"] == "bytes */0"


def static_resources(url, state, output, name):
    """Compare actual binary representations and cache metadata from both services."""
    parsed = urlsplit(url)
    frontend = state / "frontend"
    observations = []
    before = {path.name: path.read_bytes() for path in (state / "sub-store.json", state / "root.json") if path.exists()}
    contract_headers = ("content-type", "content-length", "content-encoding", "cache-control",
                        "etag", "last-modified", "vary", "accept-ranges", "content-range",
                        "content-security-policy", "x-content-type-options")

    def send(path, headers=None, method="GET", status=200, expected=None):
        connection = BufferedHTTPConnection(parsed.hostname, parsed.port, timeout=5)
        try:
            connection.request(method, path, headers=headers or {})
            response = connection.getresponse()
            body = response.read()
            raw = {"method": method, "path": path, "request_headers": headers or {},
                   "status": response.status, "headers": {key.lower(): value for key, value in response.getheaders()},
                   "body_base64": base64.b64encode(body).decode("ascii")}
        finally:
            connection.close()
        with (output / f"{name}.jsonl").open("a", encoding="utf-8") as evidence:
            evidence.write(json.dumps(raw, ensure_ascii=False) + "\n")
        assert raw["status"] == status, f"{name} {method} {path}: {raw}"
        if expected is not None:
            assert body == expected, f"{name} {method} {path}: body mismatch"
        observations.append({"method": method, "path": path, "status": raw["status"],
                             "headers": {key: raw["headers"][key] for key in contract_headers if key in raw["headers"]},
                             "body_base64": raw["body_base64"]})
        return raw["headers"]

    path = "/chunks/main-84da2e86.js"
    plain = (frontend / path.lstrip("/")).read_bytes()
    compressed = (frontend / (path.lstrip("/") + ".gz")).read_bytes()
    normal = send(path + "?cache=1", expected=plain)
    encoded = send(path, {"Accept-Encoding": "gzip"}, expected=compressed)
    assert normal["content-type"] == "application/javascript; charset=utf-8"
    assert normal["content-length"] == str(len(plain)) and encoded["content-length"] == str(len(compressed))
    assert normal["cache-control"] == "public, max-age=31536000, immutable"
    assert encoded["content-encoding"] == "gzip" and "content-encoding" not in normal
    assert normal["last-modified"] == "Fri, 01 Jan 2021 00:00:00 GMT"
    assert re.fullmatch(r'W/"[0-9a-f]+-[0-9a-f]+"', encoded["etag"])
    send(path, {"Accept-Encoding": "gzip"}, "HEAD", expected=b"")
    for accept, use_gzip in (("gzip;q=0", False), ("gzip;q=0.5", True),
                             ("gzip;q=0.5, identity;q=1", False), ("identity;q=0.5, gzip;q=1", True),
                             ("*;q=0.8", True), ("*;q=0", False), ("gzip;q=0, *;q=1", False),
                             ("GZIP; q=0.8", True), ("br, gzip;q=.8", True),
                             ("identity, gzip", False), ("gzip, identity", True),
                             ("gzip;q=0.4, gzip;q=0.8, identity;q=0.6", True),
                             ("gzip;q=invalid", False), ("gzip;q=0.8suffix", True)):
        send(path, {"Accept-Encoding": accept}, expected=compressed if use_gzip else plain)
    for match in (encoded["etag"], encoded["etag"][2:], '"miss", ' + encoded["etag"], "*"):
        result = send(path, {"Accept-Encoding": "gzip", "If-None-Match": match}, status=304, expected=b"")
        assert all(key not in result for key in ("content-type", "content-length", "content-encoding"))
    send(path, {"Accept-Encoding": "gzip", "If-None-Match": encoded["etag"]}, "HEAD", 304, b"")
    send(path, {"If-Modified-Since": normal["last-modified"]}, status=304, expected=b"")
    send(path, {"If-Modified-Since": "invalid"}, expected=plain)
    send(path, {"If-Modified-Since": "Thu, 31 Dec 2020 23:59:59 GMT"}, expected=plain)
    send(path, {"If-None-Match": normal["etag"], "Cache-Control": "max-age=0, no-cache"}, expected=plain)
    send(path, {"If-None-Match": normal["etag"], "If-Modified-Since": "invalid"}, expected=plain)
    send(path, {"Accept-Encoding": "gzip", "If-None-Match": normal["etag"]}, expected=compressed)
    send("/sw.js", {"Accept-Encoding": "gzip"}, expected=b"worker")
    send("/css/main.css?cache=1", {"Accept-Encoding": "gzip"}, expected=(frontend / "css/main.css.gz").read_bytes())
    send("/chunks/only-deadbeef.mjs", {"Accept-Encoding": "gzip"}, expected=(frontend / "chunks/only-deadbeef.mjs.gz").read_bytes())
    send(path + ".gz", expected=compressed)
    send("/text%20%E4%B8%AD%E6%96%87.txt", expected=(frontend / "text 中文.txt").read_bytes())
    send("/release-deadbeef.dir/index.html", expected=b"nested entry")
    for suffix, content in (("locale-extra", b"hashed suffix"), ("-extra", b"hashed dash suffix")):
        result = send(f"/chunks/multi-deadbeef.{suffix}.js", expected=content)
        assert result["cache-control"] == "public, max-age=31536000, immutable"
    entry = (frontend / "index.html").read_bytes()
    for route in ("/", "/settings", "/missing.js", "/.env", "/%2e%2e%2fsecret.txt"):
        result = send(route, expected=entry)
        assert result["cache-control"] == "public, max-age=0"
    send("/settings", {"Accept-Encoding": "gzip"}, expected=(frontend / "index.html.gz").read_bytes())
    for route in ("/api/not-a-resource", "/download/not-a-route/missing/extra/segment", "/share/not-a-resource"):
        send(route, status=404, expected=b"")
    static_conditions(send, frontend)
    replacement = gzip.compress(b'console.log("replacement chunk is visible")', mtime=0)
    (frontend / (path.lstrip("/") + ".gz")).write_bytes(replacement)
    os.utime(frontend / (path.lstrip("/") + ".gz"), (1609459202, 1609459202))
    refreshed = send(path, {"Accept-Encoding": "gzip", "If-None-Match": encoded["etag"]}, expected=replacement)
    assert refreshed["etag"] != encoded["etag"] and refreshed["last-modified"] != encoded["last-modified"]
    assert all((state / filename).read_bytes() == data for filename, data in before.items()), "Static request changed state"
    observations.extend(api_entities(url, state, output, name + "-api-http"))
    (output / f"{name}.json").write_text(json.dumps(observations, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    return observations


def standalone_resources(frontend_url, backend_url, mount, state, output, name):
    """Exercise the real second listener and its shared backend state."""
    parsed = urlsplit(frontend_url)
    observations = []
    before = {path.name: path.read_bytes() for path in (state / "sub-store.json", state / "root.json")}
    contract_headers = ("content-type", "content-length", "content-encoding", "cache-control", "etag",
                        "last-modified", "vary", "accept-ranges", "content-disposition", "x-powered-by",
                        "access-control-allow-origin", "access-control-allow-methods", "access-control-allow-headers",
                        "content-security-policy", "x-content-type-options", "content-range")

    def send(path, headers=None, method="GET", status=200, expected=None, body=None, api=False):
        data = json.dumps(body, ensure_ascii=False).encode("utf-8") if body is not None else None
        headers = {"Origin": "https://allowed.invalid", **({"Content-Type": "application/json"} if data else {}), **(headers or {})}
        connection = BufferedHTTPConnection(parsed.hostname, parsed.port, timeout=5)
        before_request = wall_time().astimezone()
        try:
            connection.request(method, path, body=data, headers=headers)
            response = connection.getresponse()
            content = response.read()
            raw = {"method": method, "path": path, "request_headers": headers, "request_body": body,
                   "status": response.status, "headers": {key.lower(): value for key, value in response.getheaders()},
                   "body_base64": base64.b64encode(content).decode("ascii")}
        finally:
            connection.close()
        after_request = wall_time().astimezone()
        raw.update(before=before_request.isoformat(), after=after_request.isoformat())
        with (output / f"{name}.jsonl").open("a", encoding="utf-8") as evidence:
            evidence.write(json.dumps(raw, ensure_ascii=False) + "\n")
        assert raw["status"] == status, f"{name} {method} {path}: {raw}"
        if expected is not None:
            assert content == expected, f"{name} {method} {path}: unexpected body"
        if api:
            assert raw["headers"].get("x-powered-by") == "Sub-Store", "Frontend did not enter backend routing"
            if "content-disposition" in raw["headers"]:
                filename = "sub-store_subscription_" + unquote(path.split("?")[0].rsplit("/", 1)[1])
                encoded = quote(filename, safe="-_.!~*'()")
                stamp = re.fullmatch(r'attachment; filename="' + re.escape(encoded) + r'_(\d{4}-\d{2}-\d{2}_\d{2}-\d{2}-\d{2})\.json"', raw["headers"]["content-disposition"])
                assert stamp, "Proxy lost the encoded export filename"
                instant = datetime.strptime(stamp[1], "%Y-%m-%d_%H-%M-%S").replace(tzinfo=before_request.tzinfo)
                assert before_request - timedelta(seconds=1) <= instant <= after_request, "Proxy export has an invalid calendar time"
        else:
            assert raw["headers"].get("x-powered-by") == "Express"
            assert "access-control-allow-origin" not in raw["headers"], "Static frontend inherited backend CORS"
        compared_headers = ("content-type", "x-powered-by", "vary",
                            "access-control-allow-origin", "access-control-allow-methods", "access-control-allow-headers") if api else contract_headers
        observations.append({"method": method, "path": path, "status": raw["status"],
                             "headers": {key: raw["headers"][key] for key in compared_headers if key in raw["headers"]},
                             "body": json.loads(content) if api and content.startswith(b"{") else raw["body_base64"],
                             "state": json.loads((state / "sub-store.json").read_text(encoding="utf-8"))})
        return raw["headers"]

    frontend = state / "frontend"
    entry = (frontend / "index.html").read_bytes()
    normal = send("/chunks/main-84da2e86.js?cache=1", expected=(frontend / "chunks/main-84da2e86.js").read_bytes())
    send("/chunks/main-84da2e86.js", {"Accept-Encoding": "gzip"}, expected=(frontend / "chunks/main-84da2e86.js.gz").read_bytes())
    send("/chunks/main-84da2e86.js", method="HEAD", expected=b"")
    send("/chunks/main-84da2e86.js", {"If-None-Match": normal["etag"]}, status=304, expected=b"")
    send("/", expected=entry)
    for path in ("/settings", "/missing.js", "/.env", "/%2e%2e%2fsecret.txt"):
        send(path, {"Accept": "text/html"}, expected=entry)
    send("/settings", {"Accept": "*/*", "Accept-Encoding": "gzip"}, expected=(frontend / "index.html.gz").read_bytes())
    send("/settings", {"Accept": "text/html"}, method="HEAD", expected=b"")
    for headers in ({}, {"Accept": "application/json"}, {"Accept": "application/json, text/html"}):
        send("/settings", headers, status=404)
    send("/settings?query=1", {"Accept": "text/html"}, method="POST", status=404)
    static_conditions(send, frontend)
    assert all((state / filename).read_bytes() == data for filename, data in before.items()), "Static frontend changed state"
    if mount:
        api_path = mount.rstrip("/") + "/api"
        send(api_path + "/settings", api=True)
        send(api_path + "/settings?keep=%E4%B8%AD&keep=%2B", method="PATCH", body={"standalone": "转交🙂", "zero": 0}, api=True)
        direct = request(backend_url, {"path": "/api/settings"})
        assert json.loads(direct["body"])["data"]["standalone"] == "转交🙂", "Proxy wrote a different store"
        send(api_path + "/subs", method="POST", body={"name": "独立 %?", "url": "https://example.invalid/list"}, status=201, api=True)
        encoded = quote("独立 %?", safe="")
        exported = send(api_path + "/sub/" + encoded + "?raw=1&opaque=%26%2B", api=True)
        assert "content-disposition" in exported, "Proxy lost raw export headers"
        send(api_path + "/sub/" + encoded, method="PATCH", body={"name": "renamed", "custom": False}, api=True)
        send(api_path + "/sub/renamed", method="DELETE", api=True)
        send(api_path + "/settings", method="OPTIONS", expected=b"", api=True)
        send(api_path + "/settings", {"Origin": "https://denied.invalid"}, method="PATCH", body={"mustNotWrite": True}, status=403, expected=b"CORS origin not allowed", api=True)
        assert "mustNotWrite" not in json.loads((state / "sub-store.json").read_text(encoding="utf-8"))["settings"]
        send(mount.rstrip("/") + "/download/not-a-route/missing/extra/segment?raw=1", status=404, expected=b"", api=True)
        send(api_path.upper() + "/settings", api=True)
        send(api_path + "extra/settings", {"Accept": "text/html"}, expected=entry)
        observations.extend(api_entities(frontend_url + mount.rstrip("/"), state, output, name + "-api-http", "https://allowed.invalid"))
        if mount == "/backend":
            observations.extend(request_bodies(frontend_url + mount, state, output, name + "-body-http", "https://allowed.invalid"))
            for phase in ("live", "restore"):
                observations.extend(bulk_buffers(frontend_url + mount, state, output, name + "-buffers-" + phase, phase, "https://allowed.invalid"))
    else:
        send("/api/settings", status=404)
        send("/api/settings", {"Accept": "text/html"}, expected=entry)
    (output / f"{name}.json").write_text(json.dumps(observations, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    return observations


def configuration_probes(url, state, output, name, limit, allowed, denied, wildcard):
    observations = []
    def send(step, status):
        before = {key: (state / key).read_bytes() for key in ("sub-store.json", "root.json")}
        response = request(url, step)
        after = {key: (state / key).read_bytes() for key in before}
        observations.append({"request": step, "response": {key: response[key] for key in ("status", "body")},
                             "headers": {key: response["headers"].get(key) for key in ("content-type", "access-control-allow-origin", "vary", "x-powered-by")},
                             "state": json.loads(after["sub-store.json"]), "root": json.loads(after["root.json"]),
                             "files_before_base64": {key: base64.b64encode(value).decode() for key, value in before.items()},
                             "files_after_base64": {key: base64.b64encode(value).decode() for key, value in after.items()}})
        assert response["status"] == status, f"{name}: {step}: {response}"
        if status >= 400 or step.get("method", "GET") in ("GET", "OPTIONS"):
            assert before == after, "Rejected/read-only configuration probe changed files"
        return response
    response = send({"path": "/api/settings"}, 200)
    assert response["headers"].get("access-control-allow-origin") == ("*" if wildcard else None)
    for raw, normalized in allowed:
        response = send({"path": "/api/settings", "headers": {"Origin": raw}}, 200)
        assert response["headers"]["access-control-allow-origin"] == ("*" if wildcard else normalized)
        send({"path": "/not-a-route", "method": "OPTIONS", "headers": {"Origin": raw}, "body": {"configProbe": "x" * 2000}}, 200)
    if denied:
        response = send({"path": "/api/settings", "method": "PATCH", "headers": {"Origin": denied}, "body": {"configProbe": "x" * 2000}}, 403)
        assert response["body"] == "CORS origin not allowed" and "access-control-allow-origin" not in response["headers"]
    for value in (1, "x" * 2000):
        body = {"configProbe": value}
        length = len(json.dumps(body, ensure_ascii=False).encode("utf-8"))
        send({"path": "/api/settings", "method": "PATCH", "body": body}, 200 if length <= limit else 413)
    # A nondefault prefix must still exclude the unprefixed management route.
    send({"path": "/api/settings", "origin": True}, 404)
    (output / (name + ".json")).write_text(json.dumps(observations, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    return [{key: value for key, value in row.items() if not key.startswith("files_")} for row in observations]


def error_diagnostics(url, state, output, name, mode, kind):
    parsed = urlsplit(url)
    observations = []
    cases = [
        ("missing-subscription", "GET", "/api/sub/missing", b"", {}, 500,
         "TypeError: Cannot convert undefined or null to object", True),
        ("recipient-coercion", "PATCH", "/api/sub/diagnostic", b'{"age-public-key":{"toString":null}}', {}, 500,
         "TypeError: Cannot convert object to primitive value", True),
        ("parameter-decoding", "GET", "/api/sub/%FF", b"", {}, 400,
         "URIError: Failed to decode param '%FF'", True),
        ("static-precondition", "GET", "/chunks/main-84da2e86.js", b"", {"If-Match": '"wrong"'}, 412,
         "PreconditionFailedError: Precondition Failed", True),
        ("static-range", "GET", "/chunks/main-84da2e86.js", b"", {"Range": "bytes=99999-"}, 416,
         "RangeNotSatisfiableError: Range Not Satisfiable", True),
        ("invalid-json", "PATCH", "/api/settings", b"{", {}, 400, "SyntaxError:", False),
        ("scalar-json", "PATCH", "/api/settings", b"false", {}, 400, "SyntaxError:", False),
        ("unsupported-charset", "PATCH", "/api/settings", b"{}", {"Content-Type": "application/json; charset=latin1"}, 415,
         'UnsupportedMediaTypeError: unsupported charset "LATIN1"', True),
        ("unsupported-encoding", "PATCH", "/api/settings", b"{}", {"Content-Encoding": "br"}, 415,
         'UnsupportedMediaTypeError: unsupported content encoding "br"', True),
        ("json-limit", "PATCH", "/api/settings", b'{"x":"' + b"x" * 80 + b'"}', {}, 413,
         "PayloadTooLargeError: request entity too large", True),
        ("raw-verifier", "PATCH", "/api/settings", b"{}", {"Content-Type": "application/json; charset=utf-16be"}, 403,
         "TypeError [ERR_UNKNOWN_ENCODING]: Unknown encoding: utf-16be", True),
        ("invalid-gzip", "PATCH", "/api/settings", b"not-gzip", {"Content-Encoding": "gzip"}, 400, "Error:", False),
    ]
    prefix = '<!DOCTYPE html>\n<html lang="en">\n<head>\n<meta charset="utf-8">\n<title>Error</title>\n</head>\n<body>\n<pre>'
    suffix = '</pre>\n</body>\n</html>\n'
    reasons = {400: "Bad Request", 403: "Forbidden", 412: "Precondition Failed", 413: "Payload Too Large",
               415: "Unsupported Media Type", 416: "Range Not Satisfiable", 500: "Internal Server Error"}
    for case_id, method, path, body, headers, status, detail, exact in cases:
        before = {filename: (state / filename).read_bytes() for filename in ("sub-store.json", "root.json")}
        connection = BufferedHTTPConnection(parsed.hostname, parsed.port, timeout=5)
        sent_path = path if path.startswith("/chunks/") else parsed.path.rstrip("/") + path
        headers = {"Content-Type": "application/json", **headers}
        try:
            connection.request(method, sent_path, body=body, headers=headers)
            response = connection.getresponse()
            content = response.read()
            actual_headers = {key.lower(): value for key, value in response.getheaders()}
            actual_status = response.status
        finally:
            connection.close()
        after = {filename: (state / filename).read_bytes() for filename in before}
        raw = {"case_id": case_id, "mode": mode, "method": method, "path": sent_path, "request_headers": headers,
               "request_body_base64": base64.b64encode(body).decode(), "status": actual_status, "headers": actual_headers,
               "body_base64": base64.b64encode(content).decode(),
               "files_before_base64": {key: base64.b64encode(value).decode() for key, value in before.items()},
               "files_after_base64": {key: base64.b64encode(value).decode() for key, value in after.items()}}
        with (output / (name + ".jsonl")).open("a", encoding="utf-8") as evidence:
            evidence.write(json.dumps(raw, ensure_ascii=False) + "\n")
        assert actual_status == status, f"{name}/{case_id}: {actual_status}: {content!r}"
        assert before == after, f"{name}/{case_id}: rejected request changed durable state"
        assert actual_headers["content-type"] == "text/html; charset=utf-8"
        assert actual_headers["content-security-policy"] == "default-src 'none'"
        assert actual_headers["x-content-type-options"] == "nosniff"
        assert int(actual_headers["content-length"]) == len(content)
        text = content.decode("utf-8")
        assert text.startswith(prefix) and text.endswith(suffix), f"{name}/{case_id}: HTML envelope changed"
        diagnostic = unescape_html(text[len(prefix):-len(suffix)].replace("<br>", "\n"))
        if mode == "production":
            assert diagnostic == reasons[status]
            comparison = diagnostic
        else:
            first, separator, frames = diagnostic.partition("\n")
            assert first == detail if exact else first.startswith(detail), f"{name}/{case_id}: {first!r}"
            assert separator and re.search(r"(?m)^\s+at " if kind == "node" else r"(?m)^\s*\d+: ", frames), f"{name}/{case_id}: native trace missing"
            comparison = first if exact else detail + " [parser-specific wording]"
        observations.append({"case_id": case_id, "status": status, "diagnostic": comparison,
                             "state": json.loads(after["sub-store.json"]), "root": json.loads(after["root.json"])})
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
              "full_coverage": False, "production_ready": False, "os": platform.system(), "machine": platform.machine(), "phases": [],
              "request_clock": "GetSystemTimePreciseAsFileTime" if os.name == "nt" else "datetime.now",
              "python_version": platform.python_version()}
    image = None
    try:
        report.update(source_provenance(environment, output))
        report["sources"] = {str(path.relative_to(ROOT)): hashlib.sha256(path.read_bytes()).hexdigest()
                             for path in [ROOT / "rust/Cargo.toml", ROOT / "rust/Cargo.lock", ROOT / "rust/Dockerfile", *sorted(path for path in (ROOT / "rust/src").iterdir() if path.is_file()), *sorted((ROOT / "rust/vendor").iterdir())]}
        report["harness_sha256"] = hashlib.sha256(Path(__file__).read_bytes()).hexdigest()
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
        baseline_root = json.loads((output / "node-state/root.json").read_text(encoding="utf-8"))
        _, conversion_cases = load_cases(HERE / "manifest.json", HERE / "fixtures/http-cases.json", "conversion-local")
        _, conversion_restart = load_cases(HERE / "manifest.json", HERE / "fixtures/http-cases.json", "conversion-local-restart")
        conversion_state = {}
        conversion_responses = {}
        conversion_concurrency = {}
        for kind in ("node", "rust"):
            state = output / f"{kind}-conversion-local"
            state.mkdir()
            (state / "sub-store.json").write_text(json.dumps(final["node"], ensure_ascii=False), encoding="utf-8")
            (state / "root.json").write_text(json.dumps(baseline_root, ensure_ascii=False), encoding="utf-8")
            env = {**environment, "SUB_STORE_DATA_BASE_PATH": str(state)}
            label = kind + "-conversion-local"
            manager = oracle(args.node, env, output, report, label) if kind == "node" else listener(args.binary, env, output, report, label, image, host_network=bool(image))
            with manager as url, ControlledUpstream() as upstream:
                results = check_cases(url, state, conversion_cases, output, label, upstream)
                conversion_responses[kind] = list(results)
                report["phases"].append({"name": label, "cases": len(results), "requests": sum(len(case["steps"]) for case in results)})
                conversion_concurrency[kind] = script_concurrency(url, state, output, label)
                conversion_state[kind] = {key: json.loads((state / key).read_text(encoding="utf-8")) for key in ("sub-store.json", "root.json")}
                if kind == "rust":
                    before_pending = {key: (state / key).read_bytes() for key in ("sub-store.json", "root.json")}
                    pending_steps = [
                        {"method": "POST", "path": "/api/proxy/parse", "body": {"data": '{type:"ss",name:"nonfinite",server:"127.0.0.1",port:443,extra:NaN}', "client": "JSON"}},
                        {"method": "POST", "path": "/api/proxy/parse", "body": {"data": '{type: ss, name: cyclic, server: 127.0.0.1, port: 443, extra: &loop {self: *loop}}', "client": "JSON"}},
                        {"method": "POST", "path": "/api/proxy/parse", "body": {"data": "ss://none:p@127.0.0.1:443?plugin=shadow-tls%3Bversion%3Dbad", "client": "URI"}},
                        {"method": "POST", "path": "/api/proxy/parse", "body": {"data": "ssr://aG9zdDo6b3JpZ2luOmFlcy0xMjgtY2ZiOnBsYWluOmNBLz8=", "client": "URI"}},
                        {"method": "POST", "path": "/api/proxy/parse", "body": {"data": "Huge = snell, 127.0.0.1, 443, psk=p, version=" + "9" * 400, "client": "JSON"}},
                        {"method": "POST", "path": "/api/proxy/parse", "body": {"data": 'Huge = AnyTLS, 127.0.0.1, 443, "password", min-idle-session=' + "9" * 400, "client": "JSON"}},
                        {"method": "POST", "path": "/api/proxy/parse", "body": {"data": 'WG = WireGuard, private-key=p, peers=[{public-key=k}]', "client": "JSON"}},
                        {"path": "/download/native-local/URI?$options=response-hook"},
                    ]
                    pending_responses = [{"request": step, "response": request(url, step)} for step in pending_steps]
                    (output / "rust-conversion-pending.json").write_text(json.dumps(pending_responses, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
                    assert all(item["response"]["status"] == 501 and json.loads(item["response"]["body"])["error"]["code"] == "FEATURE_NOT_IMPLEMENTED" for item in pending_responses)
                    assert all((state / key).read_bytes() == value for key, value in before_pending.items())
                    assert request(url, {"path": "/download/native-local/URI"})["status"] == 200
            before = (state / "sub-store.json").read_bytes()
            label += "-restart"
            manager = oracle(args.node, env, output, report, label) if kind == "node" else listener(args.binary, env, output, report, label, image, host_network=bool(image))
            with manager as url:
                conversion_responses[kind].extend(check_cases(url, state, conversion_restart, output, label))
                assert (state / "sub-store.json").read_bytes() == before, "Read-only conversion changed durable state"
                report["phases"].append({"name": label, "cases": len(conversion_restart)})
        for rust_case, node_case in zip(conversion_responses["rust"], conversion_responses["node"], strict=True):
            assert rust_case["id"] == node_case["id"]
            for rust_step, node_step in zip(rust_case["steps"], node_case["steps"], strict=True):
                actual, expected = rust_step["response"], node_step["response"]
                assert actual["status"] == expected["status"] and actual["body"] == expected["body"], (rust_case["id"], actual, expected)
                for key in ("content-type", "content-length", "etag", "allow"):
                    assert actual["headers"].get(key) == expected["headers"].get(key), (rust_case["id"], key, actual, expected)
        check_json(conversion_state["rust"], conversion_state["node"])
        check_json(conversion_concurrency["rust"], conversion_concurrency["node"])
        report["native_script_concurrency_and_state_checked"] = True
        report["native_local_ss_conversion_checked"] = True
        configurations = [
            ("default", {}, 1048576, [("https://sub-store.vercel.app", "https://sub-store.vercel.app")], "https://denied.invalid", False),
            ("custom-name", {"SUB_STORE_BACKEND_CUSTOM_NAME": "configuration-fixture", "SUB_STORE_BODY_JSON_LIMIT": "+1GB"}, 1073741824,
             [("http://127.0.0.1:9911", "http://127.0.0.1:9911")], "https://127.0.0.1:9911", False),
            ("wildcard", {"SUB_STORE_CORS_ALLOWED_ORIGINS": "https://unused.invalid,*", "SUB_STORE_BODY_JSON_LIMIT": "1.5 kb"}, 1536,
             [("https://arbitrary.invalid", "https://arbitrary.invalid")], None, True),
            ("zero", {"SUB_STORE_BODY_JSON_LIMIT": "0", "SUB_STORE_CORS_ALLOWED_ORIGINS": "'https://ALLOWED.invalid/a'，https://second.invalid"}, 0,
             [("https://allowed.invalid", "https://allowed.invalid"), ("https://second.invalid", "https://second.invalid")], "https://denied.invalid", False),
            ("integer-prefix", {"SUB_STORE_BODY_JSON_LIMIT": "64junk", "SUB_STORE_BACKEND_PREFIX": "false", "SUB_STORE_FRONTEND_BACKEND_PATH": "/custom",
                                "SUB_STORE_CORS_ALLOWED_ORIGINS": "\ufeffhttps://allowed.invalid\ufeff"}, 64,
             [("https://allowed.invalid", "https://allowed.invalid")], "https://denied.invalid", False),
            ("integer-exponent", {"SUB_STORE_BODY_JSON_LIMIT": "1e3"}, 1,
             [("https://sub-store.vercel.app", "https://sub-store.vercel.app")], "https://denied.invalid", False),
        ]
        for config_name, overrides, limit, allowed, denied, wildcard in configurations:
            observed = {}
            for kind in ("node", "rust"):
                label = kind + "-config-" + config_name
                state = output / label
                state.mkdir()
                (state / "sub-store.json").write_text(json.dumps(final["node"], ensure_ascii=False), encoding="utf-8")
                (state / "root.json").write_text(json.dumps({**baseline_root, "configurationRoot": [None, False, 0]}), encoding="utf-8")
                env = {**environment, **overrides, "NODE_ENV": "production", "SUB_STORE_DATA_BASE_PATH": str(state)}
                manager = oracle(args.node, env, output, report, label) if kind == "node" else listener(args.binary, env, output, report, label, image)
                with manager as url:
                    observed[kind] = configuration_probes(url, state, output, label, limit, allowed, denied, wildcard)
            check_json(observed["rust"], observed["node"])
        report["listener_prefix_cors_and_json_limit_configurations_checked"] = [row[0] for row in configurations]
        for mode in ("default", "development", "production"):
            observed = {}
            for kind in ("node", "rust"):
                label = kind + "-diagnostics-" + mode
                state = output / label
                state.mkdir()
                document = {**final["node"], "subs": [{"name": "diagnostic", "source": "local", "content": ""}], "collections": []}
                (state / "sub-store.json").write_text(json.dumps(document, ensure_ascii=False), encoding="utf-8")
                (state / "root.json").write_text(json.dumps(baseline_root, ensure_ascii=False), encoding="utf-8")
                static_fixture(state / "frontend")
                env = {**environment, "SUB_STORE_DATA_BASE_PATH": str(state), "SUB_STORE_BACKEND_MERGE": "true",
                       "SUB_STORE_FRONTEND_PATH": str(state / "frontend"), "SUB_STORE_BODY_JSON_LIMIT": "64b"}
                env.pop("NODE_ENV", None)
                if mode != "default":
                    env["NODE_ENV"] = mode
                manager = oracle(args.node, env, output, report, label) if kind == "node" else listener(args.binary, env, output, report, label, image)
                with manager as url:
                    observed[kind] = error_diagnostics(url, state, output, label, mode, kind)
            check_json(observed["rust"], observed["node"])
        report["error_environment_modes_checked"] = ["default", "development", "production"]
        report["development_error_stack_parity"] = "Native frames and parser-specific wording differ; raw responses retained; fixed reasons, classes, envelopes and rejected-state boundaries match"
        legacy = {"settings": {}, "subs": {"z-last": {"name": "z-last", "display-name": "旧数据🙂", "process": [{"type": "Useless Filter"}, {"type": "Set Property Operator", "args": {"key": "udp", "value": False}}, {"type": "Flag Operator", "args": False}, {"type": "Script Operator", "args": {"content": "require('node:fs')\r\n"}}]}, "a-first": {"name": "a-first"}}, "collections": {"c": {"name": "c", "ua": "old", "subscriptions": ["z-last"], "extra": False}}, "unknown": {"nil": None, "zero": 0, "flag": False}}
        legacy["builtin"] = {"must": "be removed"}
        legacy["subs"].update({"empty-process": {"name": "empty-process", "source": "local", "display-name": False, "process": []},
                               "string-process": {"name": "string-process", "process": "legacy-string"},
                               "zero-length": {"name": "zero-length", "process": {"length": 0, "opaque": False}}})
        legacy["subs"]["z-last"]["process"].extend(
            [{"type": "Set Property Operator", "args": {"key": key, "value": value}}
             for key, value in (("tfo", []), ("skip-cert-verify", None), ("aead", True), ("unknown", False))]
            + [{"type": "Keyword Filter"}, {"type": "Custom Keyword Operator"},
               {"type": "Flag Operator"}, {"type": "Flag Operator", "args": None},
               {"type": ["opaque"], "args": {"content": "keep\r\n"}}, {"type": ["Keyword"]},
               {}, False, 0, "ignored", []])
        legacy["collections"]["c"]["process"] = [{"type": "Flag Operator", "args": "truthy"},
                                                 {"type": "Script Operator", "args": {"content": "// JS 保留\r\n🙂"}}]
        for key in ("10", "2", "01", "0", "4294967295", "4294967294"):
            legacy["subs"][key] = {"name": "index-" + key}
        original = backup_encodings(legacy)["separated-urlsafe"].encode("utf-8")
        (output / "migration-base64-input.json").write_text(json.dumps({"encoded": original.decode("utf-8"), "decoded": legacy}, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
        root = {"opaque": {"text": "cache🙂", "zero": 0, "flag": False}, "sub-store-cached-resource": "{}"}
        migrated = {}
        for kind in ("node", "rust"):
            state = output / f"{kind}-migration"
            state.mkdir()
            (state / "sub-store.json").write_bytes(original)
            (state / "root.json").write_text(json.dumps(root, ensure_ascii=False), encoding="utf-8")
            env = {**environment, "SUB_STORE_DATA_BASE_PATH": str(state)}
            manager = oracle(args.node, env, output, report, "node-migrate") if kind == "node" else listener(args.binary, env, output, report, "rust-migrate", image)
            with manager as url:
                response = request(url, {"path": "/api/storage"})
                assert response["status"] == 200
                migrated[kind] = json.loads(response["body"])
                assert "builtin" not in migrated[kind]
                (output / f"{kind}-migration.json").write_text(json.dumps(migrated[kind], ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
                if kind == "rust":
                    assert (state / "sub-store.pre-rust.json").read_bytes() == original
                    check_json(json.loads((state / "root.json").read_text(encoding="utf-8")), root)
            for key in ("subs", "collections", "artifacts", "rules", "files", "tokens", "schemaVersion", "unknown"):
                if kind == "rust":
                    check_json(migrated[kind][key], migrated["node"][key])
        report["legacy_base64_migration_and_snapshots_checked"] = True
        report["legacy_base64_noisy_startup_checked"] = True
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
        unknown = {"zero": 0, "flag": False, "text": "保留🙂\r\n"}
        sources = ["1", "01", 1, True, 0, False, None, ["1"], [1], [0], [], {}]
        seed = {"schemaVersion": "2.0", "settings": {}, "rules": [], "modules": [], "tokens": [], "archives": [],
                "subs": [{"name": name, "subscriptions": ["reserved"], "unknown": unknown} for name in ("1", "0")],
                "collections": [{"name": "refs", "subscriptions": ["1", "1", "0", "0", 1, 0]}, {"name": "1", "subscriptions": []}],
                "artifacts": [{"name": f"sub-{index}", "type": "subscription", "source": value} for index, value in enumerate(sources)]
                             + [{"name": f"col-{index}", "type": "collection", "source": value} for index, value in enumerate(["1", 1, True, [1]])],
                "files": [{"name": f"sub-{index}", "sourceType": "subscription", "sourceName": value} for index, value in enumerate(sources)],
                "opaque": unknown}
        crud = {}
        final_crud = {}
        for kind in ("node", "rust"):
            state = output / f"{kind}-crud"
            state.mkdir()
            (state / "sub-store.json").write_text(json.dumps(seed, ensure_ascii=False), encoding="utf-8")
            env = {**environment, "SUB_STORE_DATA_BASE_PATH": str(state), "SUB_STORE_BODY_JSON_LIMIT": "32kb"}
            label = f"{kind}-crud"
            env["NODE_ENV"] = "production"
            manager = oracle(args.node, env, output, report, label) if kind == "node" else listener(args.binary, env, output, report, label, image)
            with manager as url:
                if kind == "rust":
                    before = json.loads((state / "sub-store.json").read_text(encoding="utf-8"))
                    saved = state / "saved-state.json"
                    (state / "sub-store.json").rename(saved)
                    (state / "sub-store.json").mkdir()
                    try:
                        failed = request(url, {"method": "PATCH", "path": "/api/sub/1", "body": {"name": "must-not-commit"}})
                        unchanged = request(url, {"path": "/api/storage"})
                        (output / "crud-write-failure.json").write_text(json.dumps({"before": before, "failure": failed, "after": unchanged}, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
                        assert failed["status"] == 500 and json.loads(failed["body"])["error"]["code"] == "STATE_WRITE_FAILED"
                        assert unchanged["status"] == 200
                        check_json(json.loads(unchanged["body"]), before)
                    finally:
                        (state / "sub-store.json").rmdir()
                        saved.rename(state / "sub-store.json")
                    report["crud_failed_rename_preserved_all_references"] = True
                crud[kind] = crud_edges(url, state, output, label)
                crud[kind].extend(request_bodies(url, state, output, label + "-body-http"))
                final_crud[kind] = json.loads((state / "sub-store.json").read_text(encoding="utf-8"))
                report["phases"].append({"name": label, "requests": len(crud[kind])})
            manager = oracle(args.node, env, output, report, label + "-restart") if kind == "node" else listener(args.binary, env, output, report, label + "-restart", image)
            with manager as url:
                response = request(url, {"path": "/api/storage"})
                (output / f"{label}-restart.json").write_text(json.dumps(response, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
                assert response["status"] == 200
                check_json(json.loads(response["body"]), final_crud[kind])
        check_json(crud["rust"], crud["node"])
        check_json(final_crud["rust"], final_crud["node"])
        rollback = output / "crud-node-rollback"
        rollback.mkdir()
        for filename in ("sub-store.json", "root.json"):
            shutil.copyfile(output / "rust-crud" / filename, rollback / filename)
        with oracle(args.node, {**environment, "SUB_STORE_DATA_BASE_PATH": str(rollback)}, output, report, "crud-node-rollback") as url:
            response = request(url, {"path": "/api/storage"})
            (output / "crud-node-rollback.json").write_text(json.dumps(response, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
            assert response["status"] == 200
            check_json(json.loads(response["body"]), final_crud["rust"])
        report["crud_http_and_state_equal"] = True
        report["crud_restart_and_node_rollback_checked"] = True
        report["crud_non_object_and_ordered_null_lookup_checked"] = True
        report["crud_scalar_json_rejected_before_dispatch"] = True
        report["crud_missing_subscription_production_html_checked"] = True
        report["crud_mixed_entries_restore_checked"] = True
        report["request_body_identity_json_form_raw_checked"] = True
        report["request_body_limits_errors_and_cors_checked"] = True
        report["bulk_object_rejection_and_unchanged_state_checked"] = True
        report["request_body_restart_and_node_rollback_checked"] = True
        report["compressed_request_body_parity"] = "gzip_and_deflate_checked"
        report["compressed_body_expanded_limits_and_errors_checked"] = True
        report["compressed_body_chunking_and_members_checked"] = True
        report["request_media_parameters_and_selection_checked"] = True
        report["request_charset_aliases_and_verifier_order_checked"] = True
        backup = json.loads((HERE / "fixtures/frontend-backup.json").read_text(encoding="utf-8"))
        root = {"opaque": {"text": "根缓存🙂\r\n", "zero": 0, "flag": False, "nil": None}, "sub-store": "previous backup"}
        for key in ("sub-store-cached-resource", "sub-store-cached-headers-resource", "sub-store-cached-script-resource"):
            root[key] = json.dumps({"fixture": {"time": 4102444800000, "data": "cached"}})
        zones = [("local", {}, None), ("tokyo", {"TZ": "Asia/Tokyo"}, 540),
                 ("kathmandu", {"TZ": "Asia/Kathmandu"}, 345)]
        report["backup_fixture_sha256"] = hashlib.sha256((HERE / "fixtures/frontend-backup.json").read_bytes()).hexdigest()
        for zone, overrides, offset in zones:
            restored = {}
            for kind in ("node", "rust"):
                state = output / f"{kind}-backup-{zone}"
                state.mkdir()
                (state / "sub-store.json").write_text(json.dumps(backup, ensure_ascii=False), encoding="utf-8")
                (state / "root.json").write_text(json.dumps(root, ensure_ascii=False), encoding="utf-8")
                env = {**environment, **overrides, "SUB_STORE_DATA_BASE_PATH": str(state)}
                prefix = f"{kind}-backup-{zone}"
                manager = oracle(args.node, env, output, report, prefix) if kind == "node" else listener(args.binary, env, output, report, prefix, image)
                with manager as url:
                    restored[kind] = backup_edges(url, state, output, prefix, offset)
                    report["phases"].append({"name": prefix, "requests": len(restored[kind])})
                    if kind == "rust" and zone == "local":
                        for filename in ("root.json", "sub-store.json"):
                            before = {key: (state / key).read_bytes() for key in ("root.json", "sub-store.json")}
                            saved = state / (filename + ".saved")
                            (state / filename).rename(saved)
                            (state / filename).mkdir()
                            try:
                                failed = request(url, {"method": "POST", "path": "/api/storage", "body": {"content": json.dumps(backup)}})
                                unchanged = request(url, {"path": "/api/storage"})
                                (output / f"restore-write-failure-{filename}").write_text(json.dumps({"failure": failed, "after": unchanged}, indent=2) + "\n", encoding="utf-8")
                                assert failed["status"] == 500 and json.loads(failed["body"])["error"]["code"] == "STATE_WRITE_FAILED"
                                check_json(json.loads(unchanged["body"]), json.loads(before["sub-store.json"]))
                            finally:
                                (state / filename).rmdir()
                                saved.rename(state / filename)
                            assert all((state / key).read_bytes() == value for key, value in before.items())
                            assert not (state / ".sub-store-restore.pending.json").exists()
                        report["restore_write_failure_preserved_both_files"] = True
                        original = json.loads((state / "sub-store.json").read_text(encoding="utf-8"))
                        imported = request(url, {"method": "POST", "path": "/api/storage",
                                                 "body": {"content": json.dumps({**original, "settings": False})}})
                        failure_path = output / "settings-init-write-failure.jsonl"
                        failure_path.write_text(json.dumps({"phase": "import", "response": imported}) + "\n", encoding="utf-8")
                        assert imported["status"] == 200
                        before = {key: (state / key).read_bytes() for key in ("sub-store.json", "root.json")}
                        saved = state / "sub-store.json.saved"
                        (state / "sub-store.json").rename(saved)
                        (state / "sub-store.json").mkdir()
                        try:
                            failed = request(url, {"path": "/api/settings"})
                            unchanged = request(url, {"path": "/api/storage"})
                            with failure_path.open("a", encoding="utf-8") as evidence:
                                evidence.write(json.dumps({"phase": "denied-initialization", "response": failed, "after": unchanged}) + "\n")
                            assert failed["status"] == 500 and json.loads(failed["body"])["error"]["code"] == "STATE_WRITE_FAILED"
                            check_json(json.loads(unchanged["body"]), json.loads(before["sub-store.json"]))
                        finally:
                            (state / "sub-store.json").rmdir()
                            saved.rename(state / "sub-store.json")
                        after = {key: (state / key).read_bytes() for key in before}
                        recovered = request(url, {"path": "/api/settings"})
                        with failure_path.open("a", encoding="utf-8") as evidence:
                            evidence.write(json.dumps({"phase": "recovery", "response": recovered,
                                                       "files_before_base64": {key: base64.b64encode(value).decode("ascii") for key, value in before.items()},
                                                       "files_after_base64": {key: base64.b64encode(value).decode("ascii") for key, value in after.items()}}) + "\n")
                        assert before == after, "Failed settings initialization changed durable files"
                        assert recovered["status"] == 200
                        check_json(json.loads(recovered["body"])["data"], {})
                        restored_array = request(url, {"method": "POST", "path": "/api/storage", "body": {"content": json.dumps(original, ensure_ascii=False)}})
                        with failure_path.open("a", encoding="utf-8") as evidence:
                            evidence.write(json.dumps({"phase": "restore", "response": restored_array}) + "\n")
                        assert restored_array["status"] == 200
                        check_json(json.loads((state / "sub-store.json").read_text(encoding="utf-8")), original)
                        report["settings_initialization_failed_write_preserved_state"] = True
                prefix += "-restart"
                manager = oracle(args.node, env, output, report, prefix) if kind == "node" else listener(args.binary, env, output, report, prefix, image)
                with manager as url:
                    check_json(json.loads(request(url, {"path": "/api/storage"})["body"]), restored[kind][-1]["state"])
                    check_json(json.loads((state / "root.json").read_text(encoding="utf-8"))["sub-store"], restored[kind][-1]["root"]["sub-store"])
                    check_json(json.loads(request(url, {"path": "/api/settings"})["body"])["data"], restored[kind][-1]["state"]["settings"])
            check_json(restored["rust"], restored["node"])
        report["backup_http_and_root_state_equal"] = True
        report["backup_base64_decoder_variants_checked"] = True
        report["array_settings_restore_patch_and_restart_checked"] = True
        report["legacy_falsy_lists_migration_checked"] = True
        report["calendar_filename_timezones_checked"] = [zone for zone, _, _ in zones]
        old_data = (output / "rust-backup-local/sub-store.json").read_bytes()
        old_root = (output / "rust-backup-local/root.json").read_bytes()
        for phase in ("root-only", "both"):
            state = output / f"restore-interrupted-{phase}"
            state.mkdir()
            (state / "sub-store.json").write_bytes(b'{"settings":{}}' if phase == "both" else old_data)
            (state / "root.json").write_bytes(b'{"partially_restored":true}')
            pending = state / ".sub-store-restore.pending.json"
            pending.write_text(json.dumps({"data": base64.b64encode(old_data).decode("ascii"), "root": base64.b64encode(old_root).decode("ascii")}), encoding="utf-8")
            with listener(args.binary, {**environment, "SUB_STORE_DATA_BASE_PATH": str(state)}, output, report, phase, image) as url:
                response = request(url, {"path": "/api/storage"})
                (output / f"restore-recovery-{phase}.json").write_text(json.dumps(response, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
                assert response["status"] == 200
                check_json(json.loads(response["body"]), json.loads(old_data))
                assert (state / "root.json").read_bytes() == old_root
                assert (state / "sub-store.pre-rust.json").read_bytes() == old_data
                assert not pending.exists()
        report["interrupted_restore_recovered_both_files"] = True
        restored_rollback = output / "restored-node-rollback"
        restored_rollback.mkdir()
        for filename in ("sub-store.json", "root.json"):
            shutil.copyfile(output / "rust-backup-local" / filename, restored_rollback / filename)
        with oracle(args.node, {**environment, "SUB_STORE_DATA_BASE_PATH": str(restored_rollback)}, output, report, "restored-node-rollback") as url:
            check_json(json.loads(request(url, {"path": "/api/storage"})["body"]), json.loads(old_data))
            check_json(json.loads((restored_rollback / "root.json").read_text(encoding="utf-8"))["sub-store"], json.loads(old_root)["sub-store"])
            check_json(json.loads(request(url, {"path": "/api/settings"})["body"])["data"], json.loads(old_data)["settings"])
        report["node_reads_restored_rust_data_and_root"] = True
        rollback = output / "node-rollback"
        rollback.mkdir()
        shutil.copyfile(output / "rust-state/sub-store.json", rollback / "sub-store.json")
        with oracle(args.node, {**environment, "SUB_STORE_DATA_BASE_PATH": str(rollback)}, output, report, "node-rollback") as url:
            response = request(url, {"path": "/api/storage"})
            check_json(json.loads(response["body"]), final["rust"])
        report["node_reads_rust_written_state"] = True
        for label, value in (("true", True), ("false", False), ("zero", 0), ("number", 1.25), ("empty", ""), ("string", "设置🙂\r\n")):
            startup = {}
            expected = value if value else {}
            for kind in ("node", "rust"):
                state = output / f"{kind}-settings-{label}"
                state.mkdir()
                original = json.dumps({**backup, "settings": value}, ensure_ascii=False).encode("utf-8")
                (state / "sub-store.json").write_bytes(original)
                (state / "root.json").write_text(json.dumps(root, ensure_ascii=False), encoding="utf-8")
                env = {**environment, "SUB_STORE_DATA_BASE_PATH": str(state)}
                prefix = f"{kind}-settings-{label}"
                manager = oracle(args.node, env, output, report, prefix) if kind == "node" else listener(args.binary, env, output, report, prefix, image)
                with manager as url:
                    startup[kind] = settings_reads(url, state, output, prefix, expected)
                    if kind == "rust":
                        assert (state / "sub-store.pre-rust.json").read_bytes() == original
            check_json(startup["rust"], startup["node"])
            if value:
                state = output / f"settings-node-rollback-{label}"
                state.mkdir()
                for filename in ("sub-store.json", "root.json"):
                    shutil.copyfile(output / f"rust-settings-{label}" / filename, state / filename)
                with oracle(args.node, {**environment, "SUB_STORE_DATA_BASE_PATH": str(state)}, output, report, f"settings-rollback-{label}") as url:
                    check_json(settings_reads(url, state, output, f"settings-rollback-{label}", value), startup["rust"])
        report["scalar_settings_import_and_startup_checked"] = True
        report["scalar_settings_node_rollback_checked"] = True
        unicode_results = {}
        for kind in ("node", "rust"):
            state = output / (kind + "-unicode")
            state.mkdir()
            document = {**seed, "subs": [], "collections": [], "settings": "A中🙂\ue000d800\ud800",
                        "unknown": {"\udfff": "\ud800", "literal": "\ue000d800"}}
            (state / "sub-store.json").write_text(json.dumps(document, ensure_ascii=True), encoding="utf-8")
            (state / "root.json").write_text(json.dumps({**root, "unicodeRoot": document["unknown"]}, ensure_ascii=True), encoding="utf-8")
            env = {**environment, "NODE_ENV": "production", "SUB_STORE_DATA_BASE_PATH": str(state)}
            label = kind + "-unicode"
            manager = oracle(args.node, env, output, report, label) if kind == "node" else listener(args.binary, env, output, report, label, image)
            with manager as url:
                unicode_results[kind] = unicode_state(url, state, output, label + "-live", mutate=True)
            manager = oracle(args.node, env, output, report, label + "-restart") if kind == "node" else listener(args.binary, env, output, report, label + "-restart", image)
            with manager as url:
                unicode_results[kind].extend(unicode_state(url, state, output, label + "-restart"))
        check_json(unicode_results["rust"], unicode_results["node"])
        state = output / "unicode-node-rollback"
        state.mkdir()
        for filename in ("sub-store.json", "root.json"):
            shutil.copyfile(output / "rust-unicode" / filename, state / filename)
        with oracle(args.node, {**environment, "NODE_ENV": "production", "SUB_STORE_DATA_BASE_PATH": str(state)}, output, report, "unicode-node-rollback") as url:
            rollback_results = unicode_state(url, state, output, "unicode-node-rollback")
        check_json(rollback_results, unicode_results["node"][-len(rollback_results):])
        report["unicode_json_restart_and_node_rollback_checked"] = True
        report["string_settings_patch_parity"] = True
        buffer_results = {}
        for kind in ("node", "rust"):
            state = output / (kind + "-buffers")
            state.mkdir()
            (state / "sub-store.json").write_text(json.dumps({**seed, "subs": [], "collections": []}), encoding="utf-8")
            (state / "root.json").write_text(json.dumps({**root, "opaque": unknown}), encoding="utf-8")
            env = {**environment, "NODE_ENV": "production", "SUB_STORE_DATA_BASE_PATH": str(state)}
            label = kind + "-buffers"
            manager = oracle(args.node, env, output, report, label) if kind == "node" else listener(args.binary, env, output, report, label, image)
            with manager as url:
                buffer_results[kind] = bulk_buffers(url, state, output, label + "-live", "live")
                if kind == "rust":
                    before = {filename: (state / filename).read_bytes() for filename in ("sub-store.json", "root.json")}
                    saved = state / "saved-state.json"
                    (state / "sub-store.json").rename(saved)
                    (state / "sub-store.json").mkdir()
                    failures = []
                    try:
                        for plural in ("subs", "collections"):
                            failed = request(url, {"method": "PUT", "path": "/api/" + plural, "body": []})
                            live_error = request(url, {"method": "POST", "path": "/api/" + plural, "body": {"name": "buffer-new"}})
                            failures.append({"failed": failed, "live_error": live_error})
                        failed_restore = request(url, {"method": "POST", "path": "/api/storage",
                                                       "body": {"content": json.dumps(seed, ensure_ascii=False)}})
                        live_after_restore = request(url, {"method": "POST", "path": "/api/collections", "body": {"name": "buffer-new"}})
                    finally:
                        (state / "sub-store.json").rmdir()
                        saved.rename(state / "sub-store.json")
                    failure_record = {"requests": failures, "failed_restore": failed_restore, "live_after_restore": live_after_restore,
                                      "files_before_base64": {key: base64.b64encode(value).decode() for key, value in before.items()},
                                      "files_after_base64": {key: base64.b64encode((state / key).read_bytes()).decode() for key in before}}
                    (output / "buffer-write-failure.json").write_text(json.dumps(failure_record, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
                    assert failure_record["files_before_base64"] == failure_record["files_after_base64"]
                    for result in [record["failed"] for record in failures] + [failed_restore]:
                        assert result["status"] == 500 and json.loads(result["body"])["error"]["code"] == "STATE_WRITE_FAILED"
                    for result in [record["live_error"] for record in failures] + [live_after_restore]:
                        assert result["status"] == 500
                        check_json(json.loads(result["body"]), {"status": "failed", "error": {"message": "e.push is not a function"}})
                    report["raw_bulk_failed_write_preserves_live_identity"] = True
                buffer_results[kind].extend(bulk_buffers(url, state, output, label + "-restore", "restore"))
                buffer_results[kind].extend(bulk_buffers(url, state, output, label + "-before-restart", "live-before-restart"))
            manager = oracle(args.node, env, output, report, label + "-restart") if kind == "node" else listener(args.binary, env, output, report, label + "-restart", image)
            with manager as url:
                buffer_results[kind].extend(bulk_buffers(url, state, output, label + "-restart", "stored"))
        check_json(buffer_results["rust"], buffer_results["node"])
        buffer_rollback = output / "buffers-node-rollback"
        buffer_rollback.mkdir()
        for filename in ("sub-store.json", "root.json"):
            shutil.copyfile(output / "rust-buffers" / filename, buffer_rollback / filename)
        with oracle(args.node, {**environment, "NODE_ENV": "production", "SUB_STORE_DATA_BASE_PATH": str(buffer_rollback)}, output, report, "buffers-node-rollback") as url:
            rolled_back = bulk_buffers(url, buffer_rollback, output, "buffers-node-rollback", "stored")
            check_json(rolled_back, json.loads((output / "rust-buffers-restart.json").read_text(encoding="utf-8")))
        report["raw_bulk_buffer_restore_restart_and_node_rollback_checked"] = True
        if not image:
            corrupt = output / "corrupt-state"
            corrupt.mkdir()
            (corrupt / "sub-store.json").write_bytes(b"invalid backup \xff")
            result = execute([args.binary], ROOT, {**environment, "SUB_STORE_DATA_BASE_PATH": str(corrupt)}, output, "corrupt-startup", timeout=10)
            assert result.returncode != 0 and (corrupt / "sub-store.json").read_bytes() == b"invalid backup \xff"
            report["corrupt_state_not_reset"] = True
            for label, document in (("invalid-legacy-record", {"settings": {}, "subs": [None]}),
                                    ("invalid-legacy-processes", {"settings": {}, "subs": [{"process": {}}]}),
                                    ("invalid-legacy-process", {"settings": {}, "subs": [{"process": [None]}]}),
                                    ("invalid-legacy-process-type", {"settings": {}, "subs": [{"process": [{"type": True}]}]}),
                                    ("missing-legacy-operator-args", {"settings": {}, "subs": [{"process": [{"type": "Set Property Operator"}]}]}),
                                    ("invalid-other-family", {"schemaVersion": "2.0", "settings": {}, "artifacts": [None]}),
                                    ("invalid-buffer-byte", {"schemaVersion": "2.0", "settings": {}, "subs": {"type": "Buffer", "data": [256]}}),
                                    ("invalid-buffer-shape", {"schemaVersion": "2.0", "settings": {}, "collections": {"type": "Buffer", "data": [0], "extra": True}})):
                state = output / label
                state.mkdir()
                original = json.dumps(document).encode("utf-8")
                (state / "sub-store.json").write_bytes(original)
                (state / "root.json").write_bytes(old_root)
                result = execute([args.binary], ROOT, {**environment, "SUB_STORE_DATA_BASE_PATH": str(state)}, output, label, timeout=10)
                assert result.returncode != 0 and (state / "sub-store.json").read_bytes() == original
                assert (state / "root.json").read_bytes() == old_root
            report["invalid_legacy_and_other_family_records_not_reset"] = True
            corrupt_restore = output / "corrupt-restore"
            corrupt_restore.mkdir()
            (corrupt_restore / "sub-store.json").write_bytes(old_data)
            (corrupt_restore / "root.json").write_bytes(old_root)
            pending = corrupt_restore / ".sub-store-restore.pending.json"
            invalid_record = json.dumps({"data": base64.b64encode(old_data).decode("ascii"), "root": "invalid!"}).encode("utf-8")
            pending.write_bytes(invalid_record)
            result = execute([args.binary], ROOT, {**environment, "SUB_STORE_DATA_BASE_PATH": str(corrupt_restore)}, output, "corrupt-restore-startup", timeout=10)
            assert result.returncode != 0 and pending.read_bytes() == invalid_record
            assert (corrupt_restore / "sub-store.json").read_bytes() == old_data
            assert (corrupt_restore / "root.json").read_bytes() == old_root
            report["corrupt_restore_not_reset"] = True
        for prefix in ("/backend", "/"):
            static = {}
            for kind in ("node", "rust"):
                label = f"{kind}-static-{'root' if prefix == '/' else 'prefix'}"
                state = output / label
                state.mkdir()
                (state / "sub-store.json").write_text(json.dumps(backup, ensure_ascii=False), encoding="utf-8")
                (state / "root.json").write_text(json.dumps(root, ensure_ascii=False), encoding="utf-8")
                static_fixture(state / "frontend")
                env = {**environment, "NODE_ENV": "production", "SUB_STORE_DATA_BASE_PATH": str(state), "SUB_STORE_BACKEND_MERGE": "true",
                       "SUB_STORE_FRONTEND_BACKEND_PATH": prefix, "SUB_STORE_FRONTEND_PATH": str(state / "frontend")}
                manager = oracle(args.node, env, output, report, label) if kind == "node" else listener(args.binary, env, output, report, label, image)
                with manager as url:
                    static[kind] = static_resources(url, state, output, label)
                    report["phases"].append({"name": label, "requests": len(static[kind])})
            check_json(static["rust"], static["node"])
        report["merged_frontend_static_http_equal"] = True
        report["static_prefix_configurations_checked"] = ["/backend", "/"]
        report["static_ranges_preconditions_checked"] = True
        report["static_conditional_error_mode"] = "production"
        for mode, mount, prefix in (("prefixed", "/backend", "true"), ("unprefixed", "/gateway", ""),
                                    ("root", "/", "true"), ("static-only", "", "")):
            standalone = {}
            for kind in ("node", "rust"):
                label = f"{kind}-standalone-{mode}"
                state = output / label
                state.mkdir()
                (state / "sub-store.json").write_text(json.dumps(backup, ensure_ascii=False), encoding="utf-8")
                (state / "root.json").write_text(json.dumps(root, ensure_ascii=False), encoding="utf-8")
                static_fixture(state / "frontend")
                with socket.socket() as reserved:
                    reserved.bind(("127.0.0.1", 0))
                    port = reserved.getsockname()[1]
                env = {**environment, "NODE_ENV": "production", "SUB_STORE_DATA_BASE_PATH": str(state), "SUB_STORE_BACKEND_MERGE": "",
                       "SUB_STORE_BACKEND_PREFIX": prefix, "SUB_STORE_FRONTEND_BACKEND_PATH": mount,
                       "SUB_STORE_FRONTEND_PATH": str(state / "frontend"), "SUB_STORE_FRONTEND_HOST": "127.0.0.1",
                       "SUB_STORE_FRONTEND_PORT": str(port), "SUB_STORE_CORS_ALLOWED_ORIGINS": "https://allowed.invalid"}
                env["SUB_STORE_BODY_JSON_LIMIT"] = "32kb"
                manager = oracle(args.node, env, output, report, label) if kind == "node" else listener(args.binary, env, output, report, label, image)
                with manager as url:
                    standalone[kind] = standalone_resources(f"http://127.0.0.1:{port}", url, mount, state, output, label)
                    report["phases"].append({"name": label, "requests": len(standalone[kind])})
                    if kind == "rust" and mode == "prefixed":
                        pending = request(f"http://127.0.0.1:{port}", {"path": "/share/sub/renamed?token=invalid"})
                        (output / "standalone-share-pending.json").write_text(json.dumps(pending, indent=2) + "\n", encoding="utf-8")
                        assert pending["status"] == 501, "Public share forwarding must remain unavailable pending M4"
                        report["standalone_share_proxy_unavailable"] = True
            check_json(standalone["rust"], standalone["node"])
        report["standalone_frontend_http_and_state_equal"] = True
        report["standalone_request_body_forwarding_checked"] = True
        report["standalone_raw_bulk_buffer_lifecycle_checked"] = True
        report["standalone_configurations_checked"] = ["prefixed", "unprefixed", "root", "static-only"]
        report["api_entity_validators_and_head_checked"] = True
        report["api_entity_error_and_mutation_boundaries_checked"] = True
        report["crud_route_case_suffix_and_parameter_decoding_checked"] = True
        report["m2_http_methods_and_cors_parser_precedence_checked"] = True
        report["crud_query_flags_and_ambiguous_delete_checked"] = True
        report["export_second_boundary_clock_checked"] = True
        report["api_entity_configurations_checked"] = ["merged-prefix", "merged-root", "standalone-prefixed", "standalone-unprefixed", "standalone-root"]
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
