use crate::{api, body::Body, json as codec, script, state::Store};
use hyper::Method;
use serde_json::{Value, json};
use std::{
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};

pub(crate) struct Error {
    pub code: String,
    pub kind: &'static str,
    pub message: String,
    pub details: Option<Value>,
}

impl Error {
    pub fn invalid(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            kind: "RequestInvalidError",
            message: message.into(),
            details: None,
        }
    }

    pub fn missing(message: impl Into<String>) -> Self {
        Self {
            code: "RESOURCE_NOT_FOUND".to_owned(),
            kind: "ResourceNotFoundError",
            message: message.into(),
            details: None,
        }
    }

    pub fn internal(code: &str, message: &str, reason: impl std::fmt::Display) -> Self {
        Self {
            code: code.to_owned(),
            kind: "InternalServerError",
            message: message.to_owned(),
            details: Some(json!(format!("Reason: {reason}"))),
        }
    }

    pub fn state(error: std::io::Error) -> Self {
        eprintln!("State write failed: {error}");
        Self {
            code: "STATE_WRITE_FAILED".to_owned(),
            kind: "InternalServerError",
            message: "Failed to persist state".to_owned(),
            details: None,
        }
    }

    pub fn reply(self, status: u16) -> api::Reply {
        api::failure_details(status, &self.code, self.kind, &self.message, self.details)
    }
}

pub(crate) fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

pub(crate) fn archive_mode(mode: &Value) -> Result<bool, Error> {
    if mode.is_null() || mode == "" || mode == "permanent" {
        Ok(false)
    } else if mode == "archive" {
        Ok(true)
    } else {
        Err(Error::invalid(
            "INVALID_DELETE_MODE",
            format!("Unsupported delete mode: {}", codec::js_string(mode)),
        ))
    }
}

pub async fn dispatch(
    method: &Method,
    path: &str,
    body: &Body,
    store: &Arc<Mutex<Store>>,
) -> Option<api::Reply> {
    if path != "/api/modules" && !path.starts_with("/api/module/") {
        return None;
    }
    if !(method == Method::GET
        || (path == "/api/modules" && (method == Method::POST || method == Method::PUT))
        || (path != "/api/modules" && (method == Method::PATCH || method == Method::DELETE)))
    {
        return Some(api::not_found());
    }
    let mut item = body.clone().into_json();
    if path == "/api/modules" && method == Method::POST {
        let name = match script::run(store.clone(), "module-name", item.clone()).await {
            Ok(name) => name,
            Err(crate::conversion::Error::Legacy(message)) => {
                return Some(api::uncaught_error(500, &format!("TypeError: {message}")));
            }
        };
        if let Some(fields) = item.as_object_mut() {
            fields.insert("name".to_owned(), name);
        }
    }
    Some(modules(
        method,
        path,
        body,
        item,
        &mut store.lock().unwrap(),
    ))
}

fn modules(method: &Method, path: &str, body: &Body, item: Value, store: &mut Store) -> api::Reply {
    let plural = path == "/api/modules";
    if plural && method == Method::PUT {
        let mut next = store.data.clone();
        next["modules"] = item;
        return api::persisted(
            store.commit_list(next, "modules", matches!(body, Body::Raw(_))),
            api::success(None, 200),
        );
    }
    let Some(items) = store.list("modules") else {
        return api::uncaught_error(500, "TypeError: e.find is not a function");
    };
    if plural && method == Method::GET {
        let mut metadata = Vec::new();
        for item in items {
            if item.is_null() {
                return api::uncaught_error(
                    500,
                    "TypeError: Cannot convert undefined or null to object",
                );
            }
            let mut fields = Body::Json(item.clone()).fields();
            fields.shift_remove("content");
            metadata.push(Value::Object(fields));
        }
        return api::success(Some(json!(metadata)), 200);
    }
    let name = path.strip_prefix("/api/module/").unwrap_or("");
    let requested = if plural { item.get("name") } else { None };
    let mut position = None;
    for (index, stored) in items.iter().enumerate() {
        if stored.is_null() {
            return api::uncaught_error(
                500,
                "TypeError: Cannot read properties of null (reading 'name')",
            );
        }
        let matches = if plural {
            match (stored.get("name"), requested) {
                (None, None) => true,
                (Some(a), Some(b)) if !a.is_object() && !a.is_array() => a == b,
                _ => false,
            }
        } else {
            stored.get("name").is_some_and(|stored| stored == name)
        };
        if matches {
            position = Some(index);
            break;
        }
    }
    if plural && method == Method::POST {
        if position.is_some() {
            let message = match item.get("name") {
                Some(name) if crate::state::truthy(name) => {
                    format!("已存在 name 为 {} 的模块", codec::js_string(name))
                }
                _ => "已存在相同的模块 请勿重复添加".to_owned(),
            };
            return api::failure(500, "DUPLICATE_KEY", "RequestInvalidError", &message);
        }
        let mut next = store.data.clone();
        let Some(items) = next["modules"].as_array_mut() else {
            return api::uncaught_error(500, "TypeError: e.push is not a function");
        };
        items.push(item.clone());
        return api::persisted(store.commit(next), api::success(Some(item), 201));
    }
    if !plural && method == Method::GET {
        return match position {
            Some(index) => content(items[index].get("content")),
            None => api::send(
                500,
                "text/plain; charset=utf-8",
                codec::stringify(&json!({"status":"failed","error":{
                    "code":"MODULE_NOT_FOUND","type":"ResourceNotFoundError",
                    "message":format!("Module {name} does not exist"),"details":404
                }})),
            ),
        };
    }
    if !plural && method == Method::PATCH {
        let Some(index) = position else {
            return api::failure(
                404,
                "RESOURCE_NOT_FOUND",
                "ResourceNotFoundError",
                &format!("Module {name} does not exist!"),
            );
        };
        let mut merged = Body::Json(items[index].clone()).fields();
        merged.extend(body.fields());
        let value = Value::Object(merged);
        let mut next = store.data.clone();
        next["modules"][index] = value.clone();
        return api::persisted(store.commit(next), api::success(Some(value), 200));
    }
    if !plural && method == Method::DELETE {
        let mut next = store.data.clone();
        let Some(items) = next["modules"].as_array_mut() else {
            return api::uncaught_error(500, "TypeError: e.splice is not a function");
        };
        // The source calls splice(-1, 1) for a missing name.
        if let Some(index) = position.or_else(|| items.len().checked_sub(1)) {
            items.remove(index);
        }
        return api::persisted(store.commit(next), api::success(None, 200));
    }
    api::not_found()
}

fn content(value: Option<&Value>) -> api::Reply {
    let Some(value) = value else {
        return api::text(200, "text/plain; charset=utf-8", "");
    };
    if let Some(number) = value.as_f64() {
        let status = (number.fract() == 0.0)
            .then_some(number as u16)
            .and_then(|number| hyper::StatusCode::from_u16(number).ok());
        return match status {
            Some(status) => api::send(
                status.as_u16(),
                "text/plain; charset=utf-8",
                status.canonical_reason().unwrap_or(""),
            ),
            None => api::uncaught_error(
                500,
                &format!("RangeError [ERR_HTTP_INVALID_STATUS_CODE]: Invalid status code: {number}"),
            ),
        };
    }
    let text = match value {
        Value::Null => String::new(),
        Value::String(value) => codec::text(value),
        _ => codec::stringify(&crate::state::legacy_object_order(value.clone())),
    };
    api::send(200, "text/plain; charset=utf-8", text)
}
