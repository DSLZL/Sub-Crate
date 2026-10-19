use crate::{api, body::Body, json as codec, settings::job, state::Store};
use hyper::Method;
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};

pub async fn dispatch(method: &Method, path: &str, query: &Value, body: &Body, store: &Arc<Mutex<Store>>) -> Option<api::Reply> {
    if method != Method::POST { return None; }
    let operation = match path {
        "/api/utils/age/key-pair" | "/api/utils/age/public-key" => "age-rest",
        "/api/utils/node-info" => "node-info",
        _ => return None,
    };
    Some(match job(store, operation, json!({"action":path.rsplit('/').next(),"body":body.clone().into_json(),"query":query})).await {
        Ok(result) if result.get("error").is_some() => api::send(500, "text/plain; charset=utf-8", codec::stringify(&json!({"status":"failed","error":result["error"]}))),
        Ok(result) => api::success(result.get("data").cloned(), 200),
        Err(error) => error.reply(500),
    })
}
