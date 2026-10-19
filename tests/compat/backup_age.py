"""Existing age backup wire format through actual Node and Rust service owners."""
import json

from reference import HERE, oracle
from runner import check_json, request
from upstream import ControlledUpstream


def run_contracts(binary, node, environment, baseline, root, output, report, image, listener):
    fixtures = json.loads((HERE / "fixtures/http-cases.json").read_text(encoding="utf-8"))
    key_case = next(case for case in fixtures if case["id"] == "Z01-age-rest-key-generation-and-validation")
    secret = key_case["steps"][0]["body"]["age-secret-key"]
    errors = {}
    with ControlledUpstream() as upstream:
        for kind, phase in (("node", "upload"), ("rust", "cross"), ("node", "cross")):
            label = f"{kind}-backup-age-{phase}"
            state = output / label
            state.mkdir()
            document = {**baseline, "settings": {"logsMaxCount": 1000, "gistToken": "helper-token",
                "githubApiUrl": upstream.url + "/host/gist/github", "gistUpload": "age", "age-secret-key": secret},
                "files": [{"name": "age-backup-original", "source": "local", "content": "保留🙂\r\n"}],
                "artifacts": [], "tokens": [], "archives": [], "modules": [],
                "ageBackup": {"nil": None, "zero": 0, "flag": False, "script": "require('node:fs')\r\n// stored only"}}
            (state / "sub-store.json").write_text(json.dumps(document, ensure_ascii=False), encoding="utf-8")
            (state / "root.json").write_text(json.dumps({**root, "sub-store-logs": "[]"}), encoding="utf-8")
            env = {**environment, "SUB_STORE_DATA_BASE_PATH": str(state)}
            proof = []

            def send(step, status=200):
                response = request(url, step)
                proof.append({"request": step, "response": response})
                (output / f"{label}.json").write_text(json.dumps(proof, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
                assert response["status"] == status, (label, step, response)
                return json.loads(response["body"])

            def durable():
                return json.loads((state / "sub-store.json").read_text(encoding="utf-8"))

            manager = oracle(node, env, output, report, label) if kind == "node" else listener(binary, env, output, report, label, image, host_network=bool(image))
            with manager as url:
                if phase == "cross":
                    send({"path": "/api/utils/backup?action=download"})
                    restored = durable()
                    assert restored["ageBackup"] == document["ageBackup"]
                    assert restored["subs"] == baseline["subs"], "Stored excluded script text changed in encrypted backup"
                    assert restored["files"][0] == document["files"][0]
                    assert restored["settings"]["age-secret-key"] == secret
                    if kind == "node":
                        assert restored["files"][1] == {"name": "from-rust", "source": "local", "content": "Rust 备份🙂"}
                if kind == "rust":
                    send({"method": "POST", "path": "/api/files", "body": {"name": "from-rust", "source": "local", "content": "Rust 备份🙂"}}, 201)
                if phase == "upload" or kind == "rust":
                    send({"path": "/api/utils/backup?action=upload"})
                    backup = next(value for value in upstream.script_gist.github.values() if value["description"] == "Auto Generated Sub-Store Backup")
                    ciphertext = backup["files"]["Sub-Store"]["content"]
                    assert ciphertext.startswith("-----BEGIN AGE ENCRYPTED FILE-----")
                    assert secret not in ciphertext and "保留🙂" not in ciphertext
                    before = (state / "sub-store.json").read_bytes()
                    root_before = json.loads((state / "root.json").read_text(encoding="utf-8"))
                    backup["files"]["Sub-Store"]["content"] = "-----BEGIN AGE ENCRYPTED FILE-----\ncorrupted\n-----END AGE ENCRYPTED FILE-----"
                    try:
                        errors[kind] = send({"path": "/api/utils/backup?action=download"}, 500)
                        assert (state / "sub-store.json").read_bytes() == before, "Failed decryption changed stored data"
                        root_after = json.loads((state / "root.json").read_text(encoding="utf-8"))
                        check_json({key: value for key, value in root_after.items() if key != "sub-store-logs"},
                                   {key: value for key, value in root_before.items() if key != "sub-store-logs"})
                    finally:
                        backup["files"]["Sub-Store"]["content"] = ciphertext
                    send({"path": "/api/logs?limit=10"})
                proof.append({"final_data": durable(), "final_root": json.loads((state / "root.json").read_text(encoding="utf-8")),
                              "upstream": upstream.requests, "provider": upstream.script_gist.snapshot()})
                (output / f"{label}.json").write_text(json.dumps(proof, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
                report["phases"].append({"name": label, "cases": 1, "requests": len(proof) - 1, "status": "passed"})
    check_json(errors["rust"], errors["node"])
    report["native_gist_age_backup_cross_decrypt_validation_rollback_and_recovery_checked"] = True
