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
        Ok(result) if result["error"]["type"].is_string() => api::failure_details(500,
            &codec::js_string(&result["error"]["code"]), &codec::js_string(&result["error"]["type"]),
            &codec::js_string(&result["error"]["message"]), result["error"].get("details").cloned()),
        Ok(result) if result.get("error").is_some() => api::caught_error(&codec::js_string(&result["error"]["message"])),
        Ok(result) => api::success(result.get("data").cloned(), 200),
        Err(error) => error.reply(500),
    })
}
