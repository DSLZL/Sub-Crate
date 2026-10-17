use crate::state::{Store, decode, migrate, truthy, valid_records};
use http_body_util::Full;
use hyper::{Method, Response, body::Bytes};
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

pub type Reply = Response<Full<Bytes>>;
const COMPONENT: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'_')
    .remove(b'.')
    .remove(b'!')
    .remove(b'~')
    .remove(b'*')
    .remove(b'\'')
    .remove(b'(')
    .remove(b')');

pub fn text(status: u16, content_type: &str, body: impl Into<Bytes>) -> Reply {
    Response::builder()
        .status(status)
        .header("content-type", content_type)
        .body(Full::new(body.into()))
        .unwrap()
}

fn success(data: Option<Value>, status: u16) -> Reply {
    let mut value = json!({"status":"success"});
    if let Some(data) = data {
        value["data"] = data;
    }
    text(status, "application/json; charset=utf-8", value.to_string())
}

pub fn failure(status: u16, code: &str, kind: &str, message: &str) -> Reply {
    text(
        status,
        "application/json; charset=utf-8",
        json!({
            "status":"failed", "error":{"code":code, "type":kind, "message":message}
        })
        .to_string(),
    )
}

fn invalid(code: &str, message: &str) -> Reply {
    failure(500, code, "RequestInvalidError", message)
}

fn pending(message: &str) -> Reply {
    failure(
        501,
        "FEATURE_NOT_IMPLEMENTED",
        "InternalServerError",
        message,
    )
}

fn download(value: &Value, name: &str) -> Reply {
    // shortcut: local-calendar filename timestamps remain an M2 export-format gate.
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let filename = format!("{name}_{stamp}.json");
    let disposition = format!(
        "attachment; filename=\"{}\"",
        utf8_percent_encode(&filename, COMPONENT)
    );
    let mut response = text(200, "application/json; charset=utf-8", value.to_string());
    response.headers_mut().insert(
        "access-control-expose-headers",
        "content-disposition".parse().unwrap(),
    );
    response
        .headers_mut()
        .insert("content-disposition", disposition.parse().unwrap());
    response
}

fn publish(store: &mut Store, next: Value, response: Reply) -> Reply {
    match store.commit(next) {
        Ok(()) => response,
        Err(error) => {
            eprintln!("State write failed: {error}");
            failure(
                500,
                "STATE_WRITE_FAILED",
                "InternalServerError",
                "Failed to persist state",
            )
        }
    }
}

fn editor_languages(value: &mut Value) {
    if let Some(object) = value.as_object_mut() {
        for field in ["editorLanguage", "previewLanguage"] {
            if !object
                .get(field)
                .and_then(Value::as_str)
                .is_some_and(|language| {
                    ["javascript", "json", "json5", "yaml", "ini", "plaintext"].contains(&language)
                })
            {
                object.remove(field);
            }
        }
    }
}

fn normalize_item(item: &mut Value, subscription: bool) {
    if subscription {
        item.as_object_mut().unwrap().remove("subscriptions");
    }
    if item
        .get("age-public-key")
        .is_some_and(|key| key.is_null() || key.as_str().is_some_and(|s| s.trim().is_empty()))
    {
        item.as_object_mut().unwrap().remove("age-public-key");
    }
    editor_languages(item);
    if let Some(processes) = item.get_mut("process").and_then(Value::as_array_mut) {
        for process in processes {
            if let Some(args) = process.get_mut("args") {
                editor_languages(args);
            }
        }
    }
}

fn number(value: &Value) -> f64 {
    match value {
        Value::Null => 0.0,
        Value::Bool(value) => {
            if *value {
                1.0
            } else {
                0.0
            }
        }
        Value::Number(value) => value.as_f64().unwrap_or(f64::NAN),
        Value::String(value) if value.trim().is_empty() => 0.0,
        Value::String(value) => value.trim().parse().unwrap_or(f64::NAN),
        Value::Array(value) if value.is_empty() => 0.0,
        Value::Array(value) if value.len() == 1 => number(&value[0]),
        _ => f64::NAN,
    }
}

fn settings(store: &mut Store, body: Value) -> Reply {
    let Some(patch) = body.as_object() else {
        return invalid("INVALID_SETTINGS", "Settings must be an object");
    };
    if patch
        .get("gistDownloadTokenStrategy")
        .is_some_and(|v| ![json!("ask"), json!("overwrite"), json!("keep")].contains(v))
    {
        return invalid(
            "INVALID_GIST_DOWNLOAD_TOKEN_STRATEGY",
            "Token 处理方式仅支持 ask、overwrite 或 keep",
        );
    }
    if patch
        .keys()
        .any(|key| ["gistToken", "gistUpload", "age-secret-key"].contains(&key.as_str()))
    {
        return pending("Gist and age settings integration is pending");
    }
    let mut next = store.data.clone();
    let mut settings = next["settings"].as_object().cloned().unwrap_or_default();
    let previous_appearance = settings
        .get("appearanceSetting")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    settings.extend(patch.clone());
    if let Some(appearance) = patch.get("appearanceSetting").and_then(Value::as_object) {
        let mut merged = previous_appearance;
        merged.extend(appearance.clone());
        settings.insert("appearanceSetting".to_owned(), Value::Object(merged));
    }
    for key in [
        "defaultTimeout",
        "githubApiTimeout",
        "artifactSyncBatchSize",
        "cacheThreshold",
        "resourceCacheTtl",
        "headersCacheTtl",
        "scriptCacheTtl",
        "logsMaxCount",
        "backendRequestConcurrency",
        "backendRequestConcurrencyWaitTime",
    ] {
        if let Some(raw) = settings.get(key) {
            let value = number(raw);
            let valid = match key {
                "logsMaxCount" => !raw.is_null() && raw != "" && value >= 0.0,
                "backendRequestConcurrency" => {
                    !raw.is_null() && value >= 1.0 && value.fract() == 0.0
                }
                "backendRequestConcurrencyWaitTime" => {
                    !raw.is_null() && raw != "" && value >= 0.0 && value.fract() == 0.0
                }
                _ => value > 0.0,
            };
            if !value.is_finite() || !valid {
                settings.remove(key);
            }
        }
    }
    let value = Value::Object(settings);
    next["settings"] = value.clone();
    publish(store, next, success(Some(value), 200))
}

pub fn dispatch(
    method: &Method,
    path: &str,
    query: &HashMap<String, String>,
    body: Value,
    store: &mut Store,
) -> Reply {
    if path == "/api/storage" {
        if method == Method::GET {
            return download(&store.data, "sub-store_data");
        }
        if method == Method::POST {
            let restored = body["content"]
                .as_str()
                .ok_or_else(|| "Missing content".to_owned())
                .and_then(decode);
            let Ok(mut next) = restored else {
                return invalid(
                    "INVALID_BACKUP_DATA",
                    "Invalid backup data, failed to restore!",
                );
            };
            if !next["settings"].is_object() || migrate(&mut next).is_err() || !valid_records(&next)
            {
                return invalid(
                    "INVALID_BACKUP_DATA",
                    "Invalid backup data, failed to restore!",
                );
            }
            return publish(store, next, success(None, 200));
        }
    }
    if path == "/api/settings" {
        if method == Method::GET {
            return success(Some(store.data["settings"].clone()), 200);
        }
        if method == Method::PATCH {
            return settings(store, body);
        }
    }
    if method == Method::GET {
        let key = match path {
            "/api/artifacts" => Some("artifacts"),
            "/api/wholeFiles" | "/api/files" => Some("files"),
            "/api/modules" => Some("modules"),
            "/api/tokens" => Some("tokens"),
            "/api/archives" => Some("archives"),
            _ => None,
        };
        if let Some(key) = key {
            let mut value = store.data[key].clone();
            if path == "/api/files"
                && let Some(items) = value.as_array_mut()
            {
                for item in items {
                    if let Some(item) = item.as_object_mut() {
                        item.remove("content");
                    }
                }
            }
            return success((!value.is_null()).then_some(value), 200);
        }
    }
    let (key, label, plural, singular) = if path == "/api/subs" || path.starts_with("/api/sub/") {
        ("subs", "Subscription", "/api/subs", "/api/sub/")
    } else if path == "/api/collections" || path.starts_with("/api/collection/") {
        (
            "collections",
            "Collection",
            "/api/collections",
            "/api/collection/",
        )
    } else {
        return text(404, "text/plain; charset=utf-8", "");
    };
    let subscription = key == "subs";
    if path == plural && method == Method::GET {
        return success(store.data.get(key).cloned(), 200);
    }
    let name = path.strip_prefix(singular).unwrap_or("");
    let position = store.data[key]
        .as_array()
        .and_then(|items| items.iter().position(|item| item["name"] == name));
    if path != plural && method == Method::GET {
        return if let Some(position) = position {
            let mut item = store.data[key][position].clone();
            if subscription {
                item.as_object_mut().unwrap().remove("subscriptions");
            }
            if query.get("raw").is_some_and(|s| !s.is_empty()) {
                download(
                    &item,
                    &format!(
                        "sub-store_{}_{name}",
                        if subscription {
                            "subscription"
                        } else {
                            "collection"
                        }
                    ),
                )
            } else {
                success(Some(item), 200)
            }
        } else {
            failure(
                404,
                "SUBSCRIPTION_NOT_FOUND",
                "ResourceNotFoundError",
                &format!("{label} {name} does not exist"),
            )
        };
    }
    let mut next = store.data.clone();
    if next.get(key).is_none() {
        next[key] = json!([]);
    }
    if path == plural && method == Method::PUT {
        let Some(mut items) = body.as_array().cloned() else {
            return invalid("INVALID_DATA", "Expected an array");
        };
        if items.iter().any(|item| !item.is_object()) {
            return invalid("INVALID_DATA", "Expected object items");
        }
        if items
            .iter()
            .any(|item| item.get("age-public-key").is_some_and(truthy))
        {
            return pending("age recipient validation is pending");
        }
        for item in &mut items {
            normalize_item(item, false);
        }
        next[key] = Value::Array(items);
        return publish(store, next, success(None, 200));
    }
    if path == plural && method == Method::POST {
        let mut item = body;
        if !item.is_object() {
            return invalid("INVALID_DATA", "Expected an object");
        }
        let item_name = item["name"].as_str().unwrap_or("").to_owned();
        if item_name.is_empty() || item_name.contains('/') {
            return invalid("INVALID_NAME", &format!("{label} {item_name} is invalid"));
        }
        if next[key]
            .as_array()
            .unwrap()
            .iter()
            .any(|old| old["name"] == item_name)
        {
            return invalid(
                "DUPLICATE_KEY",
                &format!("{label} {item_name} already exists."),
            );
        }
        if item.get("age-public-key").is_some_and(truthy) {
            return pending("age recipient validation is pending");
        }
        normalize_item(&mut item, subscription);
        let top = next["settings"]["appearanceSetting"]["createItemPosition"] == "top";
        let items = next[key].as_array_mut().unwrap();
        if top {
            items.insert(0, item.clone());
        } else {
            items.push(item.clone());
        }
        return publish(store, next, success(Some(item), 201));
    }
    if path != plural && (method == Method::PATCH || method == Method::DELETE) {
        let Some(position) = position else {
            return failure(
                404,
                "RESOURCE_NOT_FOUND",
                "ResourceNotFoundError",
                &format!("{label} {name} does not exist!"),
            );
        };
        if method == Method::DELETE {
            if let Some(mode) = query.get("mode") {
                if mode == "archive" {
                    return pending("Archive deletion is pending");
                }
                if !mode.is_empty() && mode != "permanent" {
                    return invalid(
                        "INVALID_DELETE_MODE",
                        &format!("Unsupported delete mode: {mode}"),
                    );
                }
            }
            next[key].as_array_mut().unwrap().remove(position);
            if subscription {
                for collection in next["collections"].as_array_mut().into_iter().flatten() {
                    if let Some(names) = collection["subscriptions"].as_array_mut() {
                        names.retain(|value| value != name);
                    }
                }
            }
            return publish(store, next, success(None, 200));
        }
        let Some(mut patch) = body.as_object().cloned() else {
            return invalid("INVALID_DATA", "Expected an object");
        };
        if patch.get("age-public-key").is_some_and(truthy) {
            return pending("age recipient validation is pending");
        }
        if !patch.get("name").is_some_and(truthy) {
            patch.insert("name".to_owned(), json!(name));
        }
        let mut item = next[key][position].clone();
        item.as_object_mut().unwrap().extend(patch);
        normalize_item(&mut item, subscription);
        let new_name = item["name"].clone();
        let source_type = if subscription {
            "subscription"
        } else {
            "collection"
        };
        if new_name != name {
            if subscription {
                for collection in next["collections"].as_array_mut().into_iter().flatten() {
                    if let Some(names) = collection["subscriptions"].as_array_mut()
                        && let Some(index) = names.iter().position(|value| value == name)
                    {
                        names[index] = new_name.clone();
                    }
                }
            }
            for artifact in next["artifacts"].as_array_mut().into_iter().flatten() {
                if artifact["type"] == source_type && artifact["source"] == name {
                    artifact["source"] = new_name.clone();
                }
            }
            for file in next["files"].as_array_mut().into_iter().flatten() {
                if file["sourceType"] == source_type && file["sourceName"] == name {
                    file["sourceName"] = new_name.clone();
                }
            }
        }
        next[key][position] = item.clone();
        return publish(store, next, success(Some(item), 200));
    }
    text(404, "text/plain; charset=utf-8", "")
}
