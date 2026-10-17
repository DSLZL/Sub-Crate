use crate::body::Body;
use crate::state::{Store, decode, legacy_object_order, migrate, truthy, valid_records};
use base64::Engine;
use http_body_util::Full;
use hyper::{Method, Response, body::Bytes};
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::io;
use std::mem::MaybeUninit;

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

pub fn send(status: u16, content_type: &str, body: impl Into<Bytes>) -> Reply {
    let body = body.into();
    let length = body.len();
    let digest = aws_lc_rs::digest::digest(&aws_lc_rs::digest::SHA1_FOR_LEGACY_USE_ONLY, &body);
    let hash = base64::engine::general_purpose::STANDARD_NO_PAD.encode(digest.as_ref());
    let etag = format!("W/\"{length:x}-{hash}\"");
    let mut response = text(status, content_type, body);
    response.headers_mut().insert("etag", etag.parse().unwrap());
    response
        .headers_mut()
        .insert("content-length", length.into());
    response
}

pub fn not_found() -> Reply {
    text(404, "text/plain;charset=UTF-8", "")
}

pub fn html_error(status: u16, message: &str) -> Reply {
    let mut response = text(
        status,
        "text/html; charset=utf-8",
        format!(
            "<!DOCTYPE html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n<title>Error</title>\n</head>\n<body>\n<pre>{message}</pre>\n</body>\n</html>\n"
        ),
    );
    response.headers_mut().insert(
        "content-security-policy",
        "default-src 'none'".parse().unwrap(),
    );
    response
        .headers_mut()
        .insert("x-content-type-options", "nosniff".parse().unwrap());
    response
}

fn null_name_error(caught: bool) -> Reply {
    if !caught {
        return html_error(500, "Internal Server Error");
    }
    send(
        500,
        "text/plain; charset=utf-8",
        json!({"status":"failed", "error":{"message":"Cannot read properties of null (reading 'name')"}}).to_string(),
    )
}

fn success(data: Option<Value>, status: u16) -> Reply {
    let mut value = json!({"status":"success"});
    if let Some(data) = data {
        value["data"] = legacy_object_order(data);
    }
    // The legacy middleware sets text/plain before Express serializes JSON.
    send(status, "text/plain; charset=utf-8", value.to_string())
}

pub fn failure(status: u16, code: &str, kind: &str, message: &str) -> Reply {
    send(
        status,
        "text/plain; charset=utf-8",
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

fn invalid_backup() -> Reply {
    send(
        500,
        "text/plain; charset=utf-8",
        json!({"status":"failed", "error":{
            "code":"INVALID_BACKUP_DATA", "type":"RequestInvalidError",
            "message":"Invalid backup data, failed to restore!",
            "details":"Reason: 备份文件校验失败, 无法还原"
        }})
        .to_string(),
    )
}

fn download(value: &Value, name: &str) -> Reply {
    let mut local = MaybeUninit::<libc::tm>::uninit();
    // SAFETY: each reentrant CRT call initializes this tm on success; time accepts null.
    let local = unsafe {
        let now = libc::time(std::ptr::null_mut());
        #[cfg(unix)]
        let valid = !libc::localtime_r(&now, local.as_mut_ptr()).is_null();
        #[cfg(windows)]
        let valid = libc::localtime_s(local.as_mut_ptr(), &now) == 0;
        if !valid {
            return failure(
                500,
                "DATE_FORMAT_FAILED",
                "InternalServerError",
                "Failed to read local time",
            );
        }
        local.assume_init()
    };
    let stamp = format!(
        "{:04}-{:02}-{:02}_{:02}-{:02}-{:02}",
        local.tm_year + 1900,
        local.tm_mon + 1,
        local.tm_mday,
        local.tm_hour,
        local.tm_min,
        local.tm_sec
    );
    let filename = format!("{name}_{stamp}.json");
    let disposition = format!(
        "attachment; filename=\"{}\"",
        utf8_percent_encode(&filename, COMPONENT)
    );
    let mut response = send(
        200,
        "application/json; charset=utf-8",
        legacy_object_order(value.clone()).to_string(),
    );
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
    persisted(store.commit(next), response)
}

fn persisted(result: io::Result<()>, response: Reply) -> Reply {
    match result {
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

fn normalize_item(item: &mut Value, subscription: bool) -> Result<(), &'static str> {
    let Some(object) = item.as_object_mut() else {
        return Ok(());
    };
    if subscription {
        object.remove("subscriptions");
    }
    let key = object
        .get("age-public-key")
        .filter(|key| !key.is_null())
        .map(js_string)
        .unwrap_or_default();
    let key = js_trim(&key);
    if key.is_empty() {
        object.remove("age-public-key");
    } else if valid_recipient(key) {
        object.insert("age-public-key".to_owned(), json!(key));
    } else {
        return Err("age-public-key 仅支持 X25519(age1...) 或 MLKEM768-X25519(age1pq1...) 公钥");
    }
    editor_languages(item);
    if let Some(processes) = item.get_mut("process").and_then(Value::as_array_mut) {
        for process in processes {
            if let Some(args) = process.get_mut("args") {
                editor_languages(args);
            }
        }
    }
    Ok(())
}

fn valid_recipient(key: &str) -> bool {
    let Some((prefix, encoded)) = key.rsplit_once('1') else {
        return false;
    };
    let bytes: usize = match prefix {
        "age" => 32,
        "age1pq" => 1216,
        _ => return false,
    };
    let words = (bytes * 8).div_ceil(5);
    if encoded.len() != words + 6 {
        return false;
    }
    let alphabet = b"qpzry9x8gf2tvdw0s3jn54khce6mua7l";
    let Some(values): Option<Vec<_>> = encoded
        .bytes()
        .map(|byte| {
            alphabet
                .iter()
                .position(|symbol| *symbol == byte)
                .map(|index| index as u32)
        })
        .collect()
    else {
        return false;
    };
    // Bech32 requires a checksum of one and zero unused bits in the last word.
    if values[words - 1] & ((1 << (words * 5 - bytes * 8)) - 1) != 0 {
        return false;
    }
    let mut checksum = 1u32;
    for value in prefix
        .bytes()
        .map(|byte| u32::from(byte >> 5))
        .chain([0])
        .chain(prefix.bytes().map(|byte| u32::from(byte & 31)))
        .chain(values)
    {
        let top = checksum >> 25;
        checksum = ((checksum & 0x1ffffff) << 5) ^ value;
        for (index, generator) in [0x3b6a57b2, 0x26508e6d, 0x1ea119fa, 0x3d4233dd, 0x2a1462b3]
            .iter()
            .enumerate()
        {
            if (top >> index) & 1 != 0 {
                checksum ^= *generator;
            }
        }
    }
    checksum == 1
}

fn js_trim(value: &str) -> &str {
    value.trim_matches(|c| {
        matches!(
            c,
            '\u{0009}'..='\u{000d}'
                | ' '
                | '\u{00a0}'
                | '\u{1680}'
                | '\u{2000}'..='\u{200a}'
                | '\u{2028}'
                | '\u{2029}'
                | '\u{202f}'
                | '\u{205f}'
                | '\u{3000}'
                | '\u{feff}'
        )
    })
}

fn js_string(value: &Value) -> String {
    match value {
        Value::String(value) => value.clone(),
        Value::Array(items) => items
            .iter()
            .map(|item| {
                if item.is_null() {
                    String::new()
                } else {
                    js_string(item)
                }
            })
            .collect::<Vec<_>>()
            .join(","),
        Value::Object(_) => "[object Object]".to_owned(),
        _ => value.to_string(),
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
        Value::String(value) => {
            let value = js_trim(value);
            if value.is_empty() {
                return 0.0;
            }
            for (prefix, radix) in [
                ("0x", 16),
                ("0X", 16),
                ("0b", 2),
                ("0B", 2),
                ("0o", 8),
                ("0O", 8),
            ] {
                if let Some(digits) = value.strip_prefix(prefix) {
                    if digits.is_empty() {
                        return f64::NAN;
                    }
                    return digits
                        .chars()
                        .try_fold(0.0, |total, digit| {
                            digit
                                .to_digit(radix)
                                .map(|digit| total * f64::from(radix) + f64::from(digit))
                        })
                        .unwrap_or(f64::NAN);
                }
            }
            value.parse().unwrap_or(f64::NAN)
        }
        Value::Array(_) => number(&Value::String(js_string(value))),
        _ => f64::NAN,
    }
}

fn settings(store: &mut Store, body: Body) -> Reply {
    let patch = body.fields();
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

fn reference_name_matches(value: &Value, name: &str, subscription: bool) -> bool {
    if value == name {
        return true;
    }
    subscription
        && match value {
            Value::Number(_) | Value::Bool(_) => number(value) == number(&json!(name)),
            Value::Array(_) | Value::Object(_) => js_string(value) == name,
            _ => false,
        }
}

pub fn dispatch(
    method: &Method,
    path: &str,
    query: &HashMap<String, String>,
    body: Body,
    store: &mut Store,
) -> Reply {
    if path == "/api/utils/refresh" && method == Method::GET {
        if truthy(&store.data["settings"]["gistToken"]) {
            return pending("Gist refresh is not implemented yet");
        }
        return persisted(store.clear_resource_cache(), success(None, 200));
    }
    if path == "/api/storage" {
        if method == Method::GET {
            return download(&store.data, "sub-store_data");
        }
        if method == Method::POST {
            let restored = body.into_json()["content"]
                .as_str()
                .ok_or_else(|| "Missing content".to_owned())
                .and_then(decode);
            let Ok(mut next) = restored else {
                return invalid_backup();
            };
            let original = next.clone();
            if !next["settings"].is_object() || migrate(&mut next).is_err() || !valid_records(&next)
            {
                return invalid_backup();
            }
            return persisted(store.restore(next, original), success(None, 200));
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
        return not_found();
    };
    let subscription = key == "subs";
    if path == plural && method == Method::GET {
        return success(store.data.get(key).cloned(), 200);
    }
    let name = path.strip_prefix(singular).unwrap_or("");
    let mut position = None;
    let mut null_record = false;
    if path != plural {
        for (index, item) in store.data[key].as_array().into_iter().flatten().enumerate() {
            if item.is_null() {
                null_record = true;
                break;
            }
            if item.get("name").is_some_and(|value| value == name) {
                position = Some(index);
                break;
            }
        }
    }
    if path != plural && method == Method::GET {
        if null_record {
            return null_name_error(false);
        }
        return if let Some(position) = position {
            if subscription {
                // Legacy GET cleans the live object; its next write persists the change.
                store.data[key][position]
                    .as_object_mut()
                    .unwrap()
                    .remove("subscriptions");
            }
            let item = store.data[key][position].clone();
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
        } else if !subscription {
            send(
                500,
                "text/plain; charset=utf-8",
                json!({"status":"failed", "error":{
                    "code":"SUBSCRIPTION_NOT_FOUND", "type":"ResourceNotFoundError",
                    "message":format!("{label} {name} does not exist"), "details":404
                }})
                .to_string(),
            )
        } else {
            html_error(500, "Internal Server Error")
        };
    }
    let mut next = store.data.clone();
    if next.get(key).is_none() {
        next[key] = json!([]);
    }
    if path == plural && method == Method::PUT {
        let Some(mut items) = body.into_json().as_array().cloned() else {
            return invalid("INVALID_DATA", "Expected an array");
        };
        for item in &mut items {
            if let Err(message) = normalize_item(item, false) {
                return invalid("INVALID_AGE_PUBLIC_KEY", message);
            }
        }
        next[key] = Value::Array(items);
        return publish(store, next, success(None, 200));
    }
    if path == plural && method == Method::POST {
        let mut item = if subscription {
            Value::Object(body.fields())
        } else {
            body.into_json()
        };
        if let Err(message) = normalize_item(&mut item, subscription) {
            return invalid("INVALID_AGE_PUBLIC_KEY", message);
        }
        let item_name = item
            .get("name")
            .map(js_string)
            .unwrap_or_else(|| "undefined".to_owned());
        if item_name.contains('/') {
            return invalid("INVALID_NAME", &format!("{label} {item_name} is invalid"));
        }
        for old in next[key].as_array().unwrap() {
            if old.is_null() {
                return null_name_error(true);
            }
            let duplicate = match (old.get("name"), item.get("name")) {
                (Some(Value::Number(old)), Some(Value::Number(new))) => {
                    old.as_f64() == new.as_f64()
                }
                (_, Some(Value::Array(_) | Value::Object(_))) => false,
                (old, new) => old == new,
            };
            if duplicate {
                return invalid(
                    "DUPLICATE_KEY",
                    &format!("{label} {item_name} already exists."),
                );
            }
        }
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
        if method == Method::DELETE
            && let Some(mode) = query.get("mode")
        {
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
        if null_record {
            return null_name_error(method == Method::DELETE);
        }
        let Some(position) = position else {
            return failure(
                if method == Method::DELETE { 500 } else { 404 },
                "RESOURCE_NOT_FOUND",
                "ResourceNotFoundError",
                &format!("{label} {name} does not exist!"),
            );
        };
        if method == Method::DELETE {
            next[key].as_array_mut().unwrap().remove(position);
            if subscription {
                for collection in next["collections"].as_array_mut().into_iter().flatten() {
                    if let Some(names) = collection
                        .get_mut("subscriptions")
                        .and_then(Value::as_array_mut)
                    {
                        names.retain(|value| value != name);
                    }
                }
            }
            return publish(store, next, success(None, 200));
        }
        let mut patch = body.fields();
        if subscription {
            patch.remove("subscriptions");
        }
        if !patch.get("name").is_some_and(truthy) {
            patch.insert("name".to_owned(), json!(name));
        }
        let mut item = next[key][position].clone();
        item.as_object_mut().unwrap().extend(patch);
        if normalize_item(&mut item, false).is_err() {
            // The legacy PATCH handler lets this error reach Express's HTML handler.
            return html_error(500, "[object Object]");
        }
        let new_name = item["name"].clone();
        let source_type = if subscription {
            "subscription"
        } else {
            "collection"
        };
        if new_name != name {
            if subscription {
                for collection in next["collections"].as_array_mut().into_iter().flatten() {
                    if let Some(names) = collection
                        .get_mut("subscriptions")
                        .and_then(Value::as_array_mut)
                        && let Some(index) = names.iter().position(|value| value == name)
                    {
                        names[index] = new_name.clone();
                    }
                }
            }
            for artifact in next["artifacts"].as_array_mut().into_iter().flatten() {
                if artifact["type"] == source_type
                    && reference_name_matches(&artifact["source"], name, subscription)
                {
                    artifact["source"] = new_name.clone();
                }
            }
            for file in next["files"].as_array_mut().into_iter().flatten() {
                if file["sourceType"] == source_type
                    && reference_name_matches(&file["sourceName"], name, subscription)
                {
                    file["sourceName"] = new_name.clone();
                }
            }
        }
        next[key][position] = item.clone();
        return publish(store, next, success(Some(item), 200));
    }
    not_found()
}
