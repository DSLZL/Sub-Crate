use crate::{api, body::Body, json as codec, resources::Error, script, state::{Store, truthy}};
use hyper::Method;
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};

pub(crate) async fn job(store: &Arc<Mutex<Store>>, operation: &str, input: Value) -> Result<Value, Error> {
    script::run(store.clone(), operation, input).await
        .map_err(|crate::conversion::Error::Legacy(reason)| Error::plain(reason))
}

pub(crate) async fn refresh(store: &Arc<Mutex<Store>>) -> Result<(), Error> {
    if !truthy(&store.lock().unwrap().data["settings"]["gistToken"]) { return Ok(()); }
    let result = job(store, "gist-request", json!({"key":"Sub-Store Artifacts Repository","action":"locate"})).await;
    let (url, status) = match result {
        Ok(result) => {
            let gist = &result["result"];
            let url = gist.get("html_url").filter(|value| !value.is_null()).unwrap_or(&gist["web_url"]).clone();
            let status = if truthy(&url) { "VALID" } else { "NOT FOUND" }; (url, status)
        }
        Err(_) => (Value::Null, "ERROR"),
    };
    let mut store = store.lock().unwrap();
    let mut next = store.data.clone();
    if !truthy(&next["settings"]) { next["settings"] = json!({}); }
    if let Some(settings) = next["settings"].as_object_mut() {
        if truthy(&url) { settings.insert("artifactStore".to_owned(), url); }
        settings.insert("artifactStoreStatus".to_owned(), json!(status));
    }
    store.commit(next).map_err(Error::state)
}

async fn update(store: &Arc<Mutex<Store>>, body: &Body) -> Result<Value, Error> {
    let patch = body.fields();
    let mut settings = api::normalized_settings(&store.lock().unwrap(), body)?;
    if patch.get("gistUpload").is_some_and(|value| value == "age") || settings["gistUpload"] == "age" && patch.contains_key("age-secret-key") {
        let normalized = job(store, "settings-age", settings).await?;
        if let Some(error) = normalized.get("error") {
            return Err(Error::invalid(codec::js_string(&error["code"]), codec::js_string(&error["message"])));
        }
        settings = normalized["settings"].clone();
    }
    if patch.get("gistDownloadTokenStrategy").is_some_and(|value| ![json!("ask"), json!("overwrite"), json!("keep")].contains(value)) {
        return Err(Error::invalid("INVALID_GIST_DOWNLOAD_TOKEN_STRATEGY", "Token 处理方式仅支持 ask、overwrite 或 keep"));
    }
    {
        let mut store = store.lock().unwrap(); let mut next = store.data.clone(); next["settings"] = settings.clone();
        store.commit(next).map_err(Error::state)?;
    }
    if ["gistToken", "githubProxy", "githubApiUrl", "defaultProxy"].iter().any(|key| patch.contains_key(*key)) {
        refresh(store).await?;
        settings = store.lock().unwrap().data["settings"].clone();
    }
    Ok(settings)
}

pub async fn dispatch(method: &Method, path: &str, body: &Body, store: &Arc<Mutex<Store>>) -> Option<api::Reply> {
    if path == "/api/utils/refresh" && method == Method::GET {
        let result = refresh(store).await.and_then(|()| store.lock().unwrap().clear_resource_cache().map_err(Error::state));
        return Some(result.map_or_else(|error| error.reply(500), |()| api::success(None, 200)));
    }
    if path != "/api/settings" { return None; }
    if method == Method::GET {
        {
            let mut store = store.lock().unwrap();
            if !truthy(&store.data["settings"]) {
                let mut next = store.data.clone(); next["settings"] = json!({});
                if let Err(error) = store.commit(next) { return Some(Error::state(error).reply(500)); }
            }
        }
        let refresh_needed = !truthy(&store.lock().unwrap().data["settings"]["artifactStore"]);
        if refresh_needed && let Err(error) = refresh(store).await {
            return Some(error.reply(500));
        }
        return Some(api::success(Some(store.lock().unwrap().data["settings"].clone()), 200));
    }
    if method != Method::PATCH { return None; }
    Some(match update(store, body).await {
        Ok(data) => api::success(Some(data), 200),
        Err(error) if error.kind == "RequestInvalidError" || error.code == "STATE_WRITE_FAILED" || error.code == "FAILED_TO_UPDATE_SETTINGS" => error.reply(500),
        Err(error) => Error::internal("FAILED_TO_UPDATE_SETTINGS", "Failed to update settings", error.message).reply(500),
    })
}
