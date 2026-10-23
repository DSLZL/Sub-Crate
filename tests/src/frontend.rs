use crate::{
    Result,
    api::Api,
    browser::Browser,
    distribution, json, process, reference, require,
    server::{self, Backend, Environment},
    upstream::Upstream,
};
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
};

pub const COMMIT: &str = "4bd7b0acbe65466b933ccea14fc61dbebe036ead";
pub const IDS: [&str; 21] = [
    "FE01-url-backend-setup",
    "FE02-create-subscription",
    "FE03-reopen-rename-subscription",
    "FE04-preview-subscription",
    "FE05-create-local-file",
    "FE06-reopen-rename-file",
    "FE07-preview-file",
    "FE08-create-count-share",
    "FE09-open-public-share",
    "FE10-reopen-update-count-share",
    "FE11-create-collection",
    "FE12-reopen-rename-collection",
    "FE13-preview-collection",
    "FE14-archive-file",
    "FE15-restore-file-archive",
    "FE16-save-log-settings",
    "FE17-read-filter-logs",
    "FE18-clear-logs",
    "FE19-download-manual-backup",
    "FE20-restore-manual-backup",
    "FE21-configured-backend-switch-and-refresh",
];
const NAME: &str = "input[placeholder=\"唯一的标识名称(请勿包含 / )\"]";
const RETENTION: &str = "input[placeholder=\"最大保存日志条数(默认: 0=关闭)\"]";
const PROFILE: &str = "({host:JSON.parse(localStorage.getItem('hostAPI')),cache:JSON.parse(localStorage.getItem('envCache'))})";
fn text(value: &Value) -> Result<&str> {
    value.as_str().ok_or_else(|| "Expected browser text".into())
}
fn open(b: &mut Browser, origin: &str, path: &str) -> Result {
    b.call(&["open", &format!("{origin}{path}")])?;
    Ok(())
}
fn done(cases: &mut Vec<Value>, number: usize, mut extra: Value) {
    extra["id"] = json!(IDS[number - 1]);
    extra["status"] = json!("browser_passed");
    cases.push(extra);
}
fn saved(state: &Path) -> Result<Value> {
    json::read(state.join("sub-store.json"))
}
fn resources(state: &Path, field: &str, name: &str) -> Result<Value> {
    let document = saved(state)?;
    let items = document[field].as_array().ok_or("Missing UI resources")?;
    require(
        items.len() == 1 && items[0]["name"] == name,
        "UI resource state mismatch",
    )?;
    Ok(items[0].clone())
}
fn submit(b: &mut Browser, path: &str) -> Result {
    b.call(&["click", ".submit-btn"])?;
    b.wait(&format!("location.pathname === {}", json!(path)))?;
    Ok(())
}
fn preview(b: &mut Browser) -> Result {
    b.call(&["click", ".compare-btn"])?;
    b.call(&["wait", ".compare-page-wrapper .processed-tr"])?;
    let value=b.evaluate("Array.from(document.querySelectorAll('.compare-page-wrapper .processed-tr'),row=>row.innerText)")?;
    let rows = value.as_array().ok_or("Missing UI preview")?;
    require(
        rows.len() == 1
            && text(&rows[0])?.contains("fixture")
            && text(&rows[0])?.contains("127.0.0.1:8388"),
        "Preview changed its parsed node",
    )?;
    b.call(&["snapshot", "-i"])?;
    Ok(())
}
fn strip_query(url: &str) -> Result<String> {
    let mut url = reqwest::Url::parse(url)?;
    url.set_query(None);
    url.set_fragment(None);
    Ok(url.into())
}
fn has(
    entries: &[Value],
    method: &str,
    url: &str,
    query: Option<(&str, &str)>,
    since: Option<jiff::Timestamp>,
) -> bool {
    entries.iter().any(|entry| {
        entry["request"]["method"] == method
            && entry["response"]["status"] == 200
            && entry["request"]["url"].as_str().is_some_and(|actual| {
                strip_query(actual).is_ok_and(|actual| actual == url)
                    && query.is_none_or(|(key, value)| {
                        reqwest::Url::parse(actual).is_ok_and(|url| {
                            url.query_pairs()
                                .filter(|(name, _)| name == key)
                                .map(|(_, value)| value)
                                .collect::<Vec<_>>()
                                == [value]
                        })
                    })
            })
            && since.is_none_or(|since| {
                entry["startedDateTime"]
                    .as_str()
                    .and_then(|s| s.parse::<jiff::Timestamp>().ok())
                    .is_some_and(|time| time >= since)
            })
    })
}
pub fn check_network(entries: &[Value], cases: &[Value], base: &str) -> Result<Value> {
    let origin = reqwest::Url::parse(base)?.origin().ascii_serialization();
    for case in cases {
        let id = text(&case["id"])?;
        let index = IDS
            .iter()
            .position(|value| *value == id)
            .ok_or("Unknown frontend case")?
            + 1;
        let required: Vec<(&str, String, u16)> = match index {
            1 => vec![("GET", "/api/utils/env".into(), 200)],
            2 => vec![
                ("POST", "/api/subs".into(), 201),
                ("GET", "/api/subs".into(), 200),
            ],
            3 => vec![("PATCH", "/api/sub/browser-fixture".into(), 200)],
            4 => vec![("POST", "/api/preview/sub".into(), 200)],
            5 => vec![
                ("POST", "/api/files".into(), 201),
                ("GET", "/api/wholeFiles".into(), 200),
            ],
            6 => vec![
                ("GET", "/api/wholeFiles".into(), 200),
                ("PATCH", "/api/file/browser-file".into(), 200),
            ],
            7 => vec![("POST", "/api/preview/file".into(), 200)],
            8 => vec![("POST", "/api/token".into(), 200)],
            9 => vec![("GET", "/share/file/browser-file-renamed".into(), 200)],
            10 => vec![
                ("GET", "/api/tokens".into(), 200),
                ("DELETE", "/api/token/browser-token".into(), 200),
                ("POST", "/api/token".into(), 200),
            ],
            11 => vec![
                ("POST", "/api/collections".into(), 201),
                ("GET", "/api/collections".into(), 200),
            ],
            12 => vec![("PATCH", "/api/collection/browser-collection".into(), 200)],
            13 => vec![("POST", "/api/preview/collection".into(), 200)],
            14 => vec![("DELETE", "/api/file/browser-file-renamed".into(), 200)],
            15 => vec![
                ("GET", "/api/archives".into(), 200),
                (
                    "POST",
                    format!("/api/archives/{}/restore", text(&case["archive_id"])?),
                    200,
                ),
            ],
            16 => vec![("PATCH", "/api/settings".into(), 200)],
            17 => vec![
                ("PATCH", "/api/file/browser-file-renamed".into(), 200),
                ("GET", "/api/logs".into(), 200),
            ],
            18 => vec![("DELETE", "/api/logs".into(), 200)],
            19 => vec![("GET", "/api/storage".into(), 200)],
            20 => vec![
                ("POST", "/api/storage".into(), 200),
                ("GET", "/api/utils/refresh".into(), 200),
                ("DELETE", "/api/sub/ui-restored-sub".into(), 200),
                ("DELETE", "/api/collection/ui-restored-col".into(), 200),
            ],
            21 => vec![],
            _ => unreachable!(),
        };
        for (method, path, status) in required {
            let url = format!(
                "{}{path}",
                if path.starts_with("/share/") {
                    &origin
                } else {
                    base.trim_end_matches('/')
                }
            );
            require(
                entries.iter().any(|entry| {
                    entry["request"]["method"] == method
                        && entry["response"]["status"] == status
                        && entry["request"]["url"].as_str().is_some_and(|actual| {
                            strip_query(actual).is_ok_and(|actual| actual == url)
                        })
                }),
                format!("{id}: missing successful {method} {path} in HAR"),
            )?;
        }
        if index == 17 {
            require(
                has(
                    entries,
                    "GET",
                    &format!("{base}/api/logs"),
                    Some(("keyword", text(&case["keyword"])?)),
                    None,
                ),
                "Log keyword missing from HAR",
            )?;
        }
        if index == 20 {
            for path in [
                "/api/sub/ui-restored-sub",
                "/api/collection/ui-restored-col",
            ] {
                require(
                    has(
                        entries,
                        "DELETE",
                        &format!("{base}{path}"),
                        Some(("mode", "permanent")),
                        None,
                    ),
                    "Permanent UI deletion missing from HAR",
                )?;
            }
        }
        if index == 21 {
            let since = text(&case["started_at"])?.parse::<jiff::Timestamp>()?;
            for base in [base, text(&case["secondary_backend_url"])?] {
                for path in ["/api/utils/env", "/api/subs", "/api/utils/refresh"] {
                    require(
                        has(entries, "GET", &format!("{base}{path}"), None, Some(since)),
                        "Backend switch did not refresh both origins after switching",
                    )?;
                }
            }
            let url = format!("{}/api/settings", text(&case["secondary_backend_url"])?);
            require(
                entries.iter().any(|entry| {
                    has(
                        std::slice::from_ref(entry),
                        "PATCH",
                        &url,
                        None,
                        Some(since),
                    ) && entry["request"]["postData"]["text"]
                        .as_str()
                        .and_then(|raw| json::parse(raw).ok())
                        == Some(json!({"appearanceSetting":case["appearance_setting"]}))
                }),
                "Secondary appearance request differs",
            )?;
        }
    }
    let mut prefixes = vec![format!("{base}/"), format!("{origin}/share/")];
    for case in cases {
        if let Some(url) = case["secondary_backend_url"].as_str() {
            prefixes.push(format!("{url}/"));
        }
    }
    Ok(json!(entries.iter().filter(|entry|entry["request"]["url"].as_str().is_some_and(|url|prefixes.iter().any(|prefix|url.starts_with(prefix)))).map(|entry|json!({"method":entry["request"]["method"],"url":entry["request"]["url"],"status":entry["response"]["status"]})).collect::<Vec<_>>()))
}

fn language(b: &mut Browser, origin: &str, report: &mut Value) -> Result {
    open(b, origin, "/my")?;
    b.call(&["wait", ".storage-language-switch .language-switch-button"])?;
    for (label, locale) in [("English", "en"), ("简体中文", "zh")] {
        b.call(&["click", ".storage-language-switch .language-switch-button"])?;
        b.call(&["wait", ".language-switch-popup"])?;
        b.popup(".language-switch-popup")?;
        b.wait("document.querySelectorAll('.Toastify__toast').length===0")?;
        let rows=b.evaluate(&format!(r#"Array.from(document.querySelectorAll('.language-switch-popup .nut-cell'),(el,index)=>{{const r=el.getBoundingClientRect();return {{index,text:el.innerText,html:el.outerHTML,rect:r.toJSON(),visible:r.width>0&&r.height>0&&getComputedStyle(el).visibility!=='hidden',inViewport:r.width>0&&r.height>0&&r.top>=0&&r.left>=0&&r.bottom<=innerHeight&&r.right<=innerWidth,uncovered:el.contains(document.elementFromPoint(r.left+r.width/2,r.top+r.height/2)),label:{}}};}})"#,json!(label)))?;
        report["language_targets"]
            .as_array_mut()
            .ok_or("Missing language report")?
            .push(rows.clone());
        let rows = rows
            .as_array()
            .ok_or("Missing language rows")?
            .iter()
            .filter(|row| {
                row["visible"] == true
                    && row["inViewport"] == true
                    && row["uncovered"] == true
                    && row["text"]
                        .as_str()
                        .is_some_and(|text| text.trim() == label)
            })
            .collect::<Vec<_>>();
        require(rows.len() == 1, "Expected unique visible language row")?;
        b.call(&[
            "find",
            "nth",
            &rows[0]["index"].to_string(),
            ".language-switch-popup .nut-cell",
            "click",
        ])?;
        b.wait(&format!(
            "localStorage.getItem('locale') === {}",
            json!(locale)
        ))?;
        b.wait("Array.from(document.querySelectorAll('.language-switch-popup,.nut-overlay')).every(el=>el.getClientRects().length===0)")?;
    }
    report["ui_locale"] = json!("zh");
    Ok(())
}

fn file_flows(b: &mut Browser, origin: &str, state: &Path, cases: &mut Vec<Value>) -> Result {
    open(b, origin, "/edit/files/UNTITLED")?;
    b.call(&["wait", NAME])?;
    b.call(&["fill", NAME, "browser-file"])?;
    b.call(&["wait", ".cm-content[contenteditable='true']"])?;
    b.call(&["click", ".cm-content[contenteditable='true']"])?;
    b.call(&["press", "Control+a"])?;
    b.call(&["press", "Backspace"])?;
    b.call(&["keyboard", "inserttext", "browser file🙂\n"])?;
    b.call(&["snapshot", "-i"])?;
    submit(b, "/files")?;
    b.call(&["wait", "--text", "browser-file"])?;
    require(
        resources(state, "files", "browser-file")?["content"] == "browser file🙂\n",
        "UI file create lost editor content",
    )?;
    done(cases, 5, json!({}));
    open(b, origin, "/edit/files/browser-file")?;
    b.call(&["wait", NAME])?;
    b.expect_value(NAME, "browser-file")?;
    b.call(&["fill", NAME, "browser-file-renamed"])?;
    b.click_name("内容", Some("tab"))?;
    b.call(&["wait", ".cm-content[contenteditable='true']"])?;
    b.wait("document.querySelector('.cm-content').innerText.includes('browser file🙂')")?;
    require(b.evaluate("Array.from(document.querySelectorAll('.cm-content .cm-line'),e=>e.textContent).join('\\n')")?=="browser file🙂\n","Editor reload lost content")?;
    submit(b, "/files")?;
    b.call(&["wait", "--text", "browser-file-renamed"])?;
    require(
        resources(state, "files", "browser-file-renamed")?["content"] == "browser file🙂\n",
        "File rename lost content",
    )?;
    done(cases, 6, json!({}));
    open(b, origin, "/edit/files/browser-file-renamed")?;
    b.call(&["wait", NAME])?;
    b.expect_value(NAME, "browser-file-renamed")?;
    b.click_name("内容", Some("tab"))?;
    b.call(&["wait", ".compare-btn"])?;
    b.call(&["click", ".compare-btn"])?;
    b.call(&["wait", ".compare-page-wrapper-overlay .cm-content"])?;
    b.wait("document.querySelector('.compare-page-wrapper-overlay .cm-content').innerText.includes('browser file🙂')")?;
    require(b.evaluate("Array.from(document.querySelectorAll('.compare-page-wrapper-overlay .cm-content .cm-line'),e=>e.textContent).join('\\n')")?=="browser file🙂\n","File preview content differs")?;
    b.call(&["snapshot", "-i"])?;
    done(cases, 7, json!({}));
    Ok(())
}
fn share_flows(b: &mut Browser, origin: &str, state: &Path, cases: &mut Vec<Value>) -> Result {
    const TOKEN: &str = "input[placeholder=\"默认后端随机 Token\"]";
    const COUNT: &str = "input[placeholder=\"请输入总次数\"]";
    open(b, origin, "/edit/shares/browser-file-renamed?type=file")?;
    b.call(&["wait", TOKEN])?;
    b.click_name("按次数", None)?;
    b.call(&["wait", COUNT])?;
    b.call(&["fill", COUNT, "2"])?;
    b.call(&["fill", TOKEN, "browser-token"])?;
    b.call(&["click", ".submit-btn"])?;
    b.wait("new URLSearchParams(location.search).get('token')==='browser-token'")?;
    let document = saved(state)?;
    require(
        document["tokens"]
            .as_array()
            .is_some_and(|tokens| tokens.len() == 1)
            && document["tokens"][0]["token"] == "browser-token"
            && document["tokens"][0]["mode"] == "count"
            && document["tokens"][0]["count"] == 2
            && document["tokens"][0]["usedCount"] == 0,
        "Share create lost count options",
    )?;
    done(cases, 8, json!({}));
    let url = format!("{origin}/share/file/browser-file-renamed?token=browser-token");
    require(
        b.evaluate(
            "document.querySelector('input[placeholder=\"保存后会在这里显示分享链接\"]').value",
        )? == url,
        "UI share URL contains wrong identity/prefix",
    )?;
    b.call(&["open", &url])?;
    b.call(&["wait", "--text", "browser file🙂"])?;
    require(
        b.evaluate("document.body.textContent")? == "browser file🙂\n"
            && saved(state)?["tokens"][0]["usedCount"] == 1,
        "Public share body or consumption differs",
    )?;
    done(cases, 9, json!({"public_share_url":url}));
    open(
        b,
        origin,
        "/edit/shares/browser-file-renamed?type=file&token=browser-token",
    )?;
    b.call(&["wait", ".editor-section-tab-list"])?;
    b.click_name("内容", Some("tab"))?;
    b.call(&["wait", COUNT])?;
    b.expect_value(COUNT, "2")?;
    b.expect_value("input[placeholder=\"请输入已用次数\"]", "1")?;
    b.call(&["fill", COUNT, "3"])?;
    b.call(&["click", ".submit-btn"])?;
    b.call(&["wait", "--text", "分享更新成功"])?;
    let document = saved(state)?;
    require(
        document["tokens"]
            .as_array()
            .is_some_and(|tokens| tokens.len() == 1)
            && document["tokens"][0]["token"] == "browser-token"
            && document["tokens"][0]["count"] == 3
            && document["tokens"][0]["usedCount"] == 1,
        "Share update lost count or duplicated token",
    )?;
    done(cases, 10, json!({}));
    Ok(())
}
fn collection_flows(b: &mut Browser, origin: &str, state: &Path, cases: &mut Vec<Value>) -> Result {
    open(b, origin, "/edit/collections/UNTITLED")?;
    b.call(&["wait", NAME])?;
    b.call(&["fill", NAME, "browser-collection"])?;
    if b.evaluate(
        "getComputedStyle(document.querySelector('.include-subs-wrapper')).display==='none'",
    )? == true
    {
        b.click_visible(".include-subs-trigger .nut-cell__title")?;
    }
    b.click_visible(".subs-checkbox-wrapper .subs-checkbox")?;
    b.expect_value(".include-subs-trigger input", "browser-renamed")?;
    b.call(&["snapshot", "-i"])?;
    submit(b, "/subs")?;
    b.call(&["wait", "--text", "browser-collection"])?;
    require(
        resources(state, "collections", "browser-collection")?["subscriptions"]
            == json!(["browser-renamed"]),
        "Collection picker lost selection",
    )?;
    done(cases, 11, json!({}));
    // shortcut: cold loads can prune references; retain list navigation until the upstream race is fixed.
    b.click_visible(".sub-item-wrapper:has(.sub-item-title[title=\"browser-collection\"]) button:has(svg[data-icon=\"pen-nib\"])")?;
    b.wait("location.pathname==='/edit/collections/browser-collection'")?;
    b.call(&["wait", NAME])?;
    b.expect_value(NAME, "browser-collection")?;
    b.call(&["fill", NAME, "browser-collection-renamed"])?;
    b.click_name("内容", Some("tab"))?;
    b.expect_value(".include-subs-trigger input", "browser-renamed")?;
    submit(b, "/subs")?;
    b.call(&["wait", "--text", "browser-collection-renamed"])?;
    require(
        resources(state, "collections", "browser-collection-renamed")?["subscriptions"]
            == json!(["browser-renamed"]),
        "Collection rename lost references",
    )?;
    done(cases, 12, json!({"navigation":"list-edit-button"}));
    b.click_visible(".sub-item-wrapper:has(.sub-item-title[title=\"browser-collection-renamed\"]) button:has(svg[data-icon=\"pen-nib\"])")?;
    b.wait("location.pathname==='/edit/collections/browser-collection-renamed'")?;
    b.call(&["wait", ".compare-btn"])?;
    b.expect_value(".include-subs-trigger input", "browser-renamed")?;
    preview(b)?;
    done(cases, 13, json!({"navigation":"list-edit-button"}));
    Ok(())
}
fn archive_flows(b: &mut Browser, origin: &str, state: &Path, cases: &mut Vec<Value>) -> Result {
    open(b, origin, "/files")?;
    b.call(&["wait", ".sub-item-actions"])?;
    b.call(&["click", ".sub-item-actions button:last-child"])?;
    b.wait("(() => {const el=document.querySelector('.nut-swipe');return el&&new DOMMatrix(el.style.transform).m41<0&&Math.abs(new DOMMatrix(getComputedStyle(el).transform).m41-new DOMMatrix(el.style.transform).m41)<0.1;})()")?;
    b.call(&["click", ".sub-item-swipe-btn.nut-button--danger"])?;
    b.call(&["wait", ".nut-dialog"])?;
    require(
        b.call(&["get", "text", ".nut-dialog__footer-cancel"])?["text"] == "归档",
        "Archive action missing",
    )?;
    b.call(&["click", ".nut-dialog__footer-cancel"])?;
    b.call(&["wait", "--text", "已归档"])?;
    let document = saved(state)?;
    require(
        document["files"] == json!([])
            && document["archives"]
                .as_array()
                .is_some_and(|entries| entries.len() == 1),
        "UI archive did not move file",
    )?;
    let entry = &document["archives"][0];
    require(
        entry["itemType"] == "file"
            && entry["name"] == "browser-file-renamed"
            && entry["snapshot"]["content"] == "browser file🙂\n",
        "UI archive snapshot changed",
    )?;
    done(cases, 14, json!({}));
    open(b, origin, "/archives")?;
    b.call(&["wait", ".archives-item-wrapper"])?;
    require(
        text(&b.evaluate("document.querySelector('.archives-item-wrapper').innerText")?)?
            .contains("browser-file-renamed"),
        "Archive absent from UI",
    )?;
    b.call(&["click", ".archives-item-actions button[title='恢复']"])?;
    b.call(&["wait", "--text", "恢复成功！"])?;
    let document = saved(state)?;
    require(
        document["archives"] == json!([]) && document["files"] == json!([entry["snapshot"]]),
        "Archive restore lost snapshot or retained archive",
    )?;
    open(b, origin, "/edit/files/browser-file-renamed")?;
    b.call(&["wait", ".editor-section-tab-list"])?;
    b.click_name("内容", Some("tab"))?;
    b.call(&["wait", ".cm-content"])?;
    b.wait("document.querySelector('.cm-content').innerText.includes('browser file🙂')")?;
    require(
        text(&b.evaluate("document.querySelector('.cm-content').innerText")?)?
            .trim_end_matches('\n')
            == "browser file🙂",
        "Restored editor content changed",
    )?;
    done(cases, 15, json!({"archive_id":entry["id"]}));
    Ok(())
}
fn settings_logs(
    b: &mut Browser,
    origin: &str,
    state: &Path,
    cases: &mut Vec<Value>,
    report: &mut Value,
) -> Result {
    open(b, origin, "/my")?;
    b.call(&["wait", ".config-card"])?;
    b.call(&["wait", "--load", "networkidle"])?;
    b.click_name("缓存配置", None)?;
    b.call(&["wait", RETENTION])?;
    b.call(&["fill", RETENTION, "100"])?;
    b.expect_value(RETENTION, "100")?;
    b.evaluate("(() => {const button=document.querySelector('.config-card .save-btn');window.__compatSaveClick=[];for(const type of ['pointerdown','pointerup','click'])document.addEventListener(type,event=>window.__compatSaveClick.push({type,trusted:event.isTrusted,inSaveButton:button.contains(event.target),x:event.clientX,y:event.clientY,target:event.target.tagName,buttonRect:button.getBoundingClientRect().toJSON()}),{capture:true,once:true});return true;})()")?;
    b.click_visible(".config-card .save-btn")?;
    let events=b.evaluate("(() => {const events=window.__compatSaveClick;delete window.__compatSaveClick;return events;})()")?;
    require(
        events.as_array().is_some_and(|events| {
            events.iter().any(|event| {
                event["type"] == "click"
                    && event["trusted"] == true
                    && event["inSaveButton"] == true
            })
        }),
        "Physical save click did not reach visible button",
    )?;
    report["settings_save_click"] = events;
    b.call(&["wait", "--text", "保存成功"])?;
    b.expect_value(RETENTION, "100")?;
    require(
        saved(state)?["settings"]["logsMaxCount"] == "100",
        "Log retention form string changed",
    )?;
    done(cases, 16, json!({}));
    open(b, origin, "/edit/files/browser-file-renamed")?;
    b.call(&["wait", ".editor-section-tab-list"])?;
    b.click_name("显示", Some("tab"))?;
    b.call(&["wait", NAME])?;
    b.expect_value(NAME, "browser-file-renamed")?;
    submit(b, "/files")?;
    require(
        saved(state)?["files"][0]["content"] == "browser file🙂\n",
        "Log-generation save lost file content",
    )?;
    open(b, origin, "/logs")?;
    b.call(&["wait", ".log-item"])?;
    b.call(&["fill", ".logs-keyword-input input", "browser-file-renamed"])?;
    b.call(&["press", "Enter"])?;
    b.wait("performance.getEntriesByType('resource').some(e=>e.name.includes('/api/logs?')&&new URL(e.name).searchParams.get('keyword')==='browser-file-renamed'&&e.responseEnd>0&&e.decodedBodySize>0)")?;
    b.wait("document.querySelectorAll('.log-item').length>0&&Array.from(document.querySelectorAll('.log-item')).every(el=>el.innerText.includes('browser-file-renamed'))")?;
    done(cases, 17, json!({"keyword":"browser-file-renamed"}));
    b.call(&["click", ".logs-clear-toggle"])?;
    b.call(&["wait", ".nut-dialog"])?;
    require(
        b.call(&["get", "text", ".nut-dialog__footer-ok"])?["text"] == "清空",
        "Log clear confirmation missing",
    )?;
    b.call(&["click", ".nut-dialog__footer-ok"])?;
    b.call(&["wait", "--text", "日志已清空"])?;
    require(
        b.evaluate("document.querySelectorAll('.log-item').length")? == 0,
        "Cleared logs remain in UI",
    )?;
    require(
        json::parse(text(
            &json::read(state.join("root.json"))?["sub-store-logs"],
        )?)? == json!([]),
        "Cleared logs were not persisted",
    )?;
    done(cases, 18, json!({}));
    Ok(())
}
fn backup_flows(
    b: &mut Browser,
    origin: &str,
    base: &str,
    state: &Path,
    cases: &mut Vec<Value>,
    report: &mut Value,
) -> Result {
    open(b, origin, "/my")?;
    b.call(&["wait", ".config-card"])?;
    b.click_name("手动管理", None)?;
    b.call(&["wait", ".profile-block .download-btn"])?;
    let before = saved(state)?;
    b.call(&["click", ".profile-block .download-btn"])?;
    b.call(&["wait", "--text", "下载成功"])?;
    json::check(&saved(state)?, &before, false)?;
    let requests = b.call(&[
        "network",
        "requests",
        "--filter",
        "/api/storage",
        "--method",
        "GET",
    ])?;
    let detail = b.call(&[
        "network",
        "request",
        text(&requests["requests"][0]["requestId"])?,
    ])?;
    require(
        detail["url"] == format!("{base}/api/storage") && detail["status"] == 200,
        "Backup response came from wrong endpoint",
    )?;
    json::check(
        &json::parse(text(&detail["responseBody"])?)?,
        &before,
        false,
    )?;
    report["backup_download_requests"] = requests;
    report["backup_download_response"] = detail;
    done(cases, 19, json!({}));
    let file = crate::compat().join("fixtures/frontend-backup.json");
    let backup = json::read(&file)?;
    report["backup_fixture_sha256"] = json!(distribution::digest(&file)?);
    let navigation = b.evaluate("performance.timeOrigin")?;
    b.call(&["upload", "input[type=\"file\"]", &file.to_string_lossy()])?;
    b.call(&["wait", "--text", "恢复成功"])?;
    b.wait(&format!("performance.timeOrigin>{navigation}"))?;
    b.call(&["wait", ".config-card"])?;
    json::check(&saved(state)?, &backup, true)?;
    json::check(
        &json::parse(text(&json::read(state.join("root.json"))?["sub-store"])?)?,
        &backup,
        false,
    )?;
    open(b, origin, "/files")?;
    b.call(&["wait", "--text", "ui-restored-file"])?;
    open(b, origin, "/subs")?;
    b.call(&["wait", "--text", "ui-restored-sub"])?;
    b.call(&["wait", "--text", "ui-restored-col"])?;
    done(cases, 20, json!({}));
    Ok(())
}
fn switch_backend(
    b: &mut Browser,
    origin: &str,
    base: &str,
    state: &Path,
    output: &Path,
    cases: &mut Vec<Value>,
) -> Result {
    let started = jiff::Timestamp::now().to_string();
    let original = b.evaluate(PROFILE)?;
    let primary = saved(state)?;
    let secondary_state = output.join("switch-backend-state");
    std::fs::create_dir(&secondary_state)?;
    let mut environment = server::environment(&secondary_state);
    server::set(
        &mut environment,
        "SUB_STORE_BACKEND_CUSTOM_NAME",
        "m5-secondary-oracle",
    );
    server::set(&mut environment, "SUB_STORE_CORS_ALLOWED_ORIGINS", origin);
    let secondary_api = Api::start(
        &Backend::Node(reference::node()),
        &environment,
        &secondary_state,
        output,
        "switch-backend",
    )?;
    let secondary_url = &secondary_api.listener.url;
    secondary_api.send(&json!({"method":"POST","path":"/api/subs","body":{"name":"m5-secondary-only","source":"local","content":"ss://YWVzLTEyOC1nY206cGFzcw==@127.0.0.1:8388#secondary"}}),201)?;
    let mut secondary_document = saved(&secondary_state)?;
    let appearance = json!({"isLeftRight":false,"isIconColor":false,"isDefaultIcon":false,"isSimpleReicon":false,"showFloatingRefreshButton":false,"istabBar":false,"istabBar2":false});
    secondary_document["settings"]["appearanceSetting"] = appearance.clone();
    open(b, origin, "/settings/api")?;
    b.call(&["wait", "--load", "networkidle"])?;
    b.call(&["wait", ".add-api-wrapper input"])?;
    let name = ".add-form-wrapper .input:nth-child(1) input";
    let url = ".add-form-wrapper .input:nth-child(2) input";
    b.call(&["fill", name, "m5-secondary"])?;
    b.expect_value(name, "m5-secondary")?;
    b.call(&["fill", url, secondary_url])?;
    b.expect_value(url, secondary_url)?;
    b.wait(&format!(
        "document.querySelector('.preview-full')?.innerText==={}",
        json!(secondary_url)
    ))?;
    b.click_visible(".add-api-wrapper .save-btn")?;
    b.wait("JSON.parse(localStorage.getItem('hostAPI')).current==='m5-secondary'")?;
    b.wait("JSON.parse(localStorage.getItem('envCache')||'null')?.data?.data?.data?.meta?.node?.env?.SUB_STORE_BACKEND_CUSTOM_NAME==='m5-secondary-oracle'")?;
    b.wait(&format!(
        "performance.getEntriesByType('resource').some(e=>e.name==={}&&e.responseEnd>0)",
        json!(format!("{secondary_url}/api/utils/refresh"))
    ))?;
    open(b, origin, "/subs")?;
    b.call(&["wait", ".sub-item-title[title=\"m5-secondary-only\"]"])?;
    require(
        b.evaluate("document.querySelector('.sub-item-title[title=\"ui-restored-sub\"]')===null")?
            == true,
        "Primary data leaked into secondary UI",
    )?;
    let secondary = b.evaluate(PROFILE)?;
    require(
        secondary["host"]["current"] == "m5-secondary",
        "Secondary profile did not survive reload",
    )?;
    let mut urls = secondary["host"]["apis"]
        .as_array()
        .ok_or("Missing saved APIs")?
        .iter()
        .map(|item| text(&item["url"]).map(str::to_owned))
        .collect::<Result<Vec<_>>>()?;
    urls.sort();
    let mut expected = vec![base.to_owned(), secondary_url.clone()];
    expected.sort();
    require(
        urls == expected
            && secondary["cache"]["data"]["data"]["data"]["meta"]
                .get("rust")
                .is_none(),
        "Saved profiles or Node metadata differ",
    )?;
    open(b, origin, "/settings/api")?;
    b.click_name(text(&original["host"]["current"])?, Some("heading"))?;
    b.wait(&format!(
        "JSON.parse(localStorage.getItem('hostAPI')).current==={}",
        original["host"]["current"]
    ))?;
    b.wait(&format!("JSON.parse(localStorage.getItem('envCache')||'null')?.expiry>{}&&JSON.parse(localStorage.getItem('envCache')).data.data.data.meta.node.env.SUB_STORE_BACKEND_CUSTOM_NAME!=='m5-secondary-oracle'",secondary["cache"]["expiry"]))?;
    b.wait(&format!(
        "performance.getEntriesByType('resource').some(e=>e.name==={}&&e.responseEnd>0)",
        json!(format!("{base}/api/utils/refresh"))
    ))?;
    open(b, origin, "/subs")?;
    b.call(&["wait", ".sub-item-title[title=\"ui-restored-sub\"]"])?;
    require(
        b.evaluate(
            "document.querySelector('.sub-item-title[title=\"m5-secondary-only\"]')===null",
        )? == true,
        "Secondary data remained after switching back",
    )?;
    let restored = b.evaluate(PROFILE)?;
    let mut expected = original["host"].clone();
    expected["apis"] = secondary["host"]["apis"].clone();
    json::check(&restored["host"], &expected, false)?;
    json::check(
        &restored["cache"]["data"]["data"]["data"],
        &original["cache"]["data"]["data"]["data"],
        false,
    )?;
    require(
        restored["cache"]["expiry"].as_f64() > original["cache"]["expiry"].as_f64(),
        "Primary environment cache did not refresh",
    )?;
    json::check(&saved(state)?, &primary, false)?;
    json::check(&saved(&secondary_state)?, &secondary_document, false)?;
    json::write_new(
        output.join("frontend-configured-profiles.json"),
        &json!({"original":original,"secondary":secondary,"restored":restored}),
    )?;
    done(
        cases,
        21,
        json!({"started_at":started,"secondary_backend_url":secondary_url,"appearance_setting":appearance}),
    );
    Ok(())
}
#[allow(clippy::too_many_arguments)] // The restart uses the same explicit listener identity and state as the UI run.
fn restart(
    b: &mut Browser,
    backend: &Backend,
    environment: &Environment,
    state: &Path,
    output: &Path,
    origin: &str,
    direct: &str,
    previous: u32,
    report: &mut Value,
) -> Result {
    let mut environment = environment.clone();
    server::set(
        &mut environment,
        "SUB_STORE_BACKEND_API_PORT",
        reqwest::Url::parse(direct)?
            .port()
            .ok_or("Missing original port")?
            .to_string(),
    );
    let mut expected =
        json!({"sub-store.json":saved(state)?,"root.json":json::read(state.join("root.json"))?});
    let mut previous = previous;
    for phase in ["restored", "deleted"] {
        let api = Api::start(
            backend,
            &environment,
            state,
            output,
            &format!("restart-{phase}"),
        )?;
        require(
            api.listener.url == direct && api.listener.process.child.id() != previous,
            "Backend restart changed identity or retained PID",
        )?;
        previous = api.listener.process.child.id();
        for file in ["sub-store.json", "root.json"] {
            json::check(&json::read(state.join(file))?, &expected[file], false)?;
        }
        open(b, origin, "/subs")?;
        b.call(&["wait", "--load", "networkidle"])?;
        if phase == "restored" {
            let feature = api.data(&json!({"path":"/api/utils/env"}))?["feature"].clone();
            let confirm = if feature["archive"] == true {
                "彻底删除"
            } else {
                "确认删除"
            };
            report["permanent_delete_confirmation"] = json!(confirm);
            for (name, field) in [
                ("ui-restored-sub", "subs"),
                ("ui-restored-col", "collections"),
            ] {
                let row = format!(".nut-swipe:has(.sub-item-title[title=\"{name}\"])");
                b.call(&["wait", &row])?;
                b.click_visible(&format!(
                    "{row} .sub-item-menu button:has(svg[data-icon=\"angles-right\"] )"
                ))?;
                b.wait(&format!("(() => {{const el=document.querySelector({});return el&&Math.abs(new DOMMatrix(el.style.transform).m41)>0&&Math.abs(new DOMMatrix(getComputedStyle(el).transform).m41-new DOMMatrix(el.style.transform).m41)<0.1;}})()",json!(row)))?;
                b.click_visible(&format!("{row} .sub-item-swipe-btn.nut-button--danger"))?;
                b.call(&["wait", ".nut-dialog"])?;
                require(
                    b.call(&["get", "text", ".nut-dialog__footer-ok"])?["text"] == confirm,
                    "Permanent delete confirmation missing",
                )?;
                b.call(&["click", ".nut-dialog__footer-ok"])?;
                b.wait(&format!("document.querySelector({})===null", json!(row)))?;
                let document = saved(state)?;
                require(document[field] == json!([]), "UI deletion did not persist")?;
                if field == "subs" {
                    require(
                        document["collections"][0]["subscriptions"] == json!([]),
                        "Deleted subscription retained collection references",
                    )?;
                }
            }
            let mut deleted = expected["sub-store.json"].clone();
            deleted["subs"] = json!([]);
            deleted["collections"] = json!([]);
            json::check(&saved(state)?, &deleted, false)?;
            expected["sub-store.json"] = deleted;
        } else {
            b.call(&["wait", "--text", "立即添加"])?;
            require(
                b.evaluate("document.querySelectorAll('.sub-item-wrapper').length")? == 0,
                "Deleted resources returned after restart",
            )?;
            open(b, origin, "/files")?;
            b.call(&["wait", "--text", "ui-restored-file"])?;
            for file in ["sub-store.json", "root.json"] {
                json::check(&json::read(state.join(file))?, &expected[file], false)?;
            }
        }
        json::write_new(
            output.join(format!("frontend-{phase}-state.json")),
            &expected,
        )?;
        report["state_restarts"]
            .as_array_mut()
            .ok_or("Missing restart report")?
            .push(json!({"phase":phase,"pid":previous,"url":direct}));
    }
    report["ui_permanent_deletion_and_restart_checked"] = json!(true);
    Ok(())
}
pub fn run(output: &Path, backend: Backend, standalone: bool) -> Result {
    let mut report = json!({"kind":"nextest-unchanged-frontend","frontend_commit":COMMIT,"full_coverage":false,"rust_parity":false,"exit_code":1,"cases":[],"language_targets":[],"state_restarts":[],"frontend_hosting":if standalone {"standalone"} else {"merged"}});
    reference::verify(output)?;
    let frontend =
        PathBuf::from(std::env::var_os("TEST_FRONTEND").ok_or("Missing pinned frontend checkout")?)
            .canonicalize()?;
    let assets = PathBuf::from(
        std::env::var_os("TEST_FRONTEND_ASSETS").ok_or("Missing extracted frontend assets")?,
    )
    .canonicalize()?;
    let expected = distribution::asset_hashes(&frontend.join("dist"))?;
    json::check(
        &json!(distribution::asset_hashes(&assets)?),
        &json!(expected),
        false,
    )?;
    require(
        assets.join("index.html").is_file(),
        "Build pinned frontend first",
    )?;
    json::write_new(output.join("frontend-assets.json"), &json!(expected))?;
    for (label, args, expected) in [
        ("frontend-commit", vec!["rev-parse", "HEAD"], COMMIT),
        ("frontend-source", vec!["diff", "--exit-code", "HEAD"], ""),
    ] {
        let actual = process::checked(
            Command::new("git").args(args).current_dir(&frontend),
            output,
            label,
            Duration::from_secs(30),
        )?;
        require(actual.stdout.trim() == expected, "Frontend source changed")?;
    }
    let state = output.join("state");
    std::fs::create_dir(&state)?;
    let mut environment = server::environment(&state);
    environment.retain(|key, _| !key.to_string_lossy().starts_with("AGENT_BROWSER_"));
    for (key, value) in [
        ("SUB_STORE_FRONTEND_BACKEND_PATH", "/backend"),
        ("SUB_STORE_BACKEND_MERGE", "true"),
    ] {
        server::set(&mut environment, key, value);
    }
    server::set(&mut environment, "SUB_STORE_FRONTEND_PATH", &assets);
    if standalone {
        let port = server::free_port()?;
        server::set(&mut environment, "SUB_STORE_BACKEND_MERGE", "");
        server::set(&mut environment, "SUB_STORE_BACKEND_PREFIX", "true");
        server::set(&mut environment, "SUB_STORE_FRONTEND_HOST", "127.0.0.1");
        server::set(
            &mut environment,
            "SUB_STORE_FRONTEND_PORT",
            port.to_string(),
        );
        server::set(
            &mut environment,
            "SUB_STORE_CORS_ALLOWED_ORIGINS",
            format!("http://127.0.0.1:{port}"),
        );
    }
    let mut browser = Browser::new(output, environment.clone())?;
    let version = process::checked(
        Command::new(&browser.executable).arg("--version"),
        output,
        "browser-version",
        Duration::from_secs(30),
    )?;
    require(
        version.stdout.trim() == "agent-browser 0.27.3",
        "Pinned agent-browser required",
    )?;
    let mut cases = Vec::new();
    let result = (|| -> Result {
        let upstream = Upstream::new(0)?;
        let api = Api::start(&backend, &environment, &state, output, "frontend")?;
        let direct = api.listener.url.clone();
        let origin = api.listener.frontend.clone();
        let base = if standalone {
            format!("{origin}/backend")
        } else {
            direct.clone()
        };
        let pid = api.listener.process.child.id();
        report["server_url"] = json!(base);
        report["server_origin"] = json!(origin);
        report["server_pid"] = json!(pid);
        report["direct_backend_url"] = json!(direct);
        browser.call(&["open", "about:blank"])?;
        report["browser_runtime"]=browser.evaluate("({userAgent:navigator.userAgent,language:navigator.language,timezone:Intl.DateTimeFormat().resolvedOptions().timeZone,viewport:{width:innerWidth,height:innerHeight}})")?;
        browser.call(&["network", "har", "start"])?;
        open(
            &mut browser,
            &origin,
            &format!("/subs?api={}", crate::fixture::quote(&base)),
        )?;
        browser.call(&["snapshot", "-i"])?;
        browser.wait("localStorage.getItem('envCache')!==null")?;
        browser.call(&["snapshot", "-i"])?;
        let metadata=browser.evaluate("({host:JSON.parse(localStorage.getItem('hostAPI')),env:JSON.parse(localStorage.getItem('envCache')).data.data.data,body:document.body.innerText})")?;
        require(
            metadata["host"]["apis"].as_array().is_some_and(|apis| {
                apis.iter()
                    .any(|api| api["url"] == base && api["name"] == metadata["host"]["current"])
            }) && metadata["env"]["backend"] == "Node"
                && metadata["env"]["version"] == "2.42.4"
                && metadata["env"]["meta"]["node"]["env"]["SUB_STORE_FRONTEND_BACKEND_PATH"]
                    == "/backend",
            "Frontend backend configuration/metadata mismatch",
        )?;
        if !matches!(backend, Backend::Node(_)) {
            json::check(
                &metadata["env"]["meta"]["rust"],
                &json!({"version":"0.0.1","milestone":"M5","complete":true}),
                false,
            )?;
        }
        report["metadata"] = metadata;
        language(&mut browser, &origin, &mut report)?;
        open(&mut browser, &origin, "/subs")?;
        done(&mut cases, 1, json!({}));
        browser.click_name("立即添加", None)?;
        browser.popup(".add-sub-popup")?;
        browser.click_name("单条订阅", Some("link"))?;
        browser.wait("location.pathname==='/edit/subs/UNTITLED'")?;
        browser.call(&["wait", NAME])?;
        browser.call(&["fill", NAME, "browser-fixture"])?;
        let url = "textarea[placeholder=\"支持多行和参数, 请点击左侧的使用说明查看用法\"]";
        browser.call(&["wait", url])?;
        browser.call(&["fill", url, &format!("{}/subscription", upstream.url())])?;
        browser.call(&["snapshot", "-i"])?;
        submit(&mut browser, "/subs")?;
        browser.call(&["wait", "--text", "browser-fixture"])?;
        require(
            resources(&state, "subs", "browser-fixture")?["url"]
                == format!("{}/subscription", upstream.url()),
            "UI subscription lost URL",
        )?;
        done(&mut cases, 2, json!({}));
        open(&mut browser, &origin, "/edit/subs/browser-fixture")?;
        browser.call(&["wait", NAME])?;
        browser.expect_value(NAME, "browser-fixture")?;
        browser.call(&["fill", NAME, "browser-renamed"])?;
        browser.call(&["snapshot", "-i"])?;
        submit(&mut browser, "/subs")?;
        browser.call(&["wait", "--text", "browser-renamed"])?;
        resources(&state, "subs", "browser-renamed")?;
        done(&mut cases, 3, json!({}));
        open(&mut browser, &origin, "/edit/subs/browser-renamed")?;
        browser.call(&["wait", ".compare-btn"])?;
        browser.expect_value(NAME, "browser-renamed")?;
        browser.call(&["snapshot", "-i"])?;
        preview(&mut browser)?;
        require(
            upstream
                .fixture
                .snapshot()
                .iter()
                .any(|item| item["method"] == "GET" && item["path"] == "/subscription"),
            "Preview did not load controlled subscription",
        )?;
        done(&mut cases, 4, json!({}));
        file_flows(&mut browser, &origin, &state, &mut cases)?;
        share_flows(&mut browser, &origin, &state, &mut cases)?;
        collection_flows(&mut browser, &origin, &state, &mut cases)?;
        archive_flows(&mut browser, &origin, &state, &mut cases)?;
        settings_logs(&mut browser, &origin, &state, &mut cases, &mut report)?;
        backup_flows(
            &mut browser,
            &origin,
            &base,
            &state,
            &mut cases,
            &mut report,
        )?;
        switch_backend(&mut browser, &origin, &base, &state, output, &mut cases)?;
        report["upstream_requests"] = json!(upstream.fixture.snapshot());
        browser.call(&[
            "screenshot",
            &output.join("frontend-page.png").to_string_lossy(),
        ])?;
        report["page_errors"] = browser.call(&["errors"])?;
        require(
            report["page_errors"]["errors"]
                .as_array()
                .is_some_and(|errors| errors.is_empty()),
            "Frontend JavaScript errors",
        )?;
        drop(api);
        restart(
            &mut browser,
            &backend,
            &environment,
            &state,
            output,
            &origin,
            &direct,
            pid,
            &mut report,
        )?;
        report["restart_page_errors"] = browser.call(&["errors"])?;
        require(
            report["restart_page_errors"]["errors"]
                .as_array()
                .is_some_and(|errors| errors.is_empty()),
            "Frontend errors after restart",
        )?;
        Ok(())
    })();
    if let Err(error) = &result {
        report["error"] = json!(error.to_string());
        browser.diagnostics("failure");
    }
    let finalize = (|| -> Result {
        browser.call(&[
            "network",
            "har",
            "stop",
            &output.join("frontend.har").to_string_lossy(),
        ])?;
        let har = json::read(output.join("frontend.har"))?;
        report["backend_requests"] = check_network(
            har["log"]["entries"]
                .as_array()
                .ok_or("Missing HAR entries")?,
            &cases,
            text(&report["server_url"])?,
        )?;
        browser.close()?;
        require(
            cases.len() == 21
                && IDS
                    .iter()
                    .all(|id| cases.iter().filter(|case| case["id"] == *id).count() == 1),
            "Missing or duplicate UI flows",
        )?;
        Ok(())
    })();
    if let Err(error) = &finalize {
        report["cleanup_error"] = json!(error.to_string());
    }
    if result.is_ok() && finalize.is_ok() {
        report["exit_code"] = json!(0);
        for case in &mut cases {
            case["status"] = json!("passed");
            case["network_verified"] = json!(true);
        }
    }
    report["cases"] = json!(cases);
    json::write_new(output.join("summary.json"), &report)?;
    result?;
    finalize
}

#[cfg(test)]
mod tests {
    use super::*;
    fn entry(method: &str, url: &str, status: u16) -> Value {
        json!({"request":{"method":method,"url":url},"response":{"status":status},"startedDateTime":"2026-10-20T00:00:02Z"})
    }
    #[test]
    fn har_requires_actual_origin_method_success_keyword_archive_and_permanent_deletion() -> Result
    {
        let base = "http://127.0.0.1:1/backend";
        let case = json!({"id":IDS[0]});
        let correct = entry("GET", &format!("{base}/api/utils/env?share=true"), 200);
        check_network(
            std::slice::from_ref(&correct),
            std::slice::from_ref(&case),
            base,
        )?;
        for changed in [
            vec![],
            vec![entry("POST", &format!("{base}/api/utils/env"), 200)],
            vec![entry(
                "GET",
                "http://127.0.0.1:2/backend/api/utils/env",
                200,
            )],
            vec![entry("GET", &format!("{base}/api/utils/env"), 500)],
        ] {
            require(
                check_network(&changed, std::slice::from_ref(&case), base).is_err(),
                "Invalid HAR accepted",
            )?;
        }
        let share = json!({"id":IDS[8]});
        check_network(
            &[entry(
                "GET",
                "http://127.0.0.1:1/share/file/browser-file-renamed?token=x",
                200,
            )],
            std::slice::from_ref(&share),
            base,
        )?;
        require(
            check_network(
                &[entry(
                    "GET",
                    &format!("{base}/share/file/browser-file-renamed?token=x"),
                    200,
                )],
                &[share],
                base,
            )
            .is_err(),
            "Admin-prefixed public share accepted",
        )?;
        let archive = vec![
            entry("GET", &format!("{base}/api/archives"), 200),
            entry("POST", &format!("{base}/api/archives/123-abc/restore"), 200),
        ];
        check_network(
            &archive,
            &[json!({"id":IDS[14],"archive_id":"123-abc"})],
            base,
        )?;
        for case in [
            json!({"id":IDS[14],"archive_id":"wrong"}),
            json!({"id":IDS[14]}),
        ] {
            require(
                check_network(&archive, &[case], base).is_err(),
                "Invalid archive ID accepted",
            )?;
        }
        let logs = vec![
            entry(
                "PATCH",
                &format!("{base}/api/file/browser-file-renamed"),
                200,
            ),
            entry(
                "GET",
                &format!("{base}/api/logs?keyword=selected&limit=20"),
                200,
            ),
        ];
        check_network(&logs, &[json!({"id":IDS[16],"keyword":"selected"})], base)?;
        require(
            check_network(&logs, &[json!({"id":IDS[16],"keyword":"wrong"})], base).is_err(),
            "Wrong log keyword accepted",
        )?;
        let mut restored = vec![
            entry("POST", &format!("{base}/api/storage"), 200),
            entry("GET", &format!("{base}/api/utils/refresh"), 200),
            entry(
                "DELETE",
                &format!("{base}/api/sub/ui-restored-sub?mode=permanent"),
                200,
            ),
            entry(
                "DELETE",
                &format!("{base}/api/collection/ui-restored-col?mode=permanent"),
                200,
            ),
        ];
        let case = json!({"id":IDS[19]});
        check_network(&restored, std::slice::from_ref(&case), base)?;
        restored[3]["request"]["url"] = json!(format!(
            "{base}/api/collection/ui-restored-col?mode=archive"
        ));
        require(
            check_network(&restored, &[case], base).is_err(),
            "Archive deletion accepted as permanent",
        )?;
        Ok(())
    }
    #[test]
    fn backend_switch_requires_new_successful_refresh_on_both_origins_and_exact_appearance()
    -> Result {
        let primary = "http://127.0.0.1:1/backend";
        let secondary = "http://127.0.0.1:2";
        let case = json!({"id":IDS[20],"secondary_backend_url":secondary,"started_at":"2026-10-20T00:00:01Z","appearance_setting":{"istabBar":false}});
        let mut entries = Vec::new();
        for base in [primary, secondary] {
            for path in ["/api/utils/env", "/api/subs", "/api/utils/refresh"] {
                entries.push(entry("GET", &format!("{base}{path}"), 200));
            }
        }
        let mut patch = entry("PATCH", &format!("{secondary}/api/settings"), 200);
        patch["request"]["postData"] =
            json!({"text":"{\"appearanceSetting\":{\"istabBar\":false}}"});
        entries.push(patch);
        check_network(&entries, std::slice::from_ref(&case), primary)?;
        for mutation in [
            "missing",
            "old",
            "failed",
            "wrong-origin",
            "missing-appearance",
            "wrong-appearance",
        ] {
            let mut changed = entries.clone();
            match mutation {
                "missing" => {
                    changed.remove(5);
                }
                "old" => changed[5]["startedDateTime"] = json!("2026-10-20T00:00:00Z"),
                "failed" => changed[5]["response"]["status"] = json!(500),
                "wrong-origin" => {
                    changed[5]["request"]["url"] = json!(format!("{primary}/api/utils/refresh"))
                }
                "missing-appearance" => {
                    changed.pop();
                }
                _ => {
                    changed[6]["request"]["postData"]["text"] =
                        json!("{\"appearanceSetting\":{\"istabBar\":true}}")
                }
            };
            require(
                check_network(&changed, std::slice::from_ref(&case), primary).is_err(),
                format!("Backend switch mutation accepted: {mutation}"),
            )?;
        }
        Ok(())
    }
}
