"""CI acceptance for the actual Rust HTTP listener, separate from the JS probe."""
import argparse
import base64
from contextlib import contextmanager
from datetime import datetime, timedelta, timezone
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

from reference import BACKEND, HERE, ROOT, execute, oracle, source_provenance
from runner import BufferedHTTPConnection, check_json, load_cases, request, run_cases


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
    prefixed = environment.get("SUB_STORE_BACKEND_MERGE") or environment.get("SUB_STORE_BACKEND_PREFIX")
    url = origin + (environment.get("SUB_STORE_FRONTEND_BACKEND_PATH", "") if prefixed else "").rstrip("/")
    name = f"m2-{os.getpid()}-{port}"
    if image:
        argv = ["docker", "run", "--rm", "--name", name, "--read-only", "--cap-drop=ALL",
                "--security-opt=no-new-privileges", "--user", f"{os.getuid()}:{os.getgid()}",
                "-p", f"127.0.0.1:{port}:3000",
                "--mount", f"type=bind,source={environment['SUB_STORE_DATA_BASE_PATH']},target=/data"]
        if environment.get("SUB_STORE_FRONTEND_PATH"):
            argv.extend(["--mount", f"type=bind,source={environment['SUB_STORE_FRONTEND_PATH']},target=/frontend,readonly"])
            if not environment.get("SUB_STORE_BACKEND_MERGE"):
                argv.extend(["-p", f"127.0.0.1:{environment['SUB_STORE_FRONTEND_PORT']}:3001"])
        for key, value in environment.items():
            if key.startswith("SUB_STORE_") or key in ("TZ", "NODE_ENV"):
                value = {"SUB_STORE_DATA_BASE_PATH": "/data", "SUB_STORE_BACKEND_API_PORT": "3000",
                         "SUB_STORE_BACKEND_API_HOST": "0.0.0.0", "SUB_STORE_FRONTEND_PATH": "/frontend",
                         "SUB_STORE_FRONTEND_HOST": "0.0.0.0", "SUB_STORE_FRONTEND_PORT": "3001"}.get(key, value)
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


def check_cases(url, state, cases, output, name):
    results = run_cases(url, state, cases)
    (output / f"{name}.json").write_text(json.dumps(results, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    assert len(results) == len(cases) and all(case["status"] == "passed" for case in results), f"{name}: {[(case['id'], case.get('error')) for case in results if case['status'] != 'passed']}"
    return results


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
    restored = json.loads((state / "sub-store.json").read_text(encoding="utf-8"))
    for key in ("subs", "collections"):
        restored[key] = [None, False, 0, "恢复🙂\r\n", [], [None, {"name": "nested"}],
                         {"0": False, "1": None, "name": "restored", "opaque": {"zero": 0}}]
    send("POST", "/api/storage", {"content": json.dumps(restored, ensure_ascii=False)})
    check_json(json.loads((state / "root.json").read_text(encoding="utf-8"))["sub-store"], json.dumps(restored, ensure_ascii=False, indent=2))
    check_json(send("GET", "/api/storage"), restored)
    return observations


def backup_edges(url, state, output, name, offset=None):
    observations = []
    local_zone = datetime.now().astimezone().tzinfo if offset is None else timezone(timedelta(minutes=offset))

    def send(method, path, body=None, status=200, filename=None):
        before = datetime.now(timezone.utc)
        response = request(url, {"method": method, "path": path, **({"body": body} if body is not None else {})})
        after = datetime.now(timezone.utc)
        stored = json.loads((state / "sub-store.json").read_text(encoding="utf-8"))
        root = json.loads((state / "root.json").read_text(encoding="utf-8"))
        with (output / f"{name}.jsonl").open("a", encoding="utf-8") as evidence:
            evidence.write(json.dumps({"method": method, "path": path, "body": body, "response": response,
                                       "before": before.isoformat(), "after": after.isoformat(),
                                       "state": stored, "root": root}, ensure_ascii=False) + "\n")
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
    for content in ("invalid backup", json.dumps({"unknown": "missing settings"})):
        result = send("POST", "/api/storage", {"content": content}, 500)
        assert result["error"]["code"] == "INVALID_BACKUP_DATA"
        assert all((state / name).read_bytes() == value for name, value in before.items()), "Invalid import changed persisted files"
    backup = json.loads((HERE / "fixtures/frontend-backup.json").read_text(encoding="utf-8"))
    legacy = {**backup, "subs": {"10": {"name": "ten"}, "2": {"name": "two", "display-name": "旧名字🙂"}},
              "collections": {"c": {"name": "legacy-col", "subscriptions": ["two"], "ua": "old"}},
              "files": {}, "artifacts": {}, "tokens": {}}
    del legacy["schemaVersion"]
    for original in (backup, legacy):
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
    prior_data = (state / "sub-store.json").read_bytes()
    send("GET", "/api/utils/refresh")
    root = json.loads((state / "root.json").read_text(encoding="utf-8"))
    assert all(root[key] == "{}" for key in ("sub-store-cached-resource", "sub-store-cached-headers-resource", "sub-store-cached-script-resource"))
    assert (state / "sub-store.json").read_bytes() == prior_data, "No-Gist refresh changed main state"
    (output / f"{name}.json").write_text(json.dumps(observations, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    return observations


def request_bodies(url, state, output, name, origin=None):
    """Exercise the production parser, durable CRUD, and the standalone forwarding path."""
    parsed = urlsplit(url)
    observations = []
    original = json.loads((state / "sub-store.json").read_text(encoding="utf-8"))
    compared_headers = ("content-type", "content-length", "etag", "content-security-policy", "x-content-type-options")

    def send(path="/api/settings", body=b"{}", media="application/json", status=200, method="PATCH", headers=None, parser_error=False):
        headers = {**({"Origin": origin} if origin else {}), **(headers or {})}
        if media is not None:
            headers["Content-Type"] = media
        before = {filename: (state / filename).read_bytes() for filename in ("sub-store.json", "root.json")}
        connection = BufferedHTTPConnection(parsed.hostname, parsed.port, timeout=5)
        try:
            connection.request(method, parsed.path.rstrip("/") + path, body=body, headers=headers)
            response = connection.getresponse()
            content = response.read()
            raw = {"method": method, "path": path, "request_headers": headers,
                   "request_body_base64": base64.b64encode(body).decode("ascii"), "status": response.status,
                   "headers": {key.lower(): value for key, value in response.getheaders()},
                   "body_base64": base64.b64encode(content).decode("ascii"),
                   "state": json.loads((state / "sub-store.json").read_text(encoding="utf-8"))}
        finally:
            connection.close()
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
    for path in ("/api/utils/env", "/", "/api/missing-parser-route"):
        send(path, b"{", status=400, method="GET")
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
             b"__proto__[polluted]=yes&parserSafe[__proto__][polluted]=yes&constructor[prototype][opaque]=keep")
    results = [send(body=body, media="application/x-www-form-urlencoded")["data"] for body in forms]
    assert results[0]["parserForm"] == {"a": {"b": "nested"}} and results[0]["parserRepeat"] == ["one", "two"]
    assert results[1]["parserSparse"] == ["a", "c"] and results[1]["parserObjects"] == [{"a": "1", "b": "2"}]
    assert results[3]["parserMalformed"] == "%E0%A4%A" and results[3]["parserPlus"] == "a b"
    assert "polluted" not in results[-1] and "__proto__" not in results[-1]
    for plural in ("subs", "collections"):
        current = json.loads((state / "sub-store.json").read_text(encoding="utf-8"))[plural]
        send("/api/" + plural, json.dumps(original[plural] + current, ensure_ascii=False).encode(), method="PUT")
    (output / f"{name}.json").write_text(json.dumps(observations, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    return observations


def api_entities(url, state, output, name, origin=None):
    """Pin send/end response boundaries on direct and forwarded business APIs."""
    parsed = urlsplit(url)
    observations = []
    compared_headers = ("content-type", "content-length", "etag", "last-modified", "content-encoding",
                        "cache-control", "vary", "content-security-policy", "x-content-type-options")

    def send(path, headers=None, method="GET", status=200, body=None, tagged=True, expected=None):
        headers = {**({"Origin": origin} if origin else {}), **(headers or {})}
        if body is not None:
            headers.setdefault("Content-Type", "application/json")
        connection = BufferedHTTPConnection(parsed.hostname, parsed.port, timeout=5)
        before = datetime.now().astimezone()
        try:
            connection.request(method, parsed.path.rstrip("/") + path, body=body, headers=headers)
            response = connection.getresponse()
            content = response.read()
            raw = {"method": method, "path": path, "request_headers": headers,
                   "request_body_base64": base64.b64encode(body or b"").decode("ascii"),
                   "status": response.status, "headers": {key.lower(): value for key, value in response.getheaders()},
                   "body_base64": base64.b64encode(content).decode("ascii"), "before": before.isoformat(),
                   "after": datetime.now().astimezone().isoformat(),
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
            assert before - timedelta(seconds=1) <= instant <= datetime.fromisoformat(raw["after"])
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
        before_request = datetime.now().astimezone()
        try:
            connection.request(method, path, body=data, headers=headers)
            response = connection.getresponse()
            content = response.read()
            raw = {"method": method, "path": path, "request_headers": headers, "request_body": body,
                   "status": response.status, "headers": {key.lower(): value for key, value in response.getheaders()},
                   "body_base64": base64.b64encode(content).decode("ascii")}
        finally:
            connection.close()
        after_request = datetime.now().astimezone()
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
    else:
        send("/api/settings", status=404)
        send("/api/settings", {"Accept": "text/html"}, expected=entry)
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
        report["request_body_restart_and_node_rollback_checked"] = True
        report["compressed_request_body_parity"] = "NOT_VERIFIED"
        report["crud_development_stack_parity"] = "NOT_VERIFIED"
        backup = json.loads((HERE / "fixtures/frontend-backup.json").read_text(encoding="utf-8"))
        root = {"opaque": {"text": "根缓存🙂\r\n", "zero": 0, "flag": False, "nil": None}, "sub-store": "previous backup"}
        for key in ("sub-store-cached-resource", "sub-store-cached-headers-resource", "sub-store-cached-script-resource"):
            root[key] = json.dumps({"fixture": {"time": 4102444800000, "data": "cached"}})
        zones = [("local", {}, None)]
        if os.name != "nt":
            zones.append(("tokyo", {"TZ": "Asia/Tokyo"}, 540))
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
                prefix += "-restart"
                manager = oracle(args.node, env, output, report, prefix) if kind == "node" else listener(args.binary, env, output, report, prefix, image)
                with manager as url:
                    check_json(json.loads(request(url, {"path": "/api/storage"})["body"]), restored[kind][-1]["state"])
                    check_json(json.loads((state / "root.json").read_text(encoding="utf-8"))["sub-store"], restored[kind][-1]["root"]["sub-store"])
            check_json(restored["rust"], restored["node"])
        report["backup_http_and_root_state_equal"] = True
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
        report["node_reads_restored_rust_data_and_root"] = True
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
            for label, document in (("invalid-legacy-record", {"settings": {}, "subs": [None]}),
                                    ("invalid-other-family", {"schemaVersion": "2.0", "settings": {}, "artifacts": [None]})):
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
        report["development_error_stack_parity"] = "NOT_VERIFIED"
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
        report["standalone_configurations_checked"] = ["prefixed", "unprefixed", "root", "static-only"]
        report["api_entity_validators_and_head_checked"] = True
        report["api_entity_error_and_mutation_boundaries_checked"] = True
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
