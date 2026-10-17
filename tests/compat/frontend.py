"""Exercise the unchanged built frontend against a disposable Node reference."""
import argparse
import base64
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import platform
import shutil
import socket
import subprocess
from urllib.parse import parse_qs, quote, urlsplit

from reference import BACKEND, HERE, ROOT, execute, oracle, source_provenance
from runner import check_json
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
        "FE16-save-log-settings": [("PATCH", "/api/settings", 200)],
        "FE17-read-filter-logs": [("PATCH", "/api/file/browser-file-renamed", 200), ("GET", "/api/logs", 200)],
        "FE18-clear-logs": [("DELETE", "/api/logs", 200)],
        "FE19-download-manual-backup": [("GET", "/api/storage", 200)],
        "FE20-restore-manual-backup": [("POST", "/api/storage", 200), ("GET", "/api/utils/refresh", 200)],
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
        if case["id"] == "FE17-read-filter-logs":
            assert any(entry["request"]["method"] == "GET" and entry["response"]["status"] == 200
                       and urlsplit(entry["request"]["url"])._replace(query="", fragment="").geturl() == backend_url.rstrip("/") + "/api/logs"
                       and parse_qs(urlsplit(entry["request"]["url"]).query).get("keyword") == [case["keyword"]]
                       for entry in entries), "Missing successful log search with the selected keyword in HAR"
        case.update(status="passed", network_verified=True)
    origin = urlsplit(backend_url)._replace(path="", query="", fragment="").geturl()
    return [{"method": method, "url": url, "status": status} for method, url, status in observed if url.startswith((backend_url.rstrip("/") + "/", origin + "/share/"))]


class StateSliceComplete(Exception):
    """The explicitly selected M2 browser slice finished."""


def main():
    if os.environ.get("GITHUB_ACTIONS") != "true":
        raise SystemExit("Build and execute frontend acceptance in GitHub Actions only")
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--node", required=True, type=Path)
    parser.add_argument("--frontend", required=True, type=Path, help="Unchanged pinned checkout with completed dist build")
    parser.add_argument("--browser", required=True, type=Path, help="Installed native agent-browser executable")
    parser.add_argument("--rust-binary", type=Path, help="Run the M2 CRUD/settings/export slice against this Rust service")
    parser.add_argument("--standalone", action="store_true", help="Use the separate Rust frontend port and API mount")
    args = parser.parse_args()
    if args.standalone and not args.rust_binary:
        parser.error("--standalone requires --rust-binary")
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
    if args.standalone:
        with socket.socket() as reserved:
            reserved.bind(("127.0.0.1", 0))
            frontend_port = reserved.getsockname()[1]
        frontend_origin = f"http://127.0.0.1:{frontend_port}"
        environment.update(SUB_STORE_BACKEND_MERGE="", SUB_STORE_BACKEND_PREFIX="true",
                           SUB_STORE_FRONTEND_HOST="127.0.0.1", SUB_STORE_FRONTEND_PORT=str(frontend_port),
                           SUB_STORE_CORS_ALLOWED_ORIGINS=frontend_origin)
    session = "sub-store-rust-" + run_id.lower().replace(".", "-")
    report = {"kind": "node-frontend-baseline", "frontend_commit": "4bd7b0acbe65466b933ccea14fc61dbebe036ead",
              "full_coverage": False,
              "rust_parity": False, "session": session, "os": platform.system(), "machine": platform.machine(),
              "exit_code": 1, "cases": [], "frontend_hosting": "standalone" if args.standalone else "merged"}
    if args.rust_binary:
        report["kind"] = "rust-frontend-service-state"
        report["binary_sha256"] = hashlib.sha256(args.rust_binary.read_bytes()).hexdigest()
        report["required_cases"] = ["FE01-url-backend-setup", "FE02-create-subscription", "FE03-reopen-rename-subscription",
                                    "FE11-create-collection", "FE12-reopen-rename-collection", "FE16-save-log-settings",
                                    "FE19-download-manual-backup", "FE20-restore-manual-backup"]
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

    def expect_value(selector, expected, message):
        command("wait", "--fn", f"document.querySelector({json.dumps(selector)})?.value === {json.dumps(expected)}")
        assert command("get", "value", selector)["value"] == expected, message

    def wait_popup(selector):
        command("wait", "--fn", """(() => {
            const popups = Array.from(document.querySelectorAll(""" + json.dumps(selector) + """)).filter(el => el.getClientRects().length);
            if (popups.length !== 1) return false;
            const popup = popups[0], r = popup.getBoundingClientRect();
            return r.width > 0 && r.height > 0 && r.top >= 0 && r.left >= 0 && r.bottom <= innerHeight && r.right <= innerWidth
                && popup.contains(document.elementFromPoint(r.left + r.width / 2, r.top + r.height / 2))
                && popup.getAnimations({subtree:true}).every(animation => animation.playState !== 'running');
        })()""")

    def click_in_view(selector):
        command("wait", selector)
        # The CLI's scrollintoview inherits smooth scrolling and returns before it settles.
        evaluate("(() => { document.querySelector(" + json.dumps(selector) + ").scrollIntoView({block:'center',inline:'center',behavior:'instant'}); return true })()")
        command("wait", "--fn", """(async () => {
            const el = document.querySelector(""" + json.dumps(selector) + """), r = el.getBoundingClientRect();
            await new Promise(requestAnimationFrame);
            await new Promise(requestAnimationFrame);
            const settled = el.getBoundingClientRect();
            let animationsStopped = el.getAnimations({subtree:true}).every(animation => animation.playState !== 'running');
            for (let parent = el.parentElement; parent; parent = parent.parentElement) {
                animationsStopped = animationsStopped && parent.getAnimations().every(animation => animation.playState !== 'running');
            }
            return r.width > 0 && r.height > 0 && r.top >= 0 && r.left >= 0 && r.bottom <= innerHeight && r.right <= innerWidth
                && el.contains(document.elementFromPoint(r.left + r.width / 2, r.top + r.height / 2))
                && !el.matches(':disabled, [aria-disabled="true"], .nut-button--loading')
                && document.fonts.status === 'loaded' && animationsStopped
                && ['x', 'y', 'width', 'height'].every(key => Math.abs(r[key] - settled[key]) < 0.25);
        })()""")
        command("click", selector)

    def click_name(name, role=None):
        command("wait", "--text", name)
        snapshot = command("snapshot", "-i")
        if role:
            matches = [key for key, value in snapshot["refs"].items() if value["name"] == name and value["role"] == role]
            assert len(matches) == 1, f"Expected one {role} named {name!r}; got {matches}"
            command("click", "@" + matches[0])
        else:
            command("find", "text", name, "click", "--exact")

    def collection_crud():
        command("open", report["server_origin"] + "/edit/collections/UNTITLED")
        command("wait", 'input[placeholder="唯一的标识名称(请勿包含 / )"]')
        command("fill", 'input[placeholder="唯一的标识名称(请勿包含 / )"]', "browser-collection")
        if evaluate("getComputedStyle(document.querySelector('.include-subs-wrapper')).display === 'none'"):
            click_in_view(".include-subs-trigger .nut-cell__title")
        click_in_view(".subs-checkbox-wrapper .subs-checkbox")
        expect_value(".include-subs-trigger input", "browser-renamed", "Collection picker did not select the visible subscription")
        command("snapshot", "-i")
        command("click", ".submit-btn")
        command("wait", "--fn", "location.pathname === '/subs'")
        command("wait", "--text", "browser-collection")
        stored = json.loads((state / "sub-store.json").read_text(encoding="utf-8"))
        assert len(stored["collections"]) == 1 and stored["collections"][0]["name"] == "browser-collection" and stored["collections"][0]["subscriptions"] == ["browser-renamed"], "UI collection create lost its selected subscription"
        report["cases"].append({"id": "FE11-create-collection", "source": "src/views/SubEditor.vue:360,390,1369", "status": "browser_passed"})
        # shortcut: cold loads can prune references; add direct-entry coverage after that upstream race is resolved.
        click_in_view('.sub-item-wrapper:has(.sub-item-title[title="browser-collection"]) button:has(svg[data-icon="pen-nib"])')
        command("wait", "--fn", "location.pathname === '/edit/collections/browser-collection'")
        command("wait", 'input[placeholder="唯一的标识名称(请勿包含 / )"]')
        expect_value('input[placeholder="唯一的标识名称(请勿包含 / )"]', "browser-collection", "Collection editor reload lost the name")
        command("fill", 'input[placeholder="唯一的标识名称(请勿包含 / )"]', "browser-collection-renamed")
        click_name("内容", "tab")
        expect_value(".include-subs-trigger input", "browser-renamed", "Collection reload lost its selected subscription")
        command("click", ".submit-btn")
        command("wait", "--fn", "location.pathname === '/subs'")
        command("wait", "--text", "browser-collection-renamed")
        stored = json.loads((state / "sub-store.json").read_text(encoding="utf-8"))
        assert len(stored["collections"]) == 1 and stored["collections"][0]["name"] == "browser-collection-renamed" and stored["collections"][0]["subscriptions"] == ["browser-renamed"], "Collection rename lost resource references"
        report["cases"].append({"id": "FE12-reopen-rename-collection", "source": "src/components/SubListItem.vue:954; src/views/SubEditor.vue:1063,1379", "navigation": "list-edit-button", "status": "browser_passed"})

    def save_settings():
        command("open", report["server_origin"] + "/my")
        command("wait", ".config-card")
        # Initial store loading can reset the form after its controls appear.
        command("wait", "--load", "networkidle")
        click_name("缓存配置")
        command("wait", 'input[placeholder="最大保存日志条数(默认: 0=关闭)"]')
        command("fill", 'input[placeholder="最大保存日志条数(默认: 0=关闭)"]', "100")
        expect_value('input[placeholder="最大保存日志条数(默认: 0=关闭)"]', "100", "Log retention did not reach the form before saving")
        evaluate("""(() => {
            const button = document.querySelector('.config-card .save-btn');
            window.__compatSaveClick = [];
            for (const type of ['pointerdown', 'pointerup', 'click']) {
                document.addEventListener(type, event => window.__compatSaveClick.push({
                    type, trusted: event.isTrusted, inSaveButton: button.contains(event.target),
                    x: event.clientX, y: event.clientY, target: event.target.tagName,
                    buttonRect: button.getBoundingClientRect().toJSON()
                }), {capture:true, once:true});
            }
            return true;
        })()""")
        click_in_view(".config-card .save-btn")
        report["settings_save_click"] = evaluate("(() => { const events = window.__compatSaveClick; delete window.__compatSaveClick; return events })()")
        assert any(event["type"] == "click" and event["trusted"] and event["inSaveButton"]
                   for event in report["settings_save_click"]), "Physical save click did not reach its visible button"
        command("wait", "--text", "保存成功")
        expect_value('input[placeholder="最大保存日志条数(默认: 0=关闭)"]', "100", "Saved log retention disappeared from the editor")
        stored = json.loads((state / "sub-store.json").read_text(encoding="utf-8"))
        assert stored["settings"]["logsMaxCount"] == "100", "UI settings did not preserve the form's string log retention"
        report["cases"].append({"id": "FE16-save-log-settings", "source": "src/views/My.vue:989,1002; src/store/settings.ts", "status": "browser_passed"})

    def download_backup():
        command("open", report["server_origin"] + "/my")
        command("wait", ".config-card")
        click_name("手动管理")
        command("wait", ".profile-block .download-btn")
        stored = json.loads((state / "sub-store.json").read_text(encoding="utf-8"))
        command("click", ".profile-block .download-btn")
        command("wait", "--text", "下载成功")
        assert json.loads((state / "sub-store.json").read_text(encoding="utf-8")) == stored, "Manual download changed backend state"
        report["backup_download_requests"] = command("network", "requests", "--filter", "/api/storage", "--method", "GET")
        report["backup_download_response"] = command("network", "request", report["backup_download_requests"]["requests"][0]["requestId"])
        exported = report["backup_download_response"]
        assert exported["url"] == backend_url + "/api/storage" and exported["status"] == 200, "Backup response came from another endpoint"
        check_json(json.loads(exported["responseBody"]), stored)
        report["cases"].append({"id": "FE19-download-manual-backup", "source": "src/views/My.vue:1234; src/api/settings/index.ts", "status": "browser_passed"})

    def restore_backup():
        backup_file = HERE / "fixtures/frontend-backup.json"
        report["backup_fixture_sha256"] = hashlib.sha256(backup_file.read_bytes()).hexdigest()
        backup = json.loads(backup_file.read_text(encoding="utf-8"))
        navigation = evaluate("performance.timeOrigin")
        command("upload", 'input[type="file"]', backup_file)
        command("wait", "--text", "恢复成功")
        command("wait", "--fn", f"performance.timeOrigin > {navigation}")
        command("wait", ".config-card")
        check_json(json.loads((state / "sub-store.json").read_text(encoding="utf-8")), backup, subset=True)
        root = json.loads((state / "root.json").read_text(encoding="utf-8"))
        check_json(json.loads(root["sub-store"]), backup)
        command("open", report["server_origin"] + "/files")
        command("wait", "--text", "ui-restored-file")
        command("open", report["server_origin"] + "/subs")
        command("wait", "--text", "ui-restored-sub")
        command("wait", "--text", "ui-restored-col")
        report["cases"].append({"id": "FE20-restore-manual-backup", "source": "src/views/My.vue:1253,1262; src/utils/initApp.ts:36,76", "status": "browser_passed"})

    try:
        report.update(source_provenance(environment, output))
        version = execute([node, "--version"], ROOT, environment, output, "node")
        assert version.returncode == 0 and version.stdout.strip() == "v24.16.0", "Pinned Node required"
        browser_version = execute([browser, "--version"], ROOT, environment, output, "browser-version")
        assert browser_version.returncode == 0 and browser_version.stdout.strip() == "agent-browser 0.27.3", "Pinned agent-browser required"
        report["browser_version"] = browser_version.stdout.strip()
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
        if args.rust_binary:
            from service import listener
            server = listener(args.rust_binary.resolve(strict=True), environment, output, report)
        else:
            bundle = execute([node, "bundle-esbuild.js"], BACKEND, environment, output, "backend-bundle")
            assert bundle.returncode == 0, "Backend bundle failed"
            server = oracle(node, environment, output, report)
        with ControlledUpstream() as upstream, server as backend_url:
            if args.standalone:
                report["direct_backend_url"] = backend_url
                backend_url = frontend_origin + "/backend"
                report.update(server_origin=frontend_origin, server_url=backend_url)
            command("open", "about:blank")
            report["browser_runtime"] = evaluate("({userAgent:navigator.userAgent,language:navigator.language,timezone:Intl.DateTimeFormat().resolvedOptions().timeZone,viewport:{width:innerWidth,height:innerHeight}})")
            command("network", "har", "start")
            command("open", report["server_origin"] + "/subs?api=" + quote(backend_url, safe=""))
            command("snapshot", "-i")
            command("wait", "--fn", "localStorage.getItem('envCache') !== null")
            command("snapshot", "-i")
            metadata = evaluate("({host:JSON.parse(localStorage.getItem('hostAPI')),env:JSON.parse(localStorage.getItem('envCache')).data.data.data,body:document.body.innerText})")
            report["metadata"] = metadata
            assert any(item["url"] == backend_url and item["name"] == metadata["host"]["current"] for item in metadata["host"]["apis"]), "UI backend setup failed"
            assert metadata["env"]["backend"] == "Node" and metadata["env"]["version"] == ("0.1.0" if args.rust_binary else "2.42.4"), "Metadata mismatch"
            if args.rust_binary:
                assert metadata["env"]["meta"]["rust"]["complete"] is False, "Partial Rust service must not claim final parity"
            assert metadata["env"]["meta"]["node"]["env"]["SUB_STORE_FRONTEND_BACKEND_PATH"] == "/backend", "Legacy config metadata missing"
            # CI's browser language differs from the Windows baseline; use the real language control.
            command("open", report["server_origin"] + "/my")
            command("wait", ".storage-language-switch .language-switch-button")
            for label, locale in (("English", "en"), ("简体中文", "zh")):
                command("click", ".storage-language-switch .language-switch-button")
                command("wait", ".language-switch-popup")
                wait_popup(".language-switch-popup")
                command("wait", "--fn", "document.querySelectorAll('.Toastify__toast').length === 0")
                targets = evaluate("""(() => {
                    const describe = el => { const r = el.getBoundingClientRect(); return {
                        text:el.innerText, html:el.outerHTML, rect:r.toJSON(),
                        visible:r.width > 0 && r.height > 0 && getComputedStyle(el).visibility !== 'hidden',
                        inViewport:r.width > 0 && r.height > 0 && r.top >= 0 && r.left >= 0 && r.bottom <= innerHeight && r.right <= innerWidth,
                        uncovered:el.contains(document.elementFromPoint(r.left + r.width / 2, r.top + r.height / 2))} };
                    const leaf = Array.from(document.querySelectorAll('*')).find(el => el.children.length === 0 && el.textContent.trim() === """ + json.dumps(label) + """);
                    return {label:""" + json.dumps(label) + """, firstTextMatch:leaf ? describe(leaf) : null,
                        rows:Array.from(document.querySelectorAll('.language-switch-popup .nut-cell'), (el,index) => ({index,...describe(el)}))};
                })()""")
                report.setdefault("language_targets", []).append(targets)
                rows = [row for row in targets["rows"] if row["visible"] and row["inViewport"] and row["uncovered"] and row["text"].strip() == label]
                assert len(rows) == 1, f"Expected one visible language row for {label!r}"
                command("find", "nth", rows[0]["index"], ".language-switch-popup .nut-cell", "click")
                command("wait", "--fn", f"localStorage.getItem('locale') === '{locale}'")
                command("wait", "--fn", "Array.from(document.querySelectorAll('.language-switch-popup,.nut-overlay')).every(el => el.getClientRects().length === 0)")
            report["ui_locale"] = "zh"
            command("open", report["server_origin"] + "/subs")
            report["cases"].append({"id": "FE01-url-backend-setup", "source": "src/hooks/useHostAPI.ts:306; src/api/env/index.ts:25", "status": "browser_passed"})
            click_name("立即添加")
            wait_popup(".add-sub-popup")
            click_name("单条订阅", "link")
            command("wait", "--fn", "location.pathname === '/edit/subs/UNTITLED'")
            command("wait", 'input[placeholder="唯一的标识名称(请勿包含 / )"]')
            command("fill", 'input[placeholder="唯一的标识名称(请勿包含 / )"]', "browser-fixture")
            command("wait", 'textarea[placeholder="支持多行和参数, 请点击左侧的使用说明查看用法"]')
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
            expect_value('input[placeholder="唯一的标识名称(请勿包含 / )"]', "browser-fixture", "Editor reload lost the resource name")
            command("fill", 'input[placeholder="唯一的标识名称(请勿包含 / )"]', "browser-renamed")
            command("snapshot", "-i")
            command("click", ".submit-btn")
            command("wait", "--fn", "location.pathname === '/subs'")
            command("wait", "--text", "browser-renamed")
            stored = json.loads((state / "sub-store.json").read_text(encoding="utf-8"))
            assert len(stored["subs"]) == 1 and stored["subs"][0]["name"] == "browser-renamed", "UI rename did not persist"
            report["cases"].append({"id": "FE03-reopen-rename-subscription", "source": "src/views/SubEditor.vue:66", "status": "browser_passed"})
            if args.rust_binary:
                collection_crud()
                save_settings()
                download_backup()
                restore_backup()
                command("screenshot", output / "frontend-page.png")
                report["page_errors"] = command("errors")
                assert not report["page_errors"]["errors"], "Frontend JavaScript errors"
                assert [case["id"] for case in report["cases"]] == report["required_cases"], "Incomplete M2 browser slice"
                raise StateSliceComplete
            command("open", report["server_origin"] + "/edit/subs/browser-renamed")
            command("wait", ".compare-btn")
            expect_value('input[placeholder="唯一的标识名称(请勿包含 / )"]', "browser-renamed", "Preview editor did not load the subscription")
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
            expect_value('input[placeholder="唯一的标识名称(请勿包含 / )"]', "browser-file", "File editor reload lost the name")
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
            expect_value('input[placeholder="唯一的标识名称(请勿包含 / )"]', "browser-file-renamed", "Preview editor did not load the file")
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
            expect_value('input[placeholder="请输入总次数"]', "2", "Share reload lost total count")
            expect_value('input[placeholder="请输入已用次数"]', "1", "Share reload lost used count")
            command("fill", 'input[placeholder="请输入总次数"]', "3")
            command("click", ".submit-btn")
            command("wait", "--text", "分享更新成功")
            stored = json.loads((state / "sub-store.json").read_text(encoding="utf-8"))
            assert len(stored["tokens"]) == 1 and stored["tokens"][0]["token"] == "browser-token" and stored["tokens"][0]["count"] == 3 and stored["tokens"][0]["usedCount"] == 1, "UI share update lost consumed count or duplicated token"
            report["cases"].append({"id": "FE10-reopen-update-count-share", "source": "src/views/share/ShareEditorPage.vue:869,1096,1465", "status": "browser_passed"})
            collection_crud()
            click_in_view('.sub-item-wrapper:has(.sub-item-title[title="browser-collection-renamed"]) button:has(svg[data-icon="pen-nib"])')
            command("wait", "--fn", "location.pathname === '/edit/collections/browser-collection-renamed'")
            command("wait", ".compare-btn")
            expect_value(".include-subs-trigger input", "browser-renamed", "Collection preview did not load its selected subscription")
            command("click", ".compare-btn")
            command("wait", ".compare-page-wrapper .processed-tr")
            preview = evaluate("Array.from(document.querySelectorAll('.compare-page-wrapper .processed-tr'), row => row.innerText)")
            assert len(preview) == 1 and "fixture" in preview[0] and "127.0.0.1:8388" in preview[0], "Collection preview lost its subscription's parsed node"
            report["cases"].append({"id": "FE13-preview-collection", "source": "src/components/SubListItem.vue:954; src/views/SubEditor.vue:569,1215; src/views/CompareTable.vue:124", "navigation": "list-edit-button", "status": "browser_passed"})
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
            command("wait", "--fn", "document.querySelector('.cm-content').innerText.includes('browser file🙂')")
            assert evaluate("document.querySelector('.cm-content').innerText").rstrip("\n") == "browser file🙂", "Restored file editor lost its content"
            report["cases"].append({"id": "FE15-restore-file-archive", "source": "src/views/archive/Archive.vue:837; src/store/archive.ts:73", "archive_id": entry["id"], "status": "browser_passed"})
            save_settings()
            command("open", report["server_origin"] + "/edit/files/browser-file-renamed")
            command("wait", ".editor-section-tab-list")
            click_name("显示", "tab")
            command("wait", 'input[placeholder="唯一的标识名称(请勿包含 / )"]')
            expect_value('input[placeholder="唯一的标识名称(请勿包含 / )"]', "browser-file-renamed", "Restored file did not load before saving")
            command("click", ".submit-btn")
            command("wait", "--fn", "location.pathname === '/files'")
            stored = json.loads((state / "sub-store.json").read_text(encoding="utf-8"))
            assert stored["files"][0]["content"] == "browser file🙂\n", "Saving restored file for log generation lost its content"
            command("open", report["server_origin"] + "/logs")
            command("wait", ".log-item")
            command("fill", ".logs-keyword-input input", "browser-file-renamed")
            command("press", "Enter")
            command("wait", "--fn", "document.querySelectorAll('.log-item').length > 0 && Array.from(document.querySelectorAll('.log-item')).every(el => el.innerText.includes('browser-file-renamed'))")
            report["cases"].append({"id": "FE17-read-filter-logs", "source": "src/components/LogsPanel.vue:549,566,613; src/views/FileEditor.vue:1164", "keyword": "browser-file-renamed", "status": "browser_passed"})
            command("click", ".logs-clear-toggle")
            command("wait", ".nut-dialog")
            assert command("get", "text", ".nut-dialog__footer-ok")["text"] == "清空", "Log clear confirmation missing"
            command("click", ".nut-dialog__footer-ok")
            command("wait", "--text", "日志已清空")
            assert evaluate("document.querySelectorAll('.log-item').length") == 0, "Cleared logs remained in the UI"
            root = json.loads((state / "root.json").read_text(encoding="utf-8"))
            assert json.loads(root["sub-store-logs"]) == [], "UI clear did not persist empty backend logs"
            report["cases"].append({"id": "FE18-clear-logs", "source": "src/components/LogsPanel.vue:639,652", "status": "browser_passed"})
            download_backup()
            restore_backup()
            report["upstream_requests"] = upstream.requests
            command("screenshot", output / "frontend-page.png")
            report["page_errors"] = command("errors")
            assert not report["page_errors"]["errors"], "Frontend JavaScript errors"
            report["exit_code"] = 0
    except StateSliceComplete:
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
