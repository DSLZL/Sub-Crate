"""Actions-only observations of the historical delivery; not an acceptance pass."""
import copy
import hashlib
import json
import os
from pathlib import Path
import sys
import time

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "tests/compat"))
from distribution import unpack_archive, verify_payload
from runner import run_cases
from service import listener
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
for instrumented in (False, True):
    for iteration in range(8):
        label = f"{'stages' if instrumented else 'original'}-{iteration}"
        state = output / label
        state.mkdir()
        case = copy.deepcopy(original)
        if instrumented:
            args = case["steps"][0]["body"]["process"][0]["args"]
            before = "const check=async(id,callback)=>{results[id]=await callback();passed.push(id);};"
            after = "const clock=Date.now.bind(Date); const check=async(id,callback)=>{const start=clock();console.log('AGE_STAGE',id,'start');try{results[id]=await callback();passed.push(id);}finally{console.log('AGE_STAGE',id,'milliseconds',clock()-start);}};"
            assert args["content"].count(before) == 1
            args["content"] = args["content"].replace(before, after)
        env = {**environment, "SUB_STORE_DATA_BASE_PATH": str(state)}
        with ControlledUpstream() as upstream, listener(binary, env, output, report, label) as url:
            start = time.monotonic()
            results = run_cases(url, state, [case], upstream)
            observed = {"label": label, "seconds": time.monotonic() - start,
                        "status": results[0]["status"], "error": results[0].get("error")}
            print(json.dumps(observed), flush=True)
            report["observations"].append(observed)
            (state / "case.json").write_text(json.dumps(results, ensure_ascii=False), encoding="utf-8")
        for line in (output / f"{label}.stderr.log").read_text(encoding="utf-8").splitlines():
            if "AGE_STAGE" in line:
                print(label, line, flush=True)
        (output / "summary.json").write_text(json.dumps(report, indent=2), encoding="utf-8")
