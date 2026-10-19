use crate::{api, json as codec, resources::{self, Error}, settings::job, state::{self, Store, truthy}};
use hyper::Method;
use serde_json::{Value, json};
use std::{sync::{Arc, Mutex}, time::Duration};

const KEY: &str = "Auto Generated Sub-Store Backup";
const FILE: &str = "Sub-Store";

async fn gist(store: &Arc<Mutex<Store>>, action: &str, files: Value) -> Result<Value, Error> {
    let result = job(store, "gist-request", json!({"key":KEY,"action":action,"filename":FILE,"files":files})).await?;
    Ok(result["result"].clone())
}

async fn crypt(store: &Arc<Mutex<Store>>, body: Value, settings: &Value, encoding: &str, encrypt: bool) -> Result<Value, Error> {
    job(store, if encrypt { "backup-encrypt" } else { "backup-decrypt" }, json!({"body":body,"settings":settings,"encoding":encoding})).await
}

async fn serialize(store: &Arc<Mutex<Store>>, encoding: &str) -> Result<Value, Error> {
    let content = store.lock().unwrap().data.clone();
    job(store, "backup-serialize", json!({"content":content,"encoding":encoding})).await
}

pub(crate) async fn action(store: &Arc<Mutex<Store>>, query: &Value) -> Result<(), Error> {
    let current = store.lock().unwrap().data.clone();
    let settings = current["settings"].clone();
    if !truthy(&settings["gistToken"]) { return Err(Error::invalid("GIST_TOKEN_NOT_FOUND", "GitHub Token is required for backup!")); }
    let encoding = [query.get("encode"), settings.get("gistUpload")].into_iter().flatten().find(|value| truthy(value))
        .map(codec::js_string).unwrap_or_else(|| "base64".to_owned());
    let encoding = if ["base64", "plaintext", "age"].contains(&encoding.as_str()) { encoding.as_str() } else { "base64" };
    if query["action"] == "upload" {
        let before = serialize(store, encoding).await?;
        if let Ok(downloaded) = gist(store, "download", Value::Null).await {
            let armored = downloaded.as_str().is_some_and(|body| codec::trim(body).starts_with("-----BEGIN AGE ENCRYPTED FILE-----"));
            if let Ok(online) = crypt(store, downloaded, &settings, encoding, false).await
                && (encoding != "age" || armored) && online == before { return Ok(()); }
        }
        {
            let mut store = store.lock().unwrap(); let mut next = store.data.clone();
            next["settings"]["syncTime"] = json!(resources::now()); store.commit(next).map_err(Error::state)?;
        }
        let body = serialize(store, encoding).await?;
        let body = crypt(store, body, &settings, encoding, true).await?;
        tokio::time::sleep(Duration::from_millis(100)).await;
        let mut files = json!({}); files[FILE] = json!({"content":body});
        if let Err(error) = gist(store, "upload", files).await {
            // The source retry references undefined `gists` before making a request.
            tokio::time::sleep(Duration::from_millis(500)).await;
            let mut store = store.lock().unwrap(); let mut next = store.data.clone();
            if let Some(old) = settings.get("syncTime") { next["settings"]["syncTime"] = old.clone(); }
            else if let Some(settings) = next["settings"].as_object_mut() { settings.shift_remove("syncTime"); }
            store.commit(next).map_err(Error::state)?; return Err(error);
        }
    } else if query["action"] == "download" {
        let strategy = if let Some(strategy) = query.get("tokenStrategy") {
            if strategy != "overwrite" && strategy != "keep" {
                return Err(Error::invalid("INVALID_GIST_DOWNLOAD_TOKEN_STRATEGY", "Token 处理方式仅支持 overwrite 或 keep"));
            }
            codec::js_string(strategy)
        } else if let Some(keep) = query.get("keep") {
            if codec::js_string(keep).split(',').any(|path| path == "settings.gistToken") { "keep" } else { "overwrite" }.to_owned()
        } else if settings["gistDownloadTokenStrategy"] == "keep" { "keep".to_owned() } else { "overwrite".to_owned() };
        let mut keep: Vec<String> = query.get("keep").filter(|value| truthy(value)).map(codec::js_string).unwrap_or_default()
            .split(',').map(codec::trim).filter(|path| !path.is_empty()).map(str::to_owned).collect();
        if strategy == "keep" && !keep.iter().any(|path| path == "settings.gistToken") { keep.push("settings.gistToken".to_owned()); }
        else if strategy == "overwrite" && let Some(index) = keep.iter().position(|path| path == "settings.gistToken") { keep.remove(index); }
        if !keep.iter().any(|path| path == "settings.gistDownloadTokenStrategy") { keep.push("settings.gistDownloadTokenStrategy".to_owned()); }
        let body = gist(store, "download", Value::Null).await?;
        let body = crypt(store, body, &settings, encoding, false).await?;
        let mut restored = job(store, "backup-decode", json!({"body":body,"current":current,"keep":keep})).await?;
        let original = restored.clone();
        state::migrate(&mut restored).map_err(|_| Error::plain("Gist 备份文件校验失败, 无法还原"))?;
        if !state::valid_records(&restored) { return Err(Error::plain("Gist 备份文件校验失败, 无法还原")); }
        store.lock().unwrap().restore(restored, original).map_err(Error::state)?;
    }
    Ok(())
}

pub async fn dispatch(method: &Method, path: &str, query: &Value, store: &Arc<Mutex<Store>>) -> Option<api::Reply> {
    if method != Method::GET || path != "/api/utils/backup" { return None; }
    Some(match action(store, query).await {
        Ok(()) => api::success(None, 200),
        Err(error) if error.kind == "RequestInvalidError" || error.code == "STATE_WRITE_FAILED" => error.reply(500),
        Err(error) => Error::internal("BACKUP_FAILED", &format!("Failed to {} gist data!", query.get("action").map(codec::js_string).unwrap_or_else(|| "undefined".to_owned())), error.message).reply(500),
    })
}
