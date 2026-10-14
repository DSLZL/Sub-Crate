"""Exercise the unchanged built frontend against a disposable frozen Node listener."""
import argparse
import base64
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
from urllib.parse import quote, urlsplit

from reference import BACKEND, HERE, ROOT, execute, oracle
from upstream import ControlledUpstream


def check_network(entries, cases, backend_url):
    required = {
        "FE01-url-backend-setup": [("GET", "/api/utils/env", 200)],
        "FE02-create-subscription": [("POST", "/api/subs", 201), ("GET", "/api/subs", 200)],
        "FE03-reopen-rename-subscription": [("PATCH", "/api/sub/browser-fixture", 200)],
        "FE04-preview-subscription": [("POST", "/api/preview/sub", 200)],
        "FE05-create-local-file": [("POST", "/api/files", 201), ("GET", "/api/wholeFiles", 200)],
        "FE06-reopen-rename-file": [("GET", "/api/wholeFiles", 200), ("PATCH", "/api/file/browser-file", 200)],
        "FE07-preview-file": [("POST", "/api/preview/file", 200)],
        "FE08-create-count-share": [("POST", "/api/token", 200)],
        "FE09-open-public-share": [("GET", "/share/file/browser-file-renamed", 200)],
        "FE10-reopen-update-count-share": [("GET", "/api/tokens", 200), ("DELETE", "/api/token/browser-token", 200), ("POST", "/api/token", 200)],
        "FE11-create-collection": [("POST", "/api/collections", 201), ("GET", "/api/collections", 200)],
        "FE12-reopen-rename-collection": [("PATCH", "/api/collection/browser-collection", 200)],
        "FE13-preview-collection": [("POST", "/api/preview/collection", 200)],
        "FE14-archive-file": [("DELETE", "/api/file/browser-file-renamed", 200)],
        "FE15-restore-file-archive": [("GET", "/api/archives", 200), ("POST", "/api/archives/{archive_id}/restore", 200)],
    }
    observed = [(entry["request"]["method"], urlsplit(entry["request"]["url"])._replace(query="", fragment="").geturl(), entry["response"]["status"])
                for entry in entries]
    for case in cases:
        for method, path, status in required[case["id"]]:
            path = path.format(**case)
            parsed = urlsplit(backend_url)
            base = f"{parsed.scheme}://{parsed.netloc}" if path.startswith("/share/") else backend_url.rstrip("/")
            if (method, base + path, status) not in observed:
                case.update(status="failed", error=f"Missing successful {method} {path} in HAR")
                raise AssertionError(f"{case['id']}: {case['error']}")
        case.update(status="passed", network_verified=True)
    origin = urlsplit(backend_url)._replace(path="", query="", fragment="").geturl()
    return [{"method": method, "url": url, "status": status} for method, url, status in observed if url.startswith((backend_url.rstrip("/") + "/", origin + "/share/"))]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--node", required=True, type=Path)
    parser.add_argument("--frontend", required=True, type=Path, help="Unchanged pinned checkout with completed dist build")
    parser.add_argument("--browser", required=True, type=Path, help="Installed native agent-browser executable")
    args = parser.parse_args()
    node = args.node.resolve(strict=True)
    frontend = args.frontend.resolve(strict=True)
    browser = args.browser.resolve(strict=True)
    run_id = datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%S.%fZ")
    output = HERE / "evidence" / f"frontend-{run_id}"
    output.mkdir(parents=True)
    state = output / "state"
    state.mkdir()
    environment = {key: value for key, value in os.environ.items() if not key.startswith(("SUB_STORE_", "AGENT_BROWSER_"))}
    environment["PATH"] = str(node.parent) + os.pathsep + environment.get("PATH", "")
    environment.update(SUB_STORE_DATA_BASE_PATH=str(state), SUB_STORE_BACKEND_API_HOST="127.0.0.1",
                       SUB_STORE_FRONTEND_BACKEND_PATH="/backend", SUB_STORE_BACKEND_MERGE="true",
                       SUB_STORE_FRONTEND_PATH=str(frontend / "dist"), AGENT_BROWSER_DEFAULT_TIMEOUT="15000")
    session = "sub-store-rust-" + run_id.lower().replace(".", "-")
    report = {"kind": "node-frontend-baseline", "frontend_commit": "4bd7b0acbe65466b933ccea14fc61dbebe036ead",
              "backend_commit": "9cc6b5fac966993364ae33da8797f446bbf7f571", "full_coverage": False,
              "rust_parity": False, "session": session, "exit_code": 1, "cases": []}
    command_index = 0

    def command(*arguments):
        nonlocal command_index
        command_index += 1
        name = f"browser-{command_index:02d}-{arguments[0]}"
        argv = [str(browser), "--session", session, "--json", *map(str, arguments)]
        rtk = shutil.which("rtk", path=environment.get("PATH"))
        # RTK hangs when the detached Windows browser daemon inherits its stdout pipe.
        exception = "RTK unavailable on this runner" if not rtk else "Direct native browser CLI with file-backed stdout avoids Windows daemon pipe hang"
        if rtk and os.name != "nt":
            argv = [rtk, "proxy", *argv]
            exception = None
        with (output / f"{name}.stdout.log").open("w", encoding="utf-8") as stdout, (output / f"{name}.stderr.log").open("w", encoding="utf-8") as stderr:
            result = subprocess.run(argv, env=environment, stdout=stdout, stderr=stderr, timeout=45)
        (output / f"{name}.command.json").write_text(json.dumps({"argv": argv, "exit_code": result.returncode,
            "rtk_exception": exception}, indent=2) + "\n", encoding="utf-8")
        if result.returncode:
            raise RuntimeError(f"{name} failed; inspect preserved logs")
        response = json.loads((output / f"{name}.stdout.log").read_text(encoding="utf-8"))
        if not response["success"]:
            raise RuntimeError(f"{name}: {response['error']}")
        return response["data"]

    def evaluate(script):
        return command("eval", "-b", base64.b64encode(script.encode("utf-8")).decode("ascii"))["result"]

    def click_name(name, role=None):
        snapshot = command("snapshot", "-i")
        if role:
            matches = [key for key, value in snapshot["refs"].items() if value["name"] == name and value["role"] == role]
            assert len(matches) == 1, f"Expected one {role} named {name!r}; got {matches}"
            command("click", "@" + matches[0])
        else:
            command("find", "text", name, "click")

    try:
        version = execute([node, "--version"], ROOT, environment, output, "node")
        assert version.returncode == 0 and version.stdout.strip() == "v24.15.0", "Pinned Node required"
        revision = execute(["git", "rev-parse", "HEAD"], frontend, environment, output, "frontend-commit")
        assert revision.returncode == 0 and revision.stdout.strip() == report["frontend_commit"], "Frontend revision mismatch"
        clean = execute(["git", "diff", "--exit-code", "HEAD"], frontend, environment, output, "frontend-source")
        assert clean.returncode == 0, "Frontend source changed"
        assert (frontend / "dist/index.html").is_file(), "Build the pinned frontend first"
        assets = {str(path.relative_to(frontend / "dist")): hashlib.sha256(path.read_bytes()).hexdigest()
                  for path in sorted((frontend / "dist").rglob("*")) if path.is_file()}
        artifact = json.dumps(assets, sort_keys=True).encode("utf-8")
        report["frontend_artifact_sha256"] = hashlib.sha256(artifact).hexdigest()
        (output / "frontend-assets.json").write_bytes(artifact)
        bundle = execute([node, "bundle-esbuild.js"], BACKEND, environment, output, "backend-bundle")
        assert bundle.returncode == 0, "Backend bundle failed"
        with ControlledUpstream() as upstream, oracle(node, environment, output, report) as backend_url:
            command("open", "about:blank")
            command("network", "har", "start")
            command("open", report["server_origin"] + "/subs?api=" + quote(backend_url, safe=""))
            command("snapshot", "-i")
            command("wait", "--fn", "localStorage.getItem('envCache') !== null")
            command("snapshot", "-i")
            metadata = evaluate("({host:JSON.parse(localStorage.getItem('hostAPI')),env:JSON.parse(localStorage.getItem('envCache')).data.data.data,body:document.body.innerText})")
            report["metadata"] = metadata
            assert any(item["url"] == backend_url and item["name"] == metadata["host"]["current"] for item in metadata["host"]["apis"]), "UI backend setup failed"
            assert metadata["env"]["backend"] == "Node" and metadata["env"]["version"] == "2.42.4", "Metadata mismatch"
            assert metadata["env"]["meta"]["node"]["env"]["SUB_STORE_FRONTEND_BACKEND_PATH"] == "/backend", "Legacy config metadata missing"
            report["cases"].append({"id": "FE01-url-backend-setup", "source": "src/hooks/useHostAPI.ts:306; src/api/env/index.ts:25", "status": "browser_passed"})
            click_name("立即添加")
            click_name("单条订阅", "link")
            command("wait", "--fn", "location.pathname === '/edit/subs/UNTITLED'")
            command("fill", 'input[placeholder="唯一的标识名称(请勿包含 / )"]', "browser-fixture")
            command("fill", 'textarea[placeholder="支持多行和参数, 请点击左侧的使用说明查看用法"]', upstream.url + "/subscription")
            command("snapshot", "-i")
            command("click", ".submit-btn")
            command("wait", "--fn", "location.pathname === '/subs'")
            command("wait", "--text", "browser-fixture")
            stored = json.loads((state / "sub-store.json").read_text(encoding="utf-8"))
            assert len(stored["subs"]) == 1 and stored["subs"][0]["name"] == "browser-fixture" and stored["subs"][0]["url"] == upstream.url + "/subscription", "UI create did not persist its input"
            report["cases"].append({"id": "FE02-create-subscription", "source": "src/views/Sub.vue:47; src/views/SubEditor.vue:66", "status": "browser_passed"})
            command("open", report["server_origin"] + "/edit/subs/browser-fixture")
            command("wait", 'input[placeholder="唯一的标识名称(请勿包含 / )"]')
            assert command("get", "value", 'input[placeholder="唯一的标识名称(请勿包含 / )"]')["value"] == "browser-fixture", "Editor reload lost the resource name"
            command("fill", 'input[placeholder="唯一的标识名称(请勿包含 / )"]', "browser-renamed")
            command("snapshot", "-i")
            command("click", ".submit-btn")
            command("wait", "--fn", "location.pathname === '/subs'")
            command("wait", "--text", "browser-renamed")
            stored = json.loads((state / "sub-store.json").read_text(encoding="utf-8"))
            assert len(stored["subs"]) == 1 and stored["subs"][0]["name"] == "browser-renamed", "UI rename did not persist"
            report["cases"].append({"id": "FE03-reopen-rename-subscription", "source": "src/views/SubEditor.vue:66", "status": "browser_passed"})
            command("open", report["server_origin"] + "/edit/subs/browser-renamed")
            command("wait", ".compare-btn")
            command("snapshot", "-i")
            command("click", ".compare-btn")
            command("wait", ".compare-page-wrapper .processed-tr")
            preview = evaluate("Array.from(document.querySelectorAll('.compare-page-wrapper .processed-tr'), row => row.innerText)")
            assert len(preview) == 1 and "fixture" in preview[0] and "127.0.0.1:8388" in preview[0], "Subscription preview did not display the parsed fixture"
            command("snapshot", "-i")
            assert any(item["method"] == "GET" and item["path"] == "/subscription" for item in upstream.requests), "Preview did not load the controlled subscription"
            report["cases"].append({"id": "FE04-preview-subscription", "source": "src/views/SubEditor.vue:569", "status": "browser_passed"})
            command("open", report["server_origin"] + "/edit/files/UNTITLED")
            command("wait", 'input[placeholder="唯一的标识名称(请勿包含 / )"]')
            command("fill", 'input[placeholder="唯一的标识名称(请勿包含 / )"]', "browser-file")
            command("wait", ".cm-content[contenteditable='true']")
            command("click", ".cm-content[contenteditable='true']")
            command("press", "Control+a")
            command("press", "Backspace")
            command("keyboard", "inserttext", "browser file🙂\n")
            command("snapshot", "-i")
            command("click", ".submit-btn")
            command("wait", "--fn", "location.pathname === '/files'")
            command("wait", "--text", "browser-file")
            stored = json.loads((state / "sub-store.json").read_text(encoding="utf-8"))
            assert len(stored["files"]) == 1 and stored["files"][0]["name"] == "browser-file" and stored["files"][0]["content"] == "browser file🙂\n", "UI file create did not persist editor content"
            report["cases"].append({"id": "FE05-create-local-file", "source": "src/views/FileEditor.vue:63,330,1159", "status": "browser_passed"})
            command("open", report["server_origin"] + "/edit/files/browser-file")
            command("wait", 'input[placeholder="唯一的标识名称(请勿包含 / )"]')
            assert command("get", "value", 'input[placeholder="唯一的标识名称(请勿包含 / )"]')["value"] == "browser-file", "File editor reload lost the name"
            command("fill", 'input[placeholder="唯一的标识名称(请勿包含 / )"]', "browser-file-renamed")
            click_name("内容", "tab")
            command("wait", ".cm-content[contenteditable='true']")
            command("wait", "--fn", "document.querySelector('.cm-content').innerText.includes('browser file🙂')")
            assert evaluate("Array.from(document.querySelectorAll('.cm-content .cm-line'), e => e.textContent).join('\\n')") == "browser file🙂\n", "File editor reload lost content"
            command("click", ".submit-btn")
            command("wait", "--fn", "location.pathname === '/files'")
            command("wait", "--text", "browser-file-renamed")
            stored = json.loads((state / "sub-store.json").read_text(encoding="utf-8"))
            assert len(stored["files"]) == 1 and stored["files"][0]["name"] == "browser-file-renamed" and stored["files"][0]["content"] == "browser file🙂\n", "File rename did not persist"
            report["cases"].append({"id": "FE06-reopen-rename-file", "source": "src/views/FileEditor.vue:916,1164", "status": "browser_passed"})
            command("open", report["server_origin"] + "/edit/files/browser-file-renamed")
            command("wait", 'input[placeholder="唯一的标识名称(请勿包含 / )"]')
            click_name("内容", "tab")
            command("wait", ".compare-btn")
            command("click", ".compare-btn")
            command("wait", ".compare-page-wrapper-overlay .cm-content")
            command("wait", "--fn", "document.querySelector('.compare-page-wrapper-overlay .cm-content').innerText.includes('browser file🙂')")
            assert evaluate("Array.from(document.querySelectorAll('.compare-page-wrapper-overlay .cm-content .cm-line'), e => e.textContent).join('\\n')") == "browser file🙂\n", "File preview returned incorrect content"
            command("snapshot", "-i")
            report["cases"].append({"id": "FE07-preview-file", "source": "src/views/FileEditor.vue:490,1066", "status": "browser_passed"})
            command("open", report["server_origin"] + "/edit/shares/browser-file-renamed?type=file")
            command("wait", 'input[placeholder="默认后端随机 Token"]')
            click_name("按次数")
            command("wait", 'input[placeholder="请输入总次数"]')
            command("fill", 'input[placeholder="请输入总次数"]', "2")
            command("fill", 'input[placeholder="默认后端随机 Token"]', "browser-token")
            command("click", ".submit-btn")
            command("wait", "--fn", "new URLSearchParams(location.search).get('token') === 'browser-token'")
            stored = json.loads((state / "sub-store.json").read_text(encoding="utf-8"))
            assert len(stored["tokens"]) == 1 and stored["tokens"][0]["token"] == "browser-token" and stored["tokens"][0]["mode"] == "count" and stored["tokens"][0]["count"] == 2 and stored["tokens"][0]["usedCount"] == 0, "UI share create lost count options"
            report["cases"].append({"id": "FE08-create-count-share", "source": "src/views/share/ShareEditorPage.vue:94,1465", "status": "browser_passed"})
            public_url = report["server_origin"] + "/share/file/browser-file-renamed?token=browser-token"
            share_url = evaluate("document.querySelector('input[placeholder=\"保存后会在这里显示分享链接\"]').value")
            assert share_url == public_url, "UI share builder used the admin prefix or wrong identity"
            report["public_share_url"] = share_url
            command("open", share_url)
            command("wait", "--text", "browser file🙂")
            assert evaluate("document.body.textContent") == "browser file🙂\n", "Public share returned incorrect file content"
            stored = json.loads((state / "sub-store.json").read_text(encoding="utf-8"))
            assert stored["tokens"][0]["usedCount"] == 1, "Public share did not consume exactly one use"
            report["cases"].append({"id": "FE09-open-public-share", "source": "src/utils/share.ts:454,469", "status": "browser_passed"})
            command("open", report["server_origin"] + "/edit/shares/browser-file-renamed?type=file&token=browser-token")
            command("wait", ".editor-section-tab-list")
            click_name("内容", "tab")
            command("wait", 'input[placeholder="请输入总次数"]')
            assert command("get", "value", 'input[placeholder="请输入总次数"]')["value"] == "2", "Share reload lost total count"
            assert command("get", "value", 'input[placeholder="请输入已用次数"]')["value"] == "1", "Share reload lost used count"
            command("fill", 'input[placeholder="请输入总次数"]', "3")
            command("click", ".submit-btn")
            command("wait", "--text", "分享更新成功")
            stored = json.loads((state / "sub-store.json").read_text(encoding="utf-8"))
            assert len(stored["tokens"]) == 1 and stored["tokens"][0]["token"] == "browser-token" and stored["tokens"][0]["count"] == 3 and stored["tokens"][0]["usedCount"] == 1, "UI share update lost consumed count or duplicated token"
            report["cases"].append({"id": "FE10-reopen-update-count-share", "source": "src/views/share/ShareEditorPage.vue:869,1096,1465", "status": "browser_passed"})
            command("open", report["server_origin"] + "/edit/collections/UNTITLED")
            command("wait", 'input[placeholder="唯一的标识名称(请勿包含 / )"]')
            command("fill", 'input[placeholder="唯一的标识名称(请勿包含 / )"]', "browser-collection")
            command("scroll", "down", "450")
            if evaluate("getComputedStyle(document.querySelector('.include-subs-wrapper')).display === 'none'"):
                command("click", ".include-subs-trigger .nut-cell__title")
            command("wait", ".subs-checkbox-wrapper .subs-checkbox")
            command("click", ".subs-checkbox-wrapper .subs-checkbox")
            assert command("get", "value", ".include-subs-trigger input")["value"] == "browser-renamed", "Collection picker did not select the visible subscription"
            command("snapshot", "-i")
            command("click", ".submit-btn")
            command("wait", "--fn", "location.pathname === '/subs'")
            command("wait", "--text", "browser-collection")
            stored = json.loads((state / "sub-store.json").read_text(encoding="utf-8"))
            assert len(stored["collections"]) == 1 and stored["collections"][0]["name"] == "browser-collection" and stored["collections"][0]["subscriptions"] == ["browser-renamed"], "UI collection create lost its selected subscription"
            report["cases"].append({"id": "FE11-create-collection", "source": "src/views/SubEditor.vue:360,390,1369", "status": "browser_passed"})
            command("open", report["server_origin"] + "/edit/collections/browser-collection")
            command("wait", 'input[placeholder="唯一的标识名称(请勿包含 / )"]')
            assert command("get", "value", 'input[placeholder="唯一的标识名称(请勿包含 / )"]')["value"] == "browser-collection", "Collection editor reload lost the name"
            command("fill", 'input[placeholder="唯一的标识名称(请勿包含 / )"]', "browser-collection-renamed")
            click_name("内容", "tab")
            assert command("get", "value", ".include-subs-trigger input")["value"] == "browser-renamed", "Collection reload lost its selected subscription"
            command("click", ".submit-btn")
            command("wait", "--fn", "location.pathname === '/subs'")
            command("wait", "--text", "browser-collection-renamed")
            stored = json.loads((state / "sub-store.json").read_text(encoding="utf-8"))
            assert len(stored["collections"]) == 1 and stored["collections"][0]["name"] == "browser-collection-renamed" and stored["collections"][0]["subscriptions"] == ["browser-renamed"], "Collection rename lost resource references"
            report["cases"].append({"id": "FE12-reopen-rename-collection", "source": "src/views/SubEditor.vue:1063,1379", "status": "browser_passed"})
            command("open", report["server_origin"] + "/edit/collections/browser-collection-renamed")
            command("wait", ".compare-btn")
            command("wait", "--fn", "document.querySelector('.include-subs-trigger input')?.value === 'browser-renamed'")
            command("click", ".compare-btn")
            command("wait", ".compare-page-wrapper .processed-tr")
            preview = evaluate("Array.from(document.querySelectorAll('.compare-page-wrapper .processed-tr'), row => row.innerText)")
            assert len(preview) == 1 and "fixture" in preview[0] and "127.0.0.1:8388" in preview[0], "Collection preview lost its subscription's parsed node"
            report["cases"].append({"id": "FE13-preview-collection", "source": "src/views/SubEditor.vue:569,1215; src/views/CompareTable.vue:124", "status": "browser_passed"})
            command("open", report["server_origin"] + "/files")
            command("wait", ".sub-item-actions")
            command("click", ".sub-item-actions button:last-child")
            command("wait", "--fn", "(() => { const el = document.querySelector('.nut-swipe'); return el && new DOMMatrix(el.style.transform).m41 < 0 && Math.abs(new DOMMatrix(getComputedStyle(el).transform).m41 - new DOMMatrix(el.style.transform).m41) < 0.1 })()")
            command("click", ".sub-item-swipe-btn.nut-button--danger")
            command("wait", ".nut-dialog")
            assert command("get", "text", ".nut-dialog__footer-cancel")["text"] == "归档", "Archive action missing from the delete dialog"
            command("click", ".nut-dialog__footer-cancel")
            command("wait", "--text", "已归档")
            stored = json.loads((state / "sub-store.json").read_text(encoding="utf-8"))
            assert stored["files"] == [] and len(stored["archives"]) == 1, "UI archive did not move the file into its archive"
            entry = stored["archives"][0]
            assert entry["itemType"] == "file" and entry["name"] == "browser-file-renamed" and entry["snapshot"]["content"] == "browser file🙂\n", "UI archive lost its file snapshot"
            report["cases"].append({"id": "FE14-archive-file", "source": "src/components/FileListItem.vue:559,588; src/store/subs.ts:523", "status": "browser_passed"})
            command("open", report["server_origin"] + "/archives")
            command("wait", ".archives-item-wrapper")
            assert "browser-file-renamed" in evaluate("document.querySelector('.archives-item-wrapper').innerText"), "Archived file missing from the archive UI"
            command("click", ".archives-item-actions button[title='恢复']")
            command("wait", "--text", "恢复成功！")
            stored = json.loads((state / "sub-store.json").read_text(encoding="utf-8"))
            assert stored["archives"] == [] and stored["files"] == [entry["snapshot"]], "UI restore lost the file or retained the archive"
            command("open", report["server_origin"] + "/edit/files/browser-file-renamed")
            command("wait", ".editor-section-tab-list")
            click_name("内容", "tab")
            command("wait", ".cm-content")
            assert evaluate("document.querySelector('.cm-content').innerText").rstrip("\n") == "browser file🙂", "Restored file editor lost its content"
            report["cases"].append({"id": "FE15-restore-file-archive", "source": "src/views/archive/Archive.vue:837; src/store/archive.ts:73", "archive_id": entry["id"], "status": "browser_passed"})
            report["upstream_requests"] = upstream.requests
            command("screenshot", output / "frontend-page.png")
            report["page_errors"] = command("errors")
            assert not report["page_errors"]["errors"], "Frontend JavaScript errors"
            report["exit_code"] = 0
    except Exception as error:
        report["error"] = f"{type(error).__name__}: {error}"
        if command_index:
            for arguments in (("snapshot", "-i"), ("errors",), ("console",), ("screenshot", output / "failure.png")):
                try:
                    command(*arguments)
                except Exception as diagnostic_error:
                    report.setdefault("diagnostic_errors", []).append(str(diagnostic_error))
    finally:
        if command_index:
            for arguments in (("network", "har", "stop", output / "frontend.har"), ("close",)):
                try:
                    command(*arguments)
                    if arguments[:3] == ("network", "har", "stop"):
                        har = json.loads((output / "frontend.har").read_text(encoding="utf-8"))
                        report["backend_requests"] = check_network(har["log"]["entries"], report["cases"], report["server_url"])
                except Exception as error:
                    report.setdefault("cleanup_errors", []).append(str(error))
                    report["exit_code"] = 1
        (output / "summary.json").write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({"kind": report["kind"], "exit_code": report["exit_code"], "evidence": str(output)}))
    return report["exit_code"]


if __name__ == "__main__":
    if not __debug__:
        raise SystemExit("Frontend baseline requires enabled assertions")
    raise SystemExit(main())
