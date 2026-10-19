"""Real-clock server lifecycle acceptance; invoked only by the Actions service gate."""
import base64
import json
import time

from reference import HERE, ROOT, execute, oracle
from runner import request
from upstream import ControlledUpstream


def run_contracts(binary, node, environment, baseline, root, output, report, image, listener):
    calendar = execute([node, str(HERE / "cron-calendar.cjs")], ROOT, environment, output, "cron-calendar")
    assert calendar.returncode == 0, "Locked cron calendar baseline changed"
    for mode in ("artifact", "overlap", "global", "produce", "upload", "download", "mmdb", "startup-plain", "startup-base64"):
        for kind in ("node", "rust"):
            label = f"{kind}-schedule-{mode}"
            state = output / label
            state.mkdir()
            proof = []
            document = {**baseline, "settings": {"logsMaxCount": 1000}, "subs": [], "collections": [],
                        "files": [], "artifacts": [], "tokens": [], "archives": [], "modules": []}
            (state / "root.json").write_text(json.dumps({**root, "sub-store-logs": "[]"}), encoding="utf-8")
            env = {**environment, "SUB_STORE_DATA_BASE_PATH": str(state), "TZ": "Asia/Shanghai"}

            def send(step):
                response = request(url, step)
                proof.append({"request": step, "response": response})
                (output / f"{label}.json").write_text(json.dumps(proof, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
                assert response["status"] in (200, 201), (label, response)
                return json.loads(response["body"]).get("data")

            def wait(check):
                deadline = time.monotonic() + 15
                while time.monotonic() < deadline:
                    value = check()
                    if value:
                        return value
                    time.sleep(0.05)
                raise AssertionError(f"{label}: actual scheduled effect timed out")

            def messages():
                return [entry["message"] for entry in send({"path": "/api/logs?limit=1000"})["logs"]]

            def durable():
                return json.loads((state / "sub-store.json").read_text(encoding="utf-8"))

            def stored_root():
                return json.loads((state / "root.json").read_text(encoding="utf-8"))

            def file(name, script=None):
                value = {"name": name, "source": "local", "content": "fixture content", "type": "text"}
                if script:
                    value["process"] = [{"type": "Script Operator", "args": {"content": script}}]
                return value

            with ControlledUpstream() as upstream:
                if mode in ("upload", "download"):
                    document["settings"].update(gistToken="helper-token", githubApiUrl=upstream.url + "/host/gist/github")
                    env[f"SUB_STORE_BACKEND_{mode.upper()}_CRON"] = "* * * * * *"
                    if mode == "download":
                        restored = {**document, "files": [file("cron-restored")]}
                        content = base64.b64encode(json.dumps(restored).encode()).decode()
                        upstream.script_gist.github["g1"] = {"description": "Auto Generated Sub-Store Backup",
                            "public": False, "files": {"Sub-Store": {"content": content}}}
                if mode == "global":
                    env["SUB_STORE_BACKEND_SYNC_CRON"] = "* * * * * *"
                    document["files"] = [file("global-source"), file("independent-source")]
                    document["artifacts"] = [
                        {"name": "global-local", "type": "file", "source": "global-source", "sync": True, "upload": False},
                        {"name": "independent", "type": "file", "source": "independent-source", "sync": True, "upload": False,
                         "cron": "0 0 0 1 jan *"}]
                if mode == "produce":
                    env["SUB_STORE_PRODUCE_CRON"] = "* * * * * *,file,produce-source; * * * * * *,file,missing-source"
                    document["files"] = [file("produce-source", "function operator(input){$substore.write('ran','#cron-produced');return input;}")]
                if mode == "mmdb":
                    env.update(SUB_STORE_MMDB_CRON="* * * * * *",
                               SUB_STORE_MMDB_COUNTRY_URL=upstream.url + "/host/runtime-country",
                               SUB_STORE_MMDB_COUNTRY_PATH=str(state / "country.mmdb"),
                               SUB_STORE_MMDB_ASN_URL=upstream.url + "/host/runtime-asn",
                               SUB_STORE_MMDB_ASN_PATH=str(state / "asn.mmdb"))
                    upstream.runtime_files["/host/runtime-country"] = (200, (HERE / "fixtures/mmdb/GeoIP2-Country-Test.mmdb").read_bytes())
                    upstream.runtime_files["/host/runtime-asn"] = (404, b"fixture missing")
                if mode.startswith("startup-"):
                    restored = {**document, "files": [file("startup-restored")]}
                    content = json.dumps(restored, ensure_ascii=False).encode()
                    if mode == "startup-base64":
                        content = base64.b64encode(content)
                    upstream.runtime_files["/host/runtime-state"] = (200, content)
                    env.update(SUB_STORE_DATA_URL=upstream.url + "/host/runtime-state",
                               SUB_STORE_DATA_URL_POST="content.settings.startupPost={nil:null,zero:0,unicode:'恢复🙂'};")
                (state / "sub-store.json").write_text(json.dumps(document, ensure_ascii=False), encoding="utf-8")
                manager = oracle(node, env, output, report, label) if kind == "node" else listener(binary, env, output, report, label, image, host_network=bool(image))
                with manager as url:
                    if mode == "artifact":
                        source = {**file("artifact-source", "function operator(input){const n=Number($substore.read('#cron-attempt')||0)+1;$substore.write(String(n),'#cron-attempt');return input;}"),
                                  "source": "remote", "url": upstream.url + "/host/runtime-artifact"}
                        upstream.runtime_files["/host/runtime-artifact"] = (503, b"fixture scheduled failure")
                        send({"method": "POST", "path": "/api/files", "body": source})
                        send({"method": "POST", "path": "/api/artifacts", "body": {"name": "scheduled", "displayName": "定时🙂", "type": "file",
                             "source": "artifact-source", "sync": True, "upload": False, "cron": "* * * * * *"}})
                        wait(lambda: any("[ARTIFACT CRON] scheduled (定时🙂) * * * * * * error:" in message for message in messages()))
                        upstream.runtime_files["/host/runtime-artifact"] = (200, b"fixture recovered")
                        wait(lambda: durable()["artifacts"][0].get("updated"))
                        wait(lambda: any("[ARTIFACT CRON] scheduled (定时🙂) * * * * * * finished" in message for message in messages()))
                        send({"method": "PATCH", "path": "/api/artifact/scheduled", "body": {"name": "renamed", "cron": "*/2 * * * * *"}})
                        wait(lambda: any("[ARTIFACT CRON] renamed (定时🙂) */2 * * * * * finished" in message for message in messages()))
                        send({"method": "PATCH", "path": "/api/artifact/renamed", "body": {"sync": False}})
                        time.sleep(0.2)
                        before = int(stored_root()["cron-attempt"])
                        time.sleep(2.3)
                        assert int(stored_root()["cron-attempt"]) == before, "Disabled artifact kept running"
                        send({"method": "PATCH", "path": "/api/artifact/renamed", "body": {"sync": True, "cron": "* * * * * *"}})
                        wait(lambda: int(stored_root()["cron-attempt"]) > before)
                        send({"method": "DELETE", "path": "/api/artifact/renamed?mode=permanent"})
                        time.sleep(0.2)
                        before = int(stored_root()["cron-attempt"])
                        time.sleep(1.3)
                        assert int(stored_root()["cron-attempt"]) == before and durable()["artifacts"] == []
                    elif mode == "overlap":
                        code = "async function operator(input){if($substore.read('#cron-active'))throw new Error('fixture overlap');$substore.write('yes','#cron-active');await new Promise(r=>setTimeout(r,2200));$substore.write('','#cron-active');return input;}"
                        send({"method": "POST", "path": "/api/files", "body": file("slow-source", code)})
                        send({"method": "POST", "path": "/api/artifacts", "body": {"name": "slow", "type": "file", "source": "slow-source",
                             "sync": True, "upload": False, "cron": "* * * * * *"}})
                        wait(lambda: stored_root().get("cron-active"))
                        send({"method": "PATCH", "path": "/api/artifact/slow", "body": {"displayName": "refreshed"}})
                        wait(lambda: any("skipped: previous run is still running" in message for message in messages()))
                        wait(lambda: any(" finished" in message and "[ARTIFACT CRON]" in message for message in messages()))
                        assert not any("fixture overlap" in message for message in messages())
                        send({"method": "PATCH", "path": "/api/artifact/slow", "body": {"sync": False}})
                    elif mode == "global":
                        wait(lambda: durable()["artifacts"][0].get("updated"))
                        time.sleep(1.2)
                        assert "updated" not in durable()["artifacts"][1], "Global cron ran an independently scheduled artifact"
                        wait(lambda: any("[SYNC CRON] * * * * * * finished" in message for message in messages()))
                        send({"method": "PATCH", "path": "/api/artifact/global-local", "body": {"upload": True}})
                        wait(lambda: any("[SYNC CRON] * * * * * * error:" in message for message in messages()))
                        send({"method": "PATCH", "path": "/api/artifact/global-local", "body": {"upload": False}})
                        previous = durable()["artifacts"][0]["updated"]
                        wait(lambda: durable()["artifacts"][0].get("updated", 0) > previous)
                    elif mode == "produce":
                        wait(lambda: stored_root().get("cron-produced") == "ran")
                        wait(lambda: any("[PRODUCE CRON] file produce-source * * * * * * finished" in message for message in messages()))
                        wait(lambda: any("[PRODUCE CRON] file missing-source * * * * * * error:" in message for message in messages()))
                    elif mode == "upload":
                        wait(lambda: upstream.script_gist.snapshot()["github"])
                        wait(lambda: any("[UPLOAD CRON] * * * * * * finished" in message for message in messages()))
                        uploaded = next(iter(upstream.script_gist.snapshot()["github"].values()))["files"]["Sub-Store"]["content"]
                        value = json.loads(base64.b64decode(uploaded))
                        assert value["settings"]["gistToken"] == "helper-token" and type(value["settings"]["syncTime"]) is int
                        assert any(entry["method"] == "POST" and "/gists" in entry["path"] for entry in upstream.requests)
                    elif mode == "download":
                        wait(lambda: durable()["files"] and durable()["files"][0]["name"] == "cron-restored")
                        wait(lambda: any("[DOWNLOAD CRON] * * * * * * finished" in message for message in messages()))
                        assert "sub-store" in stored_root()
                    elif mode == "mmdb":
                        wait(lambda: (state / "country.mmdb").exists())
                        wait(lambda: any("download failed:" in message and "runtime-asn" in message for message in messages()))
                        wait(lambda: (state / "country.mmdb").read_bytes() == upstream.runtime_files["/host/runtime-country"][1])
                        assert not (state / "asn.mmdb").exists(), "Rejected response truncated the target file"
                        upstream.runtime_files["/host/runtime-asn"] = (200, (HERE / "fixtures/mmdb/GeoLite2-ASN-Test.mmdb").read_bytes())
                        wait(lambda: (state / "asn.mmdb").exists())
                        wait(lambda: (state / "asn.mmdb").read_bytes() == upstream.runtime_files["/host/runtime-asn"][1])
                        wait(lambda: any("[MMDB CRON] * * * * * * finished" in message for message in messages()))
                        assert all(entry["method"] == "GET" for entry in upstream.requests)
                        send({"method": "POST", "path": "/api/preview/sub?target=JSON", "body": {"name": "updated-mmdb", "source": "local",
                             "content": "ss://YWVzLTEyOC1nY206cGFzcw==@127.0.0.1:8388#fixture", "process": [{"type": "Script Operator", "args": {
                                 "content": "function operator(input){const db=new ProxyUtils.MMDB();if(db.geoip('81.2.69.160')!=='GB'||db.ipasn('1.0.0.1')!==15169)throw new Error('Refreshed databases not usable');return input;}"}}]}})
                    else:
                        wait(lambda: durable()["files"] and durable()["files"][0]["name"] == "startup-restored")
                        assert durable()["settings"]["startupPost"] == {"nil": None, "zero": 0, "unicode": "恢复🙂"}
                        saved = json.loads(stored_root()["sub-store"])
                        assert saved["settings"]["startupPost"] == durable()["settings"]["startupPost"]
                        assert len(upstream.requests) == 1 and upstream.requests[0]["path"] == "/host/runtime-state"
                    assert send({"path": "/api/settings"}) is not None, "Scheduled failure stopped HTTP"
                    proof.append({"upstream": upstream.requests, "final_data": durable(), "final_root": stored_root()})
                    (output / f"{label}.json").write_text(json.dumps(proof, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
                    report["phases"].append({"name": label, "cases": 1, "requests": len(proof) - 1, "status": "passed"})
    report["real_cron_artifact_overlap_refresh_stop_global_produce_backup_mmdb_and_startup_checked"] = True
