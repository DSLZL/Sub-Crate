use crate::{
    api,
    body::Body,
    json as codec,
    resources::{self, Error},
    script,
    state::{Store, truthy},
};
use hyper::Method;
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};

pub(crate) fn same(a: &Value, b: &Value) -> bool {
    if a.is_object() || a.is_array() || b.is_object() || b.is_array() {
        false
    } else if let (Some(a), Some(b)) = (a.as_f64(), b.as_f64()) {
        a == b
    } else {
        a == b
    }
}

fn sign_error(reason: impl std::fmt::Display) -> Error {
    Error::plain(reason.to_string())
}

fn validate(store: &Store, payload: &Value) -> Result<(), Error> {
    let (kind, name, token) = (&payload["type"], &payload["name"], &payload["token"]);
    if !truthy(kind) || !truthy(name) {
        return Err(Error::invalid(
            "INVALID_PAYLOAD",
            "payload type and name are required",
        ));
    }
    if !token.is_null() {
        if !token.is_string() || token == "" {
            return Err(Error::invalid(
                "INVALID_CUSTOM_TOKEN",
                format!("Invalid custom token: {}", codec::js_string(token)),
            ));
        }
        if store.list("tokens").unwrap_or_default().iter().any(|item| {
            same(&item["token"], token) && same(&item["type"], kind) && same(&item["name"], name)
        }) {
            return Err(Error::invalid(
                "DUPLICATE_TOKEN",
                format!("Token {} already exists", codec::js_string(token)),
            ));
        }
    }
    if api::cannot_coerce(name) {
        return Err(sign_error("Cannot convert object to primitive value"));
    }
    let (key, label, code) = match kind.as_str() {
        Some("col") => ("collections", "collection", "INVALID_COLLECTION"),
        Some("file") => ("files", "file", "INVALID_FILE"),
        Some("sub") => ("subs", "sub", "INVALID_SUB"),
        _ => {
            return Err(Error::invalid(
                "INVALID_TYPE",
                format!("type {} not supported", codec::js_string(name)),
            ));
        }
    };
    if !store
        .list(key)
        .unwrap_or_default()
        .iter()
        .any(|item| same(&item["name"], name))
    {
        return Err(Error::invalid(
            code,
            format!("{label} {} not found", codec::js_string(name)),
        ));
    }
    Ok(())
}

pub(crate) async fn expiration(
    store: &Arc<Mutex<Store>>,
    payload: &Value,
    options: Value,
) -> Result<Value, Error> {
    validate(&store.lock().unwrap(), payload)?;
    match script::run(store.clone(), "token-options", options).await {
        Ok(value) if !value["error"].is_null() => {
            let error = &value["error"];
            Err(Error::invalid(
                error["code"].as_str().unwrap_or("INVALID_EXPIRATION_MODE"),
                codec::js_string(&error["message"]),
            ))
        }
        Ok(value) => Ok(value),
        Err(crate::conversion::Error::Legacy(reason)) => Err(sign_error(reason)),
    }
}

pub(crate) fn prepare_create(
    store: &Store,
    next: &mut Value,
    payload: Value,
    expiration: Value,
) -> Result<Value, Error> {
    // The expiration job yielded; recheck both uniqueness and resource existence.
    validate(store, &payload)?;
    let mut token = payload["token"].clone();
    if !truthy(&token) {
        const ALPHABET: &[u8] = b"_-0123456789abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ";
        loop {
            let mut random = [0u8; 21];
            getrandom::fill(&mut random).map_err(sign_error)?;
            token = json!(
                random
                    .iter()
                    .map(|byte| ALPHABET[(byte & 63) as usize] as char)
                    .collect::<String>()
            );
            if !store.list("tokens").unwrap_or_default().iter().any(|item| {
                same(&item["token"], &token)
                    && same(&item["type"], &payload["type"])
                    && same(&item["name"], &payload["name"])
            }) {
                break;
            }
        }
    }
    let mut item = payload;
    let fields = item.as_object_mut().expect("validated token payload");
    for key in [
        "mode",
        "exp",
        "expiresIn",
        "expiresValue",
        "expiresUnit",
        "count",
        "usedCount",
    ] {
        fields.shift_remove(key);
    }
    if api::cannot_coerce(&item["age-public-key"]) {
        return Err(sign_error("Cannot convert object to primitive value"));
    }
    api::normalize_age(&mut item)
        .map_err(|message| Error::invalid("INVALID_AGE_PUBLIC_KEY", message))?;
    item["token"] = token;
    item["createdAt"] = json!(resources::now());
    item.as_object_mut()
        .unwrap()
        .extend(expiration.as_object().unwrap().clone());
    let mut items = store.list("tokens").unwrap_or_default().to_vec();
    if store.data["settings"]["appearanceSetting"]["createItemPosition"] == "top" {
        items.insert(0, item.clone());
    } else {
        items.push(item.clone());
    }
    next["tokens"] = json!(items);
    Ok(item)
}

pub(crate) async fn create(
    store: &Arc<Mutex<Store>>,
    payload: Value,
    options: Value,
) -> Result<Value, Error> {
    let expiration = expiration(store, &payload, options).await?;
    let mut store = store.lock().unwrap();
    let mut next = store.data.clone();
    let item = prepare_create(&store, &mut next, payload, expiration)?;
    store.commit(next).map_err(Error::state)?;
    Ok(item)
}

pub(crate) fn delete(
    store: &mut Store,
    token: &Value,
    kind: &Value,
    name: &Value,
    archive: bool,
) -> Result<(), Error> {
    let items = store.list("tokens").unwrap_or_default();
    let matching = |item: &Value| {
        same(&item["token"], token) && same(&item["type"], kind) && same(&item["name"], name)
    };
    if !items.iter().any(matching) {
        return Err(Error::missing(format!(
            "Share {}/{}/{} does not exist!",
            codec::js_string(kind),
            codec::js_string(name),
            codec::js_string(token)
        )));
    }
    let kept = items
        .iter()
        .filter(|item| !matching(item))
        .cloned()
        .collect::<Vec<_>>();
    let mut next = store.data.clone();
    if archive {
        crate::archives::insert(
            &mut next,
            "share",
            items.iter().find(|item| matching(item)).unwrap(),
        )?;
    }
    next["tokens"] = json!(kept);
    store.commit(next).map_err(Error::state)
}

fn usable(item: &Value) -> bool {
    if !item["exp"].is_null() && codec::to_number(&item["exp"]) <= resources::now() as f64 {
        return false;
    }
    if item["mode"] != "count" {
        return true;
    }
    let count = codec::to_number(&item["count"]);
    let used = codec::to_number(&item["usedCount"]);
    let safe = |number: f64| {
        number.is_finite() && number.fract() == 0.0 && number.abs() <= 9_007_199_254_740_991.0
    };
    safe(count) && count > 0.0 && safe(used) && used >= 0.0 && used < count
}

pub(crate) fn find(
    store: &Store,
    token: &Value,
    kind: &Value,
    name: &Value,
    pathname: &str,
) -> Option<usize> {
    store.list("tokens")?.iter().position(|item| {
        let prefix = format!(
            "/share/{}/{}",
            codec::js_string(&item["type"]),
            codec::js_string(&item["name"])
        );
        same(&item["token"], token)
            && (!truthy(kind) || same(&item["type"], kind))
            && (!truthy(name) || same(&item["name"], name))
            && (pathname.is_empty() || pathname == prefix || pathname.starts_with(&(prefix + "/")))
            && usable(item)
    })
}

pub(crate) fn consume(
    store: &mut Store,
    token: &Value,
    kind: &Value,
    name: &Value,
    pathname: &str,
) -> Result<Option<Value>, Error> {
    let Some(index) = find(store, token, kind, name, pathname) else {
        return Ok(None);
    };
    let mut item = store.list("tokens").unwrap()[index].clone();
    if item["mode"] == "count" {
        item["usedCount"] = json!(codec::to_number(&item["usedCount"]) + 1.0);
        let mut next = store.data.clone();
        next["tokens"][index] = item.clone();
        store.commit(next).map_err(Error::state)?;
    }
    Ok(Some(item))
}

pub(crate) fn output_key(
    store: &Store,
    request: &Value,
    kind: &str,
    name: &Value,
) -> Option<Value> {
    let path = request["path"].as_str().unwrap_or("");
    if !path.starts_with("/share/") {
        return None;
    }
    let consumed = &request["shareToken"];
    let token = if !consumed.is_null() {
        consumed
    } else if !truthy(&request["query"]["token"]) {
        &Value::Null
    } else {
        find(store, &request["query"]["token"], &json!(kind), name, path)
            .map_or(&Value::Null, |index| &store.list("tokens").unwrap()[index])
    };
    let key = if truthy(&token["age-public-key"]) {
        &token["age-public-key"]
    } else {
        &request["headers"]["x-sub-store-share-age-public-key"]
    };
    let key = if key.is_null() {
        String::new()
    } else {
        codec::js_string(key)
    };
    let key = codec::trim(&key);
    (!key.is_empty()).then(|| json!(key))
}

pub(crate) async fn dispatch(
    method: &Method,
    path: &str,
    query: &Value,
    body: &Body,
    store: &Arc<Mutex<Store>>,
) -> Option<api::Reply> {
    if path == "/api/token" && method == Method::POST {
        let body = Value::Object(body.fields());
        return Some(
            match create(store, body["payload"].clone(), body["options"].clone()).await {
                Ok(item) => api::success(Some(json!({"token":item["token"]})), 200),
                Err(error) if error.kind == "Error" => {
                    Error::internal("TOKEN_SIGN_FAILED", "Failed to sign token", error.message)
                        .reply(500)
                }
                Err(error) => error.reply(500),
            },
        );
    }
    if path == "/api/tokens" && method == Method::GET {
        let store = store.lock().unwrap();
        let items = store
            .list("tokens")
            .unwrap_or_default()
            .iter()
            .filter(|item| {
                (!truthy(&query["type"]) || same(&item["type"], &query["type"]))
                    && (!truthy(&query["name"]) || same(&item["name"], &query["name"]))
            })
            .cloned()
            .collect::<Vec<_>>();
        return Some(api::success(Some(json!(items)), 200));
    }
    if let Some(token) = path.strip_prefix("/api/token/")
        && method == Method::DELETE
    {
        return Some((|| {
            if !truthy(&query["type"]) || !truthy(&query["name"]) {
                return Err(Error::invalid("INVALID_PAYLOAD", "Payload type and name are required. Please update your front-end(version >= 2.15.76)"));
            }
            let archive = resources::archive_mode(&query["mode"])?;
            delete(&mut store.lock().unwrap(), &json!(token), &query["type"], &query["name"], archive)?;
            Ok(api::success(None, 200))
        })().unwrap_or_else(|error: Error| error.reply(500)));
    }
    None
}
