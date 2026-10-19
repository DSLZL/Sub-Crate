use crate::body::Body;
use crate::conversion::Error as ConversionError;
use crate::json::{self as codec, js_string, to_number as number};
use crate::state::{Store, decode, legacy_object_order, migrate, truthy, valid_records};
use base64::Engine;
use http_body_util::Full;
use hyper::{HeaderMap, Method, Response, body::Bytes};
use percent_encoding::{percent_decode_str, utf8_percent_encode};
use serde_json::{Value, json};
use std::io;
use std::sync::{Arc, Mutex};

pub type Reply = Response<Full<Bytes>>;

#[derive(Clone)]
pub struct TransformedResponse;

fn script_response(output: Value) -> Result<Reply, ConversionError> {
    let status = output["status"].as_f64().unwrap_or(200.0) as u16;
    let body = &output["body"];
    let binary = output["binary"] == true;
    let bytes = if binary {
        base64::engine::general_purpose::STANDARD
            .decode(body.as_str().unwrap_or(""))
            .map_err(|error| ConversionError::Legacy(error.to_string()))?
    } else if body.is_null() {
        Vec::new()
    } else {
        body.as_str()
            .map_or_else(|| codec::stringify(body), str::to_owned)
            .into_bytes()
    };
    let default_type = if binary {
        "application/octet-stream"
    } else if body.is_string() {
        "text/html; charset=utf-8"
    } else {
        "application/json; charset=utf-8"
    };
    let content_type = output["headers"]["content-type"]
        .as_str()
        .unwrap_or(default_type);
    let mut response = send(status, content_type, bytes);
    if let Some(headers) = output["headers"].as_object() {
        for (key, value) in headers {
            if value.is_null() || key == "content-length" {
                continue;
            }
            let key = hyper::header::HeaderName::from_bytes(key.as_bytes())
                .map_err(|error| ConversionError::Legacy(error.to_string()))?;
            let values = value
                .as_array()
                .map_or_else(|| vec![value], |values| values.iter().collect());
            response.headers_mut().remove(&key);
            for value in values {
                let value = hyper::header::HeaderValue::from_str(&js_string(value))
                    .map_err(|error| ConversionError::Legacy(error.to_string()))?;
                response.headers_mut().append(key.clone(), value);
            }
        }
    }
    if matches!(status, 204 | 304) {
        for key in ["content-type", "content-length", "transfer-encoding"] {
            response.headers_mut().remove(key);
        }
        *response.body_mut() = Full::new(Bytes::new());
    }
    response.extensions_mut().insert(TransformedResponse);
    Ok(response)
}

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

pub fn route_not_found(method: &Method, path: &str) -> Reply {
    html_error(404, &escape_html(&format!("Cannot {method} {path}")))
}

fn escape_html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

pub fn greeting() -> Reply {
    send(
        200,
        "text/plain; charset=utf-8",
        "Hello from sub-store, made with ❤️ by Peng-YM",
    )
}

pub fn options(path: &str) -> Reply {
    let methods = match path {
        "/api/settings" => "GET,PATCH,HEAD",
        "/api/storage" => "GET,POST,HEAD",
        "/api/subs" | "/api/collections" => "GET,POST,PUT,HEAD",
        "/api/proxy/parse" | "/api/rule/parse" | "/api/preview/sub" | "/api/preview/collection" => {
            "POST,GET,HEAD"
        }
        _ if path.starts_with("/api/sub/") || path.starts_with("/api/collection/") => {
            "GET,PATCH,DELETE,HEAD"
        }
        // shortcut: remaining route families gain their method inventories with M4.
        _ => "GET,HEAD",
    };
    let mut response = send(200, "text/plain; charset=utf-8", methods);
    response
        .headers_mut()
        .insert("allow", methods.parse().unwrap());
    response
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

pub fn diagnostic(status: u16, detail: &str) -> String {
    if std::env::var("NODE_ENV").as_deref() == Ok("production") {
        return hyper::StatusCode::from_u16(status)
            .unwrap()
            .canonical_reason()
            .unwrap()
            .to_owned();
    }
    let trace = std::backtrace::Backtrace::force_capture();
    escape_html(&format!("{detail}\n{trace}"))
        .replace('\n', "<br>")
        .replace("  ", " &nbsp;")
}

pub fn uncaught_error(status: u16, detail: &str) -> Reply {
    html_error(status, &diagnostic(status, detail))
}

fn caught_error(message: &str) -> Reply {
    send(
        500,
        "text/plain; charset=utf-8",
        codec::stringify(&json!({"status":"failed", "error":{"message":message}})),
    )
}

fn null_name_error(caught: bool) -> Reply {
    if caught {
        caught_error("Cannot read properties of null (reading 'name')")
    } else {
        uncaught_error(
            500,
            "TypeError: Cannot read properties of null (reading 'name')",
        )
    }
}

fn success(data: Option<Value>, status: u16) -> Reply {
    let mut value = json!({"status":"success"});
    if let Some(data) = data {
        value["data"] = legacy_object_order(data);
    }
    // The legacy middleware sets text/plain before Express serializes JSON.
    send(
        status,
        "text/plain; charset=utf-8",
        codec::stringify(&value),
    )
}

pub fn failure(status: u16, code: &str, kind: &str, message: &str) -> Reply {
    send(
        status,
        "text/plain; charset=utf-8",
        codec::stringify(&json!({
            "status":"failed", "error":{"code":code, "type":kind, "message":message}
        })),
    )
}

fn conversion_error(error: ConversionError, context: Option<&str>) -> Reply {
    match error {
        ConversionError::Pending(message) => pending(message),
        ConversionError::Legacy(reason) => {
            if let Some(message) = context {
                send(
                    500,
                    "text/plain; charset=utf-8",
                    codec::stringify(&json!({
                        "status":"failed", "error":{"code":"INTERNAL_SERVER_ERROR",
                        "type":"InternalServerError", "message":message, "details":format!("Reason: {reason}")}
                    })),
                )
            } else {
                caught_error(&reason)
            }
        }
    }
}

fn selected_text(value: Option<&Value>) -> String {
    value.map_or_else(|| "undefined".to_owned(), js_string)
}

fn header_platform(headers: &HeaderMap) -> &'static str {
    let user_agent = headers
        .get("user-agent")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("");
    let ua = user_agent.to_lowercase();
    if user_agent.contains("Quantumult%20X") {
        return "QX";
    }
    if ua.contains("egern") {
        return "Egern";
    }
    for (needle, platform) in [
        ("Surfboard", "Surfboard"),
        ("Surge Mac", "SurgeMac"),
        ("Surge", "Surge"),
        ("Decar", "Loon"),
        ("Loon", "Loon"),
        ("Shadowrocket", "Shadowrocket"),
        ("Stash", "Stash"),
    ] {
        if user_agent.contains(needle) {
            return platform;
        }
    }
    if ua == "meta"
        || (ua.contains("clash") && ua.contains("meta"))
        || ua.contains("clash-verge")
        || ua.contains("flclash")
    {
        return "ClashMeta";
    }
    if ua.contains("clash") {
        return "Clash";
    }
    if ua.contains("v2ray") {
        return "V2Ray";
    }
    if ua.contains("sing-box") || ua.contains("singbox") {
        return "sing-box";
    }
    if headers
        .get("accept")
        .and_then(|value| value.to_str().ok())
        .is_some_and(|accept| accept.starts_with("application/json"))
    {
        "JSON"
    } else {
        "V2Ray"
    }
}

pub async fn conversion_dispatch(
    method: &Method,
    path: &str,
    query: &Value,
    body: &Body,
    headers: &HeaderMap,
    store: &Arc<Mutex<Store>>,
    request: &Value,
) -> Option<Reply> {
    if method == Method::POST && path == "/api/rule/parse" {
        let fields = Value::Object(body.fields());
        let raw = fields
            .get("data")
            .filter(|value| !value.is_null())
            .or_else(|| fields.get("content"));
        let target = selected_text(
            fields
                .get("client")
                .filter(|value| !value.is_null())
                .or_else(|| fields.get("platform")),
        );
        let result = crate::rules::parse(raw)
            .and_then(|mut rules| crate::rules::produce(&mut rules, &target));
        return Some(match result {
            Ok(output) => success(Some(json!({"par_res":output})), 200),
            Err(error) => conversion_error(error, None),
        });
    }
    if method == Method::POST && path == "/api/proxy/parse" {
        let fields = Value::Object(body.fields());
        let result = crate::script::run(store.clone(), "parse", fields).await;
        return Some(match result {
            Ok(output) => success(Some(json!({"par_res":output})), 200),
            Err(error) => conversion_error(error, None),
        });
    }
    if method == Method::POST && matches!(path, "/api/preview/sub" | "/api/preview/collection") {
        let collection = path.ends_with("/collection");
        let target = if truthy(&query["target"]) {
            js_string(&query["target"])
        } else {
            "JSON".to_owned()
        };
        let input = json!({"item":Value::Object(body.fields()),"target":codec::text(&target)});
        return Some(
            match crate::script::run(
                store.clone(),
                if collection {
                    "preview-collection"
                } else {
                    "preview-sub"
                },
                input,
            )
            .await
            {
                Ok(output) => success(Some(output), 200),
                Err(error) => conversion_error(
                    error,
                    Some(if collection {
                        "Failed to preview collection"
                    } else {
                        "Failed to preview subscription"
                    }),
                ),
            },
        );
    }
    if method != Method::GET {
        return None;
    }
    let rest = path.strip_prefix("/download/")?;
    let collection = rest.starts_with("collection/");
    let rest = rest.strip_prefix("collection/").unwrap_or(rest);
    let (name, path_target) = rest
        .split_once('/')
        .map_or((rest, None), |(name, target)| (name, Some(target)));
    let name = codec::encode(&percent_decode_str(name).decode_utf8().ok()?);
    let sub = if !collection && truthy(&query["_fakeNode"]) {
        Some(
            json!({"name":"fakeNodeInfo", "source":"local", "content":"invalid share = ss, 1.0.0.1, 80, encrypt-method=aes-128-gcm, password=password"}),
        )
    } else if !collection && truthy(&query["fakeSub"]) {
        Some(json!({"name":"fakeSub", "source":"remote", "url":""}))
    } else {
        let store = store.lock().unwrap();
        let mut found = None;
        for item in store
            .list(if collection { "collections" } else { "subs" })
            .unwrap_or_default()
        {
            if item.is_null() {
                return Some(uncaught_error(
                    500,
                    "TypeError: Cannot read properties of null (reading 'name')",
                ));
            }
            if reference_name_matches(&item["name"], &name, true) {
                found = Some(item.clone());
                break;
            }
        }
        found
    };
    let Some(sub) = sub else {
        return Some(failure(
            404,
            "RESOURCE_NOT_FOUND",
            "ResourceNotFoundError",
            &format!(
                "{} {} does not exist!",
                if collection {
                    "Collection"
                } else {
                    "Subscription"
                },
                codec::text(&name)
            ),
        ));
    };
    let target = if truthy(&query["platform"]) {
        js_string(&query["platform"])
    } else if let Some(target) = path_target {
        codec::encode(&percent_decode_str(target).decode_utf8().ok()?)
    } else if truthy(&query["target"]) {
        js_string(&query["target"])
    } else {
        header_platform(headers).to_owned()
    };
    let url = query["url"].as_str().filter(|value| !value.is_empty());
    let content = if let Some(url) =
        url.filter(|url| !url.starts_with("http://") && !url.starts_with("https://"))
    {
        Some(json!(url))
    } else {
        query.get("content").filter(|value| truthy(value)).cloned()
    };
    let mut options = query.clone();
    if let Some(fields) = options.as_object_mut() {
        fields.insert("name".to_owned(), json!(name));
        fields.insert("platform".to_owned(), json!(target));
        if let Some(content) = content {
            fields.insert("content".to_owned(), content);
            fields.shift_remove("url");
        }
        let mut produce = serde_json::Map::new();
        for (source, destination) in [
            ("includeUnsupportedProxy", "include-unsupported-proxy"),
            ("prettyYaml", "prettyYaml"),
        ] {
            if let Some(value) = query.get(source) {
                produce.insert(destination.to_owned(), value.clone());
            }
        }
        if !produce.contains_key("prettyYaml")
            && let Some(value) = query.get("pretty-yaml")
        {
            produce.insert("prettyYaml".to_owned(), value.clone());
        }
        let use_external =
            path_target.map_or(query["target"] == "SurgeMac", |target| target == "SurgeMac");
        produce.insert("useMihomoExternal".to_owned(), json!(use_external));
        if use_external {
            for (key, field) in [
                ("mihomoMerge", "merge"),
                ("mihomoMergeName", "mergeName"),
                ("mihomoExternal", "mihomoExternal"),
            ] {
                if let Some(value) = query.get(key) {
                    produce.insert(field.to_owned(), value.clone());
                }
            }
            if let Some(value) = query.get("mihomoLocalPort") {
                let text = js_string(value);
                let text = codec::trim(&text);
                let end = text
                    .char_indices()
                    .skip(usize::from(text.starts_with(['+', '-'])))
                    .find(|(_, ch)| !ch.is_ascii_digit())
                    .map_or(text.len(), |(index, _)| index);
                if let Ok(port) = text[..end].parse::<u16>()
                    && port != 0
                {
                    produce.insert("localPort".to_owned(), json!(port));
                }
            }
        }
        fields.insert("produceOpts".to_owned(), Value::Object(produce));
    }
    let mut request = request.clone();
    let mut params = json!({"name":name});
    if let Some(target) = path_target {
        params["target"] = json!(codec::encode(
            &percent_decode_str(target).decode_utf8().ok()?
        ));
        request["query"]["target"] = params["target"].clone();
    }
    request["params"] = params;
    let result = crate::script::run(
        store.clone(),
        if collection {
            "download-collection"
        } else {
            "download-sub"
        },
        json!({"item":sub,"options":options,"request":request}),
    )
    .await;
    Some(match result.and_then(script_response) {
        Ok(response) => response,
        Err(error) => conversion_error(
            error,
            Some(&format!(
                "Failed to download {}: {}",
                if collection {
                    "collection"
                } else {
                    "subscription"
                },
                codec::text(&name)
            )),
        ),
    })
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
        codec::stringify(&json!({"status":"failed", "error":{
            "code":"INVALID_BACKUP_DATA", "type":"RequestInvalidError",
            "message":"Invalid backup data, failed to restore!",
            "details":"Reason: 备份文件校验失败, 无法还原"
        }})),
    )
}

fn download(value: &Value, name: &str) -> Reply {
    let Ok(name) = String::from_utf16(&codec::units(name)) else {
        return uncaught_error(500, "URIError: URI malformed");
    };
    let stamp = jiff::Zoned::now().strftime("%Y-%m-%d_%H-%M-%S");
    let filename = format!("{name}_{stamp}.json");
    let disposition = format!(
        "attachment; filename=\"{}\"",
        utf8_percent_encode(&filename, codec::COMPONENT)
    );
    let mut response = send(
        200,
        "application/json; charset=utf-8",
        codec::stringify(&legacy_object_order(value.clone())),
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
    let key = codec::trim(&key);
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

const PRIMITIVE_ERROR: &str = "Cannot convert object to primitive value";

fn cannot_coerce(value: &Value) -> bool {
    match value {
        Value::Object(fields) => fields.contains_key("toString"),
        Value::Array(items) => items.iter().any(cannot_coerce),
        _ => false,
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
    let mut settings = Body::Json(next["settings"].clone()).fields();
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
            if cannot_coerce(raw) {
                return send(
                    500,
                    "text/plain; charset=utf-8",
                    codec::stringify(&json!({"status":"failed", "error":{
                        "code":"FAILED_TO_UPDATE_SETTINGS", "type":"InternalServerError",
                        "message":"Failed to update settings",
                        "details":format!("Reason: {PRIMITIVE_ERROR}")
                    }})),
                );
            }
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

fn collection_references(
    store: &Store,
    next: &mut Value,
    name: &str,
    renamed: Option<&Value>,
) -> Result<(), &'static str> {
    if !truthy(&next["collections"]) {
        return Ok(());
    }
    let collections = if next["collections"].is_array() {
        &mut next["collections"]
    } else if store.list("collections").is_some() {
        &mut next["collections"]["data"]
    } else {
        return Err(if renamed.is_some() {
            "a is not iterable"
        } else {
            "n is not iterable"
        });
    };
    for collection in collections.as_array_mut().unwrap() {
        if renamed.is_some() && collection["subscriptions"].is_string() {
            // The frozen CJS bundle silently ignores assignment to a string index.
            continue;
        }
        let error = if collection.is_null() {
            "Cannot read properties of null (reading 'subscriptions')"
        } else if renamed.is_some() {
            match collection.get("subscriptions") {
                None => "Cannot read properties of undefined (reading 'indexOf')",
                Some(Value::Null) => "Cannot read properties of null (reading 'indexOf')",
                _ => "i.subscriptions.indexOf is not a function",
            }
        } else {
            match collection.get("subscriptions") {
                None => "Cannot read properties of undefined (reading 'filter')",
                Some(Value::Null) => "Cannot read properties of null (reading 'filter')",
                _ => "i.subscriptions.filter is not a function",
            }
        };
        let Some(names) = collection
            .get_mut("subscriptions")
            .and_then(Value::as_array_mut)
        else {
            return Err(error);
        };
        if let Some(renamed) = renamed {
            if let Some(index) = names.iter().position(|value| value == name) {
                names[index] = renamed.clone();
            }
        } else {
            names.retain(|value| value != name);
        }
    }
    Ok(())
}

pub fn dispatch(
    method: &Method,
    path: &str,
    query: &Value,
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
                .and_then(|text| decode(&codec::text(text)));
            let Ok(mut next) = restored else {
                return invalid_backup();
            };
            let original = next.clone();
            if next["settings"].is_null() || migrate(&mut next).is_err() || !valid_records(&next) {
                return invalid_backup();
            }
            return persisted(store.restore(next, original), success(None, 200));
        }
    }
    if path == "/api/settings" {
        if method == Method::GET {
            if !truthy(&store.data["settings"]) {
                let mut next = store.data.clone();
                next["settings"] = json!({});
                return publish(store, next, success(Some(json!({})), 200));
            }
            return success(Some(store.data["settings"].clone()), 200);
        }
        if method == Method::PATCH {
            return settings(store, body);
        }
    }
    if method == Method::GET {
        let key = match path {
            "/api/artifacts" => Some("artifacts"),
            "/api/wholefiles" | "/api/files" => Some("files"),
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
    let lookup_failed = path != plural && store.list(key).is_none();
    if path != plural {
        for (index, item) in store.list(key).into_iter().flatten().enumerate() {
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
        if lookup_failed {
            return uncaught_error(500, "TypeError: e.find is not a function");
        }
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
            if truthy(&query["raw"]) {
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
                codec::stringify(&json!({"status":"failed", "error":{
                    "code":"SUBSCRIPTION_NOT_FOUND", "type":"ResourceNotFoundError",
                    "message":format!("{label} {name} does not exist"), "details":404
                }})),
            )
        } else {
            uncaught_error(500, "TypeError: Cannot convert undefined or null to object")
        };
    }
    let mut next = store.data.clone();
    if next.get(key).is_none() {
        next[key] = json!([]);
    }
    if path == plural && method == Method::PUT {
        let raw = matches!(body, Body::Raw(_));
        let mut items = body.into_json();
        if let Some(items) = items.as_array_mut() {
            for item in items {
                if cannot_coerce(&item["age-public-key"]) {
                    return caught_error(PRIMITIVE_ERROR);
                }
                if let Err(message) = normalize_item(item, false) {
                    return invalid("INVALID_AGE_PUBLIC_KEY", message);
                }
            }
        } else if !raw {
            // The frozen minified Node bundle names this local variable r on both routes.
            return caught_error("r.forEach is not a function");
        }
        next[key] = items;
        return persisted(store.commit_list(next, key, raw), success(None, 200));
    }
    if path == plural && method == Method::POST {
        let mut item = if subscription {
            Value::Object(body.fields())
        } else {
            body.into_json()
        };
        if cannot_coerce(&item["age-public-key"]) {
            return caught_error(PRIMITIVE_ERROR);
        }
        if let Err(message) = normalize_item(&mut item, subscription) {
            return invalid("INVALID_AGE_PUBLIC_KEY", message);
        }
        if cannot_coerce(&item["name"]) {
            return caught_error(PRIMITIVE_ERROR);
        }
        let item_name = item
            .get("name")
            .map(js_string)
            .unwrap_or_else(|| "undefined".to_owned());
        if item_name.contains('/') {
            return invalid("INVALID_NAME", &format!("{label} {item_name} is invalid"));
        }
        let Some(items) = store.list(key) else {
            return caught_error("e.find is not a function");
        };
        for old in items {
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
                if truthy(old) {
                    return invalid(
                        "DUPLICATE_KEY",
                        &format!("{label} {item_name} already exists."),
                    );
                }
                break;
            }
        }
        let top = next["settings"]["appearanceSetting"]["createItemPosition"] == "top";
        let Some(items) = next[key].as_array_mut() else {
            return caught_error(if top {
                "e.unshift is not a function"
            } else {
                "e.push is not a function"
            });
        };
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
            if mode != "" && mode != "permanent" {
                if cannot_coerce(mode) {
                    return caught_error(PRIMITIVE_ERROR);
                }
                return invalid(
                    "INVALID_DELETE_MODE",
                    &format!("Unsupported delete mode: {}", js_string(mode)),
                );
            }
        }
        if lookup_failed {
            return if method == Method::DELETE {
                caught_error("e.find is not a function")
            } else {
                uncaught_error(500, "TypeError: e.find is not a function")
            };
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
            if subscription
                && let Err(message) = collection_references(store, &mut next, name, None)
            {
                return caught_error(message);
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
        if cannot_coerce(&item["age-public-key"]) {
            return uncaught_error(500, &format!("TypeError: {PRIMITIVE_ERROR}"));
        }
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
            if subscription
                && let Err(message) = collection_references(store, &mut next, name, Some(&new_name))
            {
                return uncaught_error(500, &format!("TypeError: {message}"));
            }
            for artifact in next["artifacts"].as_array_mut().into_iter().flatten() {
                if subscription
                    && artifact["type"] == source_type
                    && cannot_coerce(&artifact["source"])
                {
                    return uncaught_error(500, &format!("TypeError: {PRIMITIVE_ERROR}"));
                }
                if artifact["type"] == source_type
                    && reference_name_matches(&artifact["source"], name, subscription)
                {
                    artifact["source"] = new_name.clone();
                }
            }
            for file in next["files"].as_array_mut().into_iter().flatten() {
                if subscription
                    && file["sourceType"] == source_type
                    && cannot_coerce(&file["sourceName"])
                {
                    return uncaught_error(500, &format!("TypeError: {PRIMITIVE_ERROR}"));
                }
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
