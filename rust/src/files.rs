use crate::{
    api,
    body::Body,
    json as codec,
    script,
    state::{Store, truthy},
};
use hyper::Method;
use serde_json::{Value, json};
use std::{
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};

fn normalize(file: &mut Value) -> Result<(), &'static str> {
    if file["type"] == "mihomoProfile" {
        file["type"] = json!("mihomoConfig");
    }
    api::normalize_item(file, false)
}

fn last(value: &Value) -> &Value {
    value
        .as_array()
        .and_then(|values| values.last())
        .unwrap_or(value)
}

fn supplied(value: &Value) -> bool {
    let value = last(value);
    !value.is_null() && value != ""
}

fn error(
    share: bool,
    status: u16,
    code: &str,
    kind: &str,
    message: &str,
    details: Option<Value>,
) -> api::Reply {
    api::failure_details(
        status,
        code,
        kind,
        message,
        if share {
            Some(json!("详情请查看日志"))
        } else {
            details
        },
    )
}

fn missing(name: &str, status: u16) -> api::Reply {
    api::failure(
        status,
        "RESOURCE_NOT_FOUND",
        "ResourceNotFoundError",
        &format!("File {name} does not exist!"),
    )
}

pub async fn dispatch(
    method: &Method,
    path: &str,
    query: &Value,
    body: &Body,
    store: &Arc<Mutex<Store>>,
    request: &Value,
) -> Option<api::Reply> {
    if path == "/api/preview/file" && method == Method::POST {
        let mut file = Value::Object(body.fields());
        if file["type"] == "mihomoProfile" {
            file["type"] = json!("mihomoConfig");
        }
        return Some(match script::run(store.clone(), "preview-file", file).await {
            Ok(value) => api::success(Some(value), 200),
            Err(crate::conversion::Error::Legacy(reason)) => error(
                false,
                500,
                "INTERNAL_SERVER_ERROR",
                "InternalServerError",
                "Failed to preview file",
                Some(json!(format!("Reason: {reason}"))),
            ),
        });
    }
    if (path.starts_with("/api/file/") || path.starts_with("/share/file/"))
        && method == Method::GET
    {
        let share = path.starts_with("/share/");
        let name = path.split_once("/file/").unwrap().1;
        let fake = truthy(&query["fakeFile"]);
        let invalid = if share && fake {
            Some((
                "UNSUPPORTED_SHARE_FAKE_FILE",
                "share/file 不支持 fakeFile 参数",
            ))
        } else if fake
            && !(supplied(&query["content"])
                || supplied(&query["url"])
                || matches!(
                    last(&query["type"]).as_str(),
                    Some("mihomoProfile" | "mihomoConfig")
                ) && (last(&query["sourceType"]) == "none"
                    || matches!(
                        last(&query["sourceType"]).as_str(),
                        Some("subscription" | "collection")
                    ) && supplied(&query["sourceName"])))
        {
            Some((
                "INVALID_FAKE_FILE_SOURCE",
                "fakeFile 需要提供 content/url 或 mihomo 配置来源参数",
            ))
        } else if share
            && (!query["url"].is_null() && query["url"] != ""
                || !query["content"].is_null() && query["content"] != "")
        {
            Some((
                "UNSUPPORTED_SHARE_FILE_SOURCE_OVERRIDE",
                "share/file 不支持 url 或 content 参数",
            ))
        } else if share
            && ["type", "source", "sourceType", "sourceName", "mode"]
                .iter()
                .any(|key| supplied(&query[*key]))
        {
            Some((
                "UNSUPPORTED_SHARE_FILE_RUNTIME_OVERRIDE",
                "share/file 不支持文件来源配置覆盖参数",
            ))
        } else if share && !query["subInfoUrl"].is_null() && query["subInfoUrl"] != "" {
            Some((
                "UNSUPPORTED_SHARE_FILE_SUB_INFO_URL",
                "share/file 不支持 subInfoUrl 参数",
            ))
        } else if share && truthy(&query["mergeSources"]) {
            Some((
                "UNSUPPORTED_SHARE_FILE_MERGE_SOURCES",
                "share/file 不支持 mergeSources 参数",
            ))
        } else {
            None
        };
        if let Some((code, message)) = invalid {
            return Some(error(
                share,
                400,
                code,
                "RequestInvalidError",
                message,
                None,
            ));
        }
        let mut file = if fake {
            json!({"name":"fakeFile", "source":"remote", "url":""})
        } else {
            let store = store.lock().unwrap();
            let Some(file) = store
                .list("files")
                .and_then(|items| items.iter().find(|file| file["name"] == name))
            else {
                return Some(error(
                    share,
                    404,
                    "RESOURCE_NOT_FOUND",
                    "ResourceNotFoundError",
                    &format!("File {name} does not exist!"),
                    None,
                ));
            };
            file.clone()
        };
        for key in [
            "content",
            "download",
            "ignoreFailedRemoteFile",
            "mergeSources",
            "noCache",
            "produceType",
            "proxy",
            "subInfoUrl",
            "subInfoUserAgent",
            "type",
            "ua",
            "url",
            "source",
        ] {
            if supplied(&query[key]) {
                file[key] = last(&query[key]).clone();
            }
        }
        if file["type"] == "mihomoProfile" {
            file["type"] = json!("mihomoConfig");
        }
        if file["type"] == "mihomoConfig" {
            for key in ["sourceType", "sourceName", "mode"] {
                if supplied(&query[key]) {
                    file[key] = last(&query[key]).clone();
                }
            }
            if !truthy(&file["sourceType"]) {
                if supplied(&query["url"]) {
                    file["sourceType"] = json!("remote");
                } else if supplied(&query["content"]) {
                    file["sourceType"] = json!("local");
                }
            }
        }
        if supplied(&query["includeUnsupportedProxy"]) {
            file["includeUnsupportedProxy"] = last(&query["includeUnsupportedProxy"]).clone();
        }
        let mut request = request.clone();
        request["params"] = json!({"name":name});
        return Some(
            match script::run(
                store.clone(),
                "download-file",
                json!({"item":file,"request":request}),
            )
            .await
            .and_then(api::script_response)
            {
                Ok(response) => response,
                Err(crate::conversion::Error::Legacy(reason)) => error(
                    share,
                    500,
                    "INTERNAL_SERVER_ERROR",
                    "InternalServerError",
                    &format!("Failed to download file: {name}"),
                    Some(json!(format!("Reason: {reason}"))),
                ),
            },
        );
    }
    if (path == "/api/files" && matches!(*method, Method::GET | Method::POST | Method::PUT))
        || (path == "/api/wholefiles" && method == Method::GET)
        || (path.starts_with("/api/wholefile/") && method == Method::GET)
        || (path.starts_with("/api/file/") && matches!(*method, Method::PATCH | Method::DELETE))
    {
        return Some(manage(
            method,
            path,
            query,
            body,
            &mut store.lock().unwrap(),
        ));
    }
    None
}

fn manage(method: &Method, path: &str, query: &Value, body: &Body, store: &mut Store) -> api::Reply {
    if path == "/api/wholefiles" {
        return api::success(Some(store.data["files"].clone()), 200);
    }
    if path == "/api/files" && method == Method::PUT {
        let mut files = body.clone().into_json();
        let Some(items) = files.as_array_mut() else {
            return api::caught_error("r.map is not a function");
        };
        for file in items {
            if let Err(message) = normalize(file) {
                return api::failure(
                    500,
                    "INVALID_AGE_PUBLIC_KEY",
                    "RequestInvalidError",
                    message,
                );
            }
        }
        let mut next = store.data.clone();
        next["files"] = files;
        return api::persisted(store.commit(next), api::success(None, 200));
    }
    let Some(files) = store.list("files") else {
        return api::uncaught_error(500, "TypeError: e.find is not a function");
    };
    if path == "/api/files" && method == Method::GET {
        let mut metadata = Vec::new();
        for file in files {
            if file.is_null() {
                return api::uncaught_error(
                    500,
                    "TypeError: Cannot convert undefined or null to object",
                );
            }
            let mut fields = Body::Json(file.clone()).fields();
            fields.shift_remove("content");
            metadata.push(Value::Object(fields));
        }
        return api::success(Some(json!(metadata)), 200);
    }
    if path == "/api/files" && method == Method::POST {
        let mut file = Value::Object(body.fields());
        if let Err(message) = normalize(&mut file) {
            return api::failure(
                500,
                "INVALID_AGE_PUBLIC_KEY",
                "RequestInvalidError",
                message,
            );
        }
        let name = if file["name"].is_null() {
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
                .to_string()
        } else {
            codec::js_string(&file["name"])
        };
        file["name"] = json!(name);
        if files.iter().any(|old| old["name"] == file["name"]) {
            let message = if truthy(&Value::Object(body.fields())["name"]) {
                format!("已存在 name 为 {name} 的文件")
            } else {
                "无法同时创建相同的文件 可稍后重试".to_owned()
            };
            return api::failure(500, "DUPLICATE_KEY", "RequestInvalidError", &message);
        }
        let mut next = store.data.clone();
        let top = next["settings"]["appearanceSetting"]["createItemPosition"] == "top";
        let files = next["files"].as_array_mut().unwrap();
        if top {
            files.insert(0, file.clone());
        } else {
            files.push(file.clone());
        }
        return api::persisted(store.commit(next), api::success(Some(file), 201));
    }
    let whole = path.starts_with("/api/wholefile/");
    let name = path
        .strip_prefix(if whole {
            "/api/wholefile/"
        } else {
            "/api/file/"
        })
        .unwrap_or("");
    if method == Method::DELETE
        && !query["mode"].is_null()
        && query["mode"] != ""
        && query["mode"] != "permanent"
    {
        return if query["mode"] == "archive" {
            api::failure(
                501,
                "FEATURE_NOT_IMPLEMENTED",
                "InternalServerError",
                "Archive deletion is pending",
            )
        } else {
            api::failure(
                500,
                "INVALID_DELETE_MODE",
                "RequestInvalidError",
                &format!(
                    "Unsupported delete mode: {}",
                    codec::js_string(&query["mode"])
                ),
            )
        };
    }
    let Some(index) = files.iter().position(|file| file["name"] == name) else {
        return if whole {
            api::failure_details(
                500,
                "FILE_NOT_FOUND",
                "ResourceNotFoundError",
                &format!("File {name} does not exist"),
                Some(json!(404)),
            )
        } else {
            missing(name, if method == Method::DELETE { 500 } else { 404 })
        };
    };
    if whole {
        return if truthy(&query["raw"]) {
            api::download(&files[index], &format!("sub-store_file_{name}"))
        } else {
            api::success(Some(files[index].clone()), 200)
        };
    }
    let mut next = store.data.clone();
    if method == Method::DELETE {
        next["files"].as_array_mut().unwrap().remove(index);
        return api::persisted(store.commit(next), api::success(None, 200));
    }
    let mut patch = body.fields();
    if !patch.get("name").is_some_and(truthy) {
        patch.insert("name".to_owned(), files[index]["name"].clone());
    }
    let mut fields = Body::Json(files[index].clone()).fields();
    fields.extend(patch);
    let mut file = Value::Object(fields);
    if normalize(&mut file).is_err() {
        return api::html_error(500, "[object Object]");
    }
    if file["name"] != files[index]["name"] {
        for artifact in next["artifacts"].as_array_mut().into_iter().flatten() {
            if artifact["type"] == "file" && artifact["source"] == files[index]["name"] {
                artifact["source"] = file["name"].clone();
            }
        }
    }
    next["files"][index] = file.clone();
    api::persisted(store.commit(next), api::success(Some(file), 200))
}
