"""Actions-only observations of the historical delivery; not an acceptance pass."""
import copy
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import time

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "tests/compat"))
from distribution import unpack_archive, verify_payload
from runner import request, run_cases
from service import isolate_age_requests, listener
from upstream import ControlledUpstream

if os.environ.get("GITHUB_ACTIONS") != "true" or os.name != "nt":
    raise SystemExit("Run this diagnosis on a native Windows GitHub Actions runner")

root = Path(os.environ["RUNNER_TEMP"])
delivery = root / "age-historical"
archive = next(delivery.glob("*.zip"))
checksum = (delivery / "SHA256SUMS").read_text(encoding="ascii").split()[0]
assert hashlib.sha256(archive.read_bytes()).hexdigest() == checksum
unpack_archive(archive, delivery / "extracted")
payload = next((delivery / "extracted").iterdir())
manifest = json.loads((delivery / "manifest.json").read_text(encoding="utf-8"))
verify_payload(payload, manifest)
assert manifest["source_commit"] == "2f55b9da0290d5e7d90e08b100563151b1137794"
binary = payload / "sub-store-rust.exe"
cases = json.loads((Path("tests/compat/fixtures/http-cases.json")).read_text(encoding="utf-8"))
original = next(case for case in cases if case["id"] == "PC-native-age-public-and-download")
output = root / "age-timeout-evidence"
output.mkdir()
report = {"kind": "diagnostic-only", "historical_source": manifest["source_commit"],
          "binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest(), "observations": []}
environment = {key: value for key, value in os.environ.items() if not key.startswith("SUB_STORE_")}
environment["SUB_STORE_BACKEND_API_HOST"] = "127.0.0.1"
for mode in ("original", "stages", "contended-original", "contended-isolated"):
    for iteration in range(2 if mode.startswith("contended") else 4):
        label = f"{mode}-{iteration}"
        state = output / label
        state.mkdir()
        case = copy.deepcopy(original)
        if mode == "contended-isolated":
            isolate_age_requests([case])
        if mode in ("stages", "contended-original"):
            args = case["steps"][0]["body"]["process"][0]["args"]
            before = "const check=async(id,callback)=>{if($arguments.check && id!==$arguments.check)return;results[id]=await callback();passed.push(id);};"
            after = "const clock=Date.now.bind(Date); const check=async(id,callback)=>{const start=clock();console.log('AGE_STAGE',id,'start');try{results[id]=await callback();passed.push(id);}finally{console.log('AGE_STAGE',id,'milliseconds',clock()-start);}};"
            assert args["content"].count(before) == 1
            args["content"] = args["content"].replace(before, after)
        env = {**environment, "SUB_STORE_DATA_BASE_PATH": str(state)}
        with ControlledUpstream() as upstream, listener(binary, env, output, report, label) as url:
            workers = []
            try:
                if mode.startswith("contended"):
                    for _ in range(os.cpu_count() or 1):
                        workers.append(subprocess.Popen([sys.executable, "-c",
                            "import time; end=time.monotonic()+30\nwhile time.monotonic()<end: pass"]))
                    time.sleep(0.5)
                start = time.monotonic()
                results = run_cases(url, state, [case], upstream)
                observed = {"label": label, "seconds": time.monotonic() - start,
                            "status": results[0]["status"], "error": results[0].get("error"),
                            "http_steps": len(results[0]["steps"]), "load_workers": len(workers)}
                observed["health_after"] = request(url, {"path": "/api/utils/env"})["status"]
                if observed["status"] == "failed":
                    time.sleep(5)  # Keep the original request alive long enough to observe completion.
            finally:
                for worker in workers:
                    worker.terminate()
                    worker.wait(timeout=5)
            print(json.dumps(observed), flush=True)
            report["observations"].append(observed)
            (state / "case.json").write_text(json.dumps(results, ensure_ascii=False), encoding="utf-8")
        for line in (output / f"{label}.stderr.log").read_text(encoding="utf-8").splitlines():
            if "AGE_STAGE" in line:
                print(label, line, flush=True)
        (output / "summary.json").write_text(json.dumps(report, indent=2), encoding="utf-8")

assert any(item["status"] == "failed" and item["error"] == "TimeoutError: timed out"
           and item["health_after"] == 200 for item in report["observations"]
           if item["label"].startswith("contended-original")), "CPU contention did not reproduce the timeout"
assert all(item["status"] == "passed" and item["http_steps"] == 6
           for item in report["observations"] if item["label"].startswith("contended-isolated")), "Isolated requests still fail under contention"
