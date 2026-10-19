use crate::{api, artifacts, body::Body, files, json as codec, resources::{self, Error}, state::Store, tokens};
use hyper::Method;
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};

pub(crate) fn insert(next: &mut Value, kind: &str, snapshot: &Value) -> Result<(), Error> {
    let mut random = [0u8; 8];
    getrandom::fill(&mut random).map_err(|error| Error::plain(error.to_string()))?;
    let suffix = random.iter().map(|byte| b"0123456789abcdefghijklmnopqrstuvwxyz"[(byte % 36) as usize] as char).collect::<String>();
    let mut entry = json!({"id":format!("{}-{suffix}", resources::now()),"archivedAt":resources::now(),"itemType":kind});
    for field in ["name", "displayName", "remark", "tag"] {
        if let Some(value) = snapshot.get(field) { entry[field] = value.clone(); }
    }
    entry["snapshot"] = snapshot.clone();
    if kind == "share" && let Some(value) = snapshot.get("type") { entry["shareType"] = value.clone(); }
    if !next["archives"].is_array() { next["archives"] = json!([]); }
    next["archives"].as_array_mut().unwrap().insert(0, entry);
    Ok(())
}

fn position(data: &Value, id: &str) -> Result<usize, Error> {
    data["archives"].as_array().and_then(|entries| entries.iter().position(|entry| entry["id"] == id)).ok_or_else(|| {
        let mut error = Error::missing(format!("Archive entry {id} does not exist"));
        error.code = "ARCHIVE_ENTRY_NOT_FOUND".to_owned();
        error
    })
}

fn restore_error(error: Error) -> api::Reply {
    match error.kind {
        "RequestInvalidError" => error.reply(400),
        "ResourceNotFoundError" => error.reply(404),
        _ if error.code == "STATE_WRITE_FAILED" => error.reply(500),
        _ => Error::internal("ARCHIVE_RESTORE_FAILED", "Failed to restore archive entry", error.message).reply(500),
    }
}

async fn restore(store: &Arc<Mutex<Store>>, id: &str) -> Result<Value, Error> {
    let entry = {
        let store = store.lock().unwrap();
        store.data["archives"][position(&store.data, id)?].clone()
    };
    let snapshot = entry.get("snapshot").cloned().ok_or_else(|| Error::plain("\"undefined\" is not valid JSON"))?;
    let expiration = if entry["itemType"] == "share" {
        let mut options = json!({});
        for key in ["mode", "expiresIn", "expiresValue", "expiresUnit", "exp", "count", "usedCount"] {
            if let Some(value) = snapshot.get(key) { options[key] = value.clone(); }
        }
        tokens::expiration(store, &snapshot, options).await?
    } else { Value::Null };
    let mut store = store.lock().unwrap();
    let index = position(&store.data, id)?;
    let mut next = store.data.clone();
    let body = Body::Json(snapshot.clone());
    let item = match entry["itemType"].as_str() {
        Some("sub") => api::prepare_create(&store, &mut next, "subs", &body)?,
        Some("col") => api::prepare_create(&store, &mut next, "collections", &body)?,
        Some("file") => files::prepare_create(&store, &mut next, &body)?,
        Some("artifact") => {
            let mut fields = body.fields();
            fields.shift_remove("updated");
            fields.shift_remove("url");
            artifacts::prepare_create(&store, &mut next, Value::Object(fields))?
        }
        Some("share") => tokens::prepare_create(&store, &mut next, snapshot, expiration)?,
        _ => return Err(Error::invalid("INVALID_ARCHIVE_TYPE", format!("Unsupported archive item type: {}",
            entry.get("itemType").map(codec::js_string).unwrap_or_else(|| "undefined".to_owned())))),
    };
    next["archives"].as_array_mut().unwrap().remove(index);
    store.commit(next).map_err(Error::state)?;
    Ok(item)
}

pub async fn dispatch(method: &Method, path: &str, body: &Body, store: &Arc<Mutex<Store>>) -> Option<api::Reply> {
    if path == "/api/sort/archives" && method == Method::POST {
        let ids = body.clone().into_json();
        let Some(ids) = ids.as_array() else { return Some(api::html_error(500, "[object Object]")); };
        let mut store = store.lock().unwrap();
        let mut next = store.data.clone();
        if !next["archives"].is_array() { next["archives"] = json!([]); }
        next["archives"].as_array_mut().unwrap().sort_by_key(|entry| {
            ids.iter().rposition(|id| tokens::same(id, &entry["id"])).unwrap_or(usize::MAX)
        });
        let entries = next["archives"].clone();
        return Some(api::persisted(store.commit(next), api::success(Some(entries), 200)));
    }
    if path == "/api/archives" && method == Method::GET {
        return Some(api::success(Some(store.lock().unwrap().data["archives"].clone()), 200));
    }
    let tail = path.strip_prefix("/api/archives/")?;
    if let Some(id) = tail.strip_suffix("/restore") && method == Method::POST {
        return Some(restore(store, id).await.map_or_else(restore_error, |item| api::success(Some(item), 200)));
    }
    if tail.contains('/') || (method != Method::GET && method != Method::DELETE) { return Some(api::not_found()); }
    let mut store = store.lock().unwrap();
    let index = match position(&store.data, tail) { Ok(index) => index, Err(error) => return Some(error.reply(404)) };
    let entry = store.data["archives"][index].clone();
    Some(if method == Method::DELETE {
        let mut next = store.data.clone();
        next["archives"].as_array_mut().unwrap().remove(index);
        api::persisted(store.commit(next), api::success(Some(entry), 200))
    } else { api::success(Some(entry), 200) })
}
