use crate::{
    api,
    body::Body,
    cron, json as codec,
    resources::{self, Error},
    script,
    state::{Store, truthy},
};
use hyper::Method;
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};

fn name(item: &Value) -> Result<String, Error> {
    if item.is_null() {
        return Err(Error::plain(
            "Cannot read properties of null (reading 'name')",
        ));
    }
    if api::cannot_coerce(&item["name"]) {
        return Err(Error::plain("Cannot convert object to primitive value"));
    }
    Ok(item
        .get("name")
        .map(codec::js_string)
        .unwrap_or_else(|| "undefined".to_owned()))
}

fn normalize(item: &mut Value) -> Result<(), Error> {
    if api::cannot_coerce(&item["age-public-key"]) {
        return Err(Error::plain("Cannot convert object to primitive value"));
    }
    api::normalize_age(item).map_err(|message| Error::invalid("INVALID_AGE_PUBLIC_KEY", message))
}

fn validate_name(item: &Value) -> Result<(), Error> {
    let name = name(item)?;
    if name == ".sub-store-placeholder"
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte))
    {
        return Err(Error::invalid(
            "INVALID_ARTIFACT_NAME",
            format!("Artifact name {name} is invalid."),
        ));
    }
    Ok(())
}

fn normalize_cron(item: &mut Value) -> Result<(), Error> {
    if !item.is_object() {
        return Ok(());
    }
    if api::cannot_coerce(&item["cron"]) {
        return Err(Error::plain("Cannot convert object to primitive value"));
    }
    let text = if item["cron"].is_null() {
        String::new()
    } else {
        codec::js_string(&item["cron"])
    };
    let text = codec::trim(&text);
    if text.is_empty() {
        item.as_object_mut().unwrap().shift_remove("cron");
    } else {
        cron::parse(text).map_err(|reason| {
            let mut error = Error::invalid(
                "INVALID_ARTIFACT_CRON",
                format!("Artifact cron {text} is invalid."),
            );
            error.details = Some(Box::new(json!(format!("Reason: {reason}"))));
            error
        })?;
        item["cron"] = json!(text);
    }
    Ok(())
}

pub(crate) fn prepare_create(
    store: &Store,
    next: &mut Value,
    mut item: Value,
) -> Result<Value, Error> {
    normalize(&mut item)?;
    validate_name(&item)?;
    normalize_cron(&mut item)?;
    let items = store
        .list("artifacts")
        .ok_or_else(|| Error::plain("e.find is not a function"))?;
    for old in items {
        if old.is_null() {
            return Err(Error::plain(
                "Cannot read properties of null (reading 'name')",
            ));
        }
        let same = match (old.get("name"), item.get("name")) {
            (_, Some(Value::Array(_) | Value::Object(_))) => false,
            (Some(a), Some(b)) => crate::tokens::same(a, b),
            (a, b) => a == b,
        };
        if same && truthy(old) {
            return Err(Error::invalid(
                "DUPLICATE_KEY",
                format!("Artifact {} already exists.", name(&item)?),
            ));
        }
    }
    let top = next["settings"]["appearanceSetting"]["createItemPosition"] == "top";
    let items = next["artifacts"].as_array_mut().ok_or_else(|| {
        Error::plain(if top {
            "e.unshift is not a function"
        } else {
            "e.push is not a function"
        })
    })?;
    if top {
        items.insert(0, item.clone());
    } else {
        items.push(item.clone());
    }
    Ok(item)
}

pub(crate) async fn upload(store: &Arc<Mutex<Store>>, files: Value) -> Result<Value, Error> {
    let response = script::run(store.clone(), "artifact-upload", files)
        .await
        .map_err(|crate::conversion::Error::Legacy(reason)| Error::plain(reason))?;
    let body = response["body"]
        .as_str()
        .and_then(|body| codec::parse(body).ok())
        .unwrap_or_else(|| json!({}));
    let url = body
        .get("html_url")
        .filter(|value| !value.is_null())
        .unwrap_or(&body["web_url"]);
    let mut store = store.lock().unwrap();
    let mut next = store.data.clone();
    if !truthy(&next["settings"]) {
        next["settings"] = json!({});
    }
    if let Some(settings) = next["settings"].as_object_mut() {
        if truthy(url) {
            settings.insert("artifactStore".to_owned(), url.clone());
        }
        settings.insert(
            "artifactStoreStatus".to_owned(),
            json!(if truthy(url) { "VALID" } else { "NOT FOUND" }),
        );
    }
    store.commit(next).map_err(Error::state)?;
    Ok(response)
}

pub async fn dispatch(
    method: &Method,
    path: &str,
    query: &Value,
    body: &Body,
    store: &Arc<Mutex<Store>>,
) -> Option<api::Reply> {
    if path == "/api/artifacts/restore" && method == Method::GET {
        let result = restore(store).await;
        return Some(match result {
            Ok(()) => api::success(None, 200),
            Err(error) if error.code == "STATE_WRITE_FAILED" => error.reply(500),
            Err(error) => Error::internal("FAILED_TO_RESTORE_ARTIFACTS", "Failed to restore artifacts", error.message).reply(500),
        });
    }
    if path != "/api/artifacts" && !path.starts_with("/api/artifact/") {
        return None;
    }
    if method == Method::DELETE && path.starts_with("/api/artifact/") {
        let result = delete(store, path.strip_prefix("/api/artifact/").unwrap(), query).await;
        return Some(result.map_or_else(
            |error| error.reply(500),
            |data| api::success(Some(data), 200),
        ));
    }
    Some(manage(method, path, body, &mut store.lock().unwrap()))
}

async fn restore(store: &Arc<Mutex<Store>>) -> Result<(), Error> {
    if !truthy(&store.lock().unwrap().data["settings"]["gistToken"]) {
        return Err(Error::plain("未设置 GitHub Token！"));
    }
    let response = crate::settings::job(store, "gist-request", json!({"key":"Sub-Store Artifacts Repository","action":"locate"})).await?;
    let items = crate::settings::job(store, "artifact-restored", response.get("result").cloned().unwrap_or(Value::Null)).await?;
    let mut store = store.lock().unwrap(); let mut next = store.data.clone(); next["artifacts"] = items;
    store.commit(next).map_err(Error::state)
}

fn manage(method: &Method, path: &str, body: &Body, store: &mut Store) -> api::Reply {
    if path == "/api/artifacts" && method == Method::GET {
        return api::success(store.data.get("artifacts").cloned(), 200);
    }
    let result = (|| -> Result<api::Reply, Error> {
        let mut next = store.data.clone();
        if path == "/api/artifacts" && method == Method::POST {
            let item = prepare_create(store, &mut next, body.clone().into_json())?;
            store.commit(next).map_err(Error::state)?;
            return Ok(api::success(Some(item), 201));
        }
        if path == "/api/artifacts" && method == Method::PUT {
            let mut items = body.clone().into_json();
            let list = items
                .as_array_mut()
                .ok_or_else(|| Error::plain("e.forEach is not a function"))?;
            for item in list.iter_mut() {
                normalize(item)?;
            }
            for item in list {
                normalize_cron(item)?;
            }
            next["artifacts"] = items;
            store.commit(next).map_err(Error::state)?;
            return Ok(api::success(None, 200));
        }
        let Some(name) = path.strip_prefix("/api/artifact/") else {
            return Ok(api::not_found());
        };
        if method != Method::GET && method != Method::PATCH {
            return Ok(api::not_found());
        }
        let index = store
            .list("artifacts")
            .unwrap_or_default()
            .iter()
            .position(|item| item["name"] == name);
        let Some(index) = index else {
            return Ok(if method == Method::GET {
                Error::missing(format!("Artifact {name} does not exist!")).reply(404)
            } else {
                Error::invalid("DUPLICATE_KEY", format!("Artifact {name} already exists."))
                    .reply(500)
            });
        };
        if method == Method::GET {
            return Ok(api::success(Some(next["artifacts"][index].clone()), 200));
        }
        let mut patch = body.fields();
        if !patch.get("name").is_some_and(truthy) {
            patch.insert("name".to_owned(), next["artifacts"][index]["name"].clone());
        }
        let mut item = next["artifacts"][index].clone();
        item.as_object_mut().unwrap().extend(patch);
        if let Err(error) = normalize(&mut item) {
            return Ok(if error.kind == "Error" {
                api::uncaught_error(500, &format!("TypeError: {}", error.message))
            } else {
                api::html_error(500, "[object Object]")
            });
        }
        if let Err(error) = validate_name(&item) {
            return if error.kind == "Error" {
                Ok(api::uncaught_error(
                    500,
                    &format!("TypeError: {}", error.message),
                ))
            } else {
                Err(error)
            };
        }
        normalize_cron(&mut item)?;
        next["artifacts"][index] = item.clone();
        store.commit(next).map_err(Error::state)?;
        Ok(api::success(Some(item), 200))
    })();
    result.unwrap_or_else(|error| error.reply(500))
}

async fn delete(store: &Arc<Mutex<Store>>, name: &str, query: &Value) -> Result<Value, Error> {
    let archive = resources::archive_mode(&query["mode"])?;
    let artifact = {
        let mut store = store.lock().unwrap();
        let artifact = store
            .list("artifacts")
            .unwrap_or_default()
            .iter()
            .find(|item| item["name"] == name)
            .cloned()
            .ok_or_else(|| Error::missing(format!("Artifact {name} does not exist!")))?;
        if archive {
            let mut next = store.data.clone();
            crate::archives::insert(&mut next, "artifact", &artifact)?;
            store.commit(next).map_err(Error::state)?;
        }
        artifact
    };
    let mut remote = json!({"attempted":false,"status":"not_attempted"});
    if truthy(&artifact["url"]) || truthy(&artifact["updated"]) && artifact["upload"] != false {
        let raw = codec::js_string(&artifact["name"]);
        let encoded = codec::uri_component(&raw).map_err(|_| {
            Error::internal(
                "FAILED_TO_DELETE_ARTIFACT",
                &format!("Failed to delete artifact {name}"),
                "URIError: URI malformed",
            )
        })?;
        let mut files = json!({});
        files[encoded.clone()] = json!({"content":""});
        if encoded != raw {
            files[raw] = json!({"content":""});
        }
        remote["attempted"] = json!(true);
        match upload(store, files).await {
            Ok(response) => {
                let fallback = &response["subStoreUploadMeta"]["emptyFileFallback"];
                remote["status"] = json!(if fallback["status"] == "created"
                    || fallback["status"] == "retained"
                {
                    "placeholder_retained"
                } else {
                    "deleted"
                });
                if truthy(&fallback["filename"]) {
                    remote["placeholderFilename"] = fallback["filename"].clone();
                }
            }
            Err(error) => {
                remote["status"] = json!("failed");
                remote["message"] = json!(error.message);
            }
        }
    }
    let mut store = store.lock().unwrap();
    let mut next = store.data.clone();
    let items = next["artifacts"].as_array_mut().unwrap();
    if !items.is_empty() {
        let index = items
            .iter()
            .position(|item| item["name"] == name)
            .unwrap_or(items.len() - 1);
        items.remove(index);
    }
    store.commit(next).map_err(Error::state)?;
    Ok(json!({"artifact":artifact,"remote":remote}))
}
