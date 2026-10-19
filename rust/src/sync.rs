//! Artifact sync owns generation, provider upload and the resulting resource writes.
use crate::{
    api, artifacts, json as codec, logs,
    resources::{self, Error},
    script,
    state::{Store, truthy},
};
use hyper::Method;
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
use tokio::task::JoinSet;

async fn job(store: &Arc<Mutex<Store>>, operation: &str, input: Value) -> Result<Value, Error> {
    script::run(store.clone(), operation, input)
        .await
        .map_err(|crate::conversion::Error::Legacy(reason)| Error::plain(reason))
}

fn name(item: &Value) -> String {
    item.get("name")
        .map(codec::js_string)
        .unwrap_or_else(|| "undefined".to_owned())
}

fn mark(item: &mut Value, updated: u64, result: &Value) {
    if let Some(item) = item.as_object_mut() {
        item.insert("updated".to_owned(), json!(updated));
        if let Some(url) = result.get("url") {
            item.insert("url".to_owned(), url.clone());
        } else {
            item.shift_remove("url");
        }
    }
}

pub(crate) async fn one(store: &Arc<Mutex<Store>>, requested: &str) -> Result<Value, Error> {
    let snapshot = store
        .lock()
        .unwrap()
        .list("artifacts")
        .unwrap_or_default()
        .iter()
        .find(|item| item["name"] == requested)
        .cloned();
    let snapshot = snapshot.ok_or_else(|| {
        let message = format!("找不到远程配置 {requested}");
        logs::error(&mut store.lock().unwrap(), &message);
        Error::missing(message)
    })?;
    if !truthy(&snapshot["source"]) {
        let mut error = Error::missing(format!("远程配置 {} 未设置来源", log_name(&snapshot)));
        logs::error(&mut store.lock().unwrap(), &error.message);
        error.code = "RESOURCE_HAS_NO_SOURCE".to_owned();
        return Err(error);
    }
    logs::info(
        &mut store.lock().unwrap(),
        &format!("开始同步远程配置 {}...", log_name(&snapshot)),
    );
    if snapshot["platform"] == "SurgeMac" {
        logs::info(
            &mut store.lock().unwrap(),
            "手动指定了 target 为 SurgeMac, 将使用 mihomo External",
        );
    }
    let content = job(store, "artifact-output", snapshot.clone()).await?;
    let mut updated = resources::now();
    let result = if snapshot["upload"] == false {
        logs::info(
            &mut store.lock().unwrap(),
            &format!("配置 {} 已关闭上传, 仅更新执行时间", log_name(&snapshot)),
        );
        json!({})
    } else {
        logs::info(
            &mut store.lock().unwrap(),
            &format!(
                "正在上传配置：{}\n>>>{}",
                log_name(&snapshot),
                String::from_utf8_lossy(&codec::pretty(&snapshot))
            ),
        );
        let encoded =
            codec::uri_component(requested).map_err(|_| Error::plain("URIError: URI malformed"))?;
        let mut files = json!({});
        files[encoded] = content;
        let response = artifacts::upload(store, files).await?;
        updated = resources::now();
        job(
            store,
            "artifact-upload-result",
            json!({"body":response["body"],"name":requested,"logResponse":true}),
        )
        .await?
    };
    let mut store = store.lock().unwrap();
    let mut next = store.data.clone();
    let mut fallback = snapshot;
    let item = next["artifacts"]
        .as_array_mut()
        .and_then(|items| items.iter_mut().find(|item| item["name"] == requested));
    let response = if let Some(item) = item {
        mark(item, updated, &result);
        item.clone()
    } else {
        logs::info(
            &mut store,
            &format!("远程配置 {requested} 已不存在, 跳过同步结果写入"),
        );
        mark(&mut fallback, updated, &result);
        fallback
    };
    if next != store.data {
        store.commit(next).map_err(Error::state)?;
    }
    Ok(response)
}

pub(crate) fn log_name(item: &Value) -> String {
    let raw = item
        .get("name")
        .filter(|value| !value.is_null())
        .map(codec::js_string)
        .unwrap_or_default();
    let display = item
        .get("displayName")
        .filter(|value| !value.is_null())
        .or_else(|| item.get("display-name"));
    let display = display
        .filter(|value| !value.is_null())
        .map(codec::js_string)
        .unwrap_or_default();
    let display = codec::trim(&display);
    if display.is_empty() || display == raw {
        raw
    } else {
        format!("{raw} ({display})")
    }
}

fn selected(item: &Value, skip_cron: bool) -> bool {
    truthy(&item["sync"])
        && truthy(&item["source"])
        && (!skip_cron
            || item
                .get("cron")
                .filter(|value| !value.is_null())
                .is_none_or(|value| codec::trim(&codec::js_string(value)).is_empty()))
}

pub(crate) async fn all(store: &Arc<Mutex<Store>>, skip_cron: bool) -> Result<(), Error> {
    logs::info(&mut store.lock().unwrap(), "开始同步所有远程配置...");
    let result = all_inner(store, skip_cron).await;
    if let Err(error) = &result {
        logs::error(
            &mut store.lock().unwrap(),
            &format!("同步配置失败，原因：{}", error.message),
        );
    }
    result
}

async fn all_inner(store: &Arc<Mutex<Store>>, skip_cron: bool) -> Result<(), Error> {
    let (mut items, settings, subs, collections) = {
        let store = store.lock().unwrap();
        (
            store.list("artifacts").unwrap_or_default().to_vec(),
            store.data["settings"].clone(),
            store.list("subs").unwrap_or_default().to_vec(),
            store.list("collections").unwrap_or_default().to_vec(),
        )
    };
    let enabled = items
        .iter()
        .filter(|item| selected(item, skip_cron))
        .count();
    if enabled == 0 {
        logs::info(
            &mut store.lock().unwrap(),
            &format!("需同步的配置: 0, 总数: {}", items.len()),
        );
        return Ok(());
    }
    let mut warm = Vec::new();
    for item in items.iter().filter(|item| selected(item, skip_cron)) {
        let names = if item["type"] == "subscription" {
            vec![item["source"].clone()]
        } else if item["type"] == "collection" {
            collections
                .iter()
                .find(|col| col["name"] == item["source"])
                .and_then(|col| col["subscriptions"].as_array())
                .cloned()
                .unwrap_or_default()
        } else {
            vec![]
        };
        for name in names {
            if !warm.contains(&name)
                && subs
                    .iter()
                    .any(|sub| sub["name"] == name && truthy(&sub["url"]))
            {
                warm.push(name);
            }
        }
    }
    let mut jobs = JoinSet::new();
    for name in warm {
        let store = store.clone();
        jobs.spawn(async move {
            let _ = job(
                &store,
                "artifact-warm",
                json!({"type":"subscription","name":name,"awaitCustomCache":true}),
            )
            .await;
        });
    }
    while jobs.join_next().await.is_some() {}
    let mut jobs = JoinSet::new();
    for (index, item) in items
        .iter()
        .enumerate()
        .filter(|(_, item)| selected(item, skip_cron))
    {
        let store = store.clone();
        let item = item.clone();
        jobs.spawn(async move {
            logs::info(
                &mut store.lock().unwrap(),
                &format!("正在同步云配置：{}...", log_name(&item)),
            );
            if item["platform"] == "SurgeMac" {
                logs::info(
                    &mut store.lock().unwrap(),
                    "手动指定了 target 为 SurgeMac, 将使用 mihomo External",
                );
            }
            (index, job(&store, "artifact-output", item).await)
        });
    }
    if jobs.is_empty() {
        return Ok(());
    }
    let (mut valid, mut invalid, mut files, mut local) =
        (Vec::new(), Vec::new(), json!({}), Vec::new());
    while let Some(done) = jobs.join_next().await {
        let (index, result) = done.map_err(|error| Error::plain(error.to_string()))?;
        match result {
            Ok(_) if items[index]["upload"] == false => {
                mark(&mut items[index], resources::now(), &json!({}));
                local.push(name(&items[index]));
            }
            Ok(content) => {
                let name = name(&items[index]);
                match codec::uri_component(&name) {
                    Ok(encoded) => {
                        files[encoded] = content;
                        valid.push(name);
                    }
                    Err(_) => {
                        logs::error(
                            &mut store.lock().unwrap(),
                            &format!(
                                "生成同步配置 {} 发生错误: URI malformed",
                                log_name(&items[index])
                            ),
                        );
                        invalid.push(name);
                    }
                }
            }
            Err(error) => {
                logs::error(
                    &mut store.lock().unwrap(),
                    &format!(
                        "生成同步配置 {} 发生错误: {}",
                        log_name(&items[index]),
                        error.message
                    ),
                );
                invalid.push(name(&items[index]));
            }
        }
    }
    logs::info(
        &mut store.lock().unwrap(),
        &format!(
            "{} 个同步配置生成成功: {}",
            valid.len() + local.len(),
            valid
                .iter()
                .chain(&local)
                .cloned()
                .collect::<Vec<_>>()
                .join(", ")
        ),
    );
    logs::info(
        &mut store.lock().unwrap(),
        &format!(
            "{} 个同步配置生成失败: {}",
            invalid.len(),
            invalid.join(", ")
        ),
    );
    if !local.is_empty() {
        logs::info(
            &mut store.lock().unwrap(),
            &format!(
                "{} 个同步配置仅生成未上传: {}",
                local.len(),
                local.join(", ")
            ),
        );
    }
    if valid.is_empty() && local.is_empty() {
        return Err(Error::plain(format!(
            "同步配置 {} 生成失败 详情请查看日志",
            invalid.join(", ")
        )));
    }
    let size = job(
        store,
        "artifact-batch-size",
        settings["artifactSyncBatchSize"].clone(),
    )
    .await?
    .as_f64()
    .unwrap_or(10.0) as usize;
    let mut uploaded = 0usize;
    let batches = valid.len().div_ceil(size);
    if valid.is_empty() {
        logs::info(&mut store.lock().unwrap(), "没有需要上传的同步配置");
    } else {
        logs::info(
            &mut store.lock().unwrap(),
            &format!(
                "准备分批上传同步配置: 共 {} 个, 每批 {size} 个, 批次数 {batches}",
                valid.len()
            ),
        );
    }
    for (index, batch) in valid.chunks(size).enumerate() {
        let mut batch_files = json!({});
        for name in batch {
            let encoded =
                codec::uri_component(name).map_err(|_| Error::plain("URIError: URI malformed"))?;
            batch_files[&encoded] = files[&encoded].clone();
        }
        logs::info(
            &mut store.lock().unwrap(),
            &format!(
                "正在上传第 {}/{batches} 批同步配置: {}",
                index + 1,
                batch.join(", ")
            ),
        );
        let response = artifacts::upload(store, batch_files).await;
        match response {
            Err(error) => {
                logs::error(
                    &mut store.lock().unwrap(),
                    &format!(
                        "第 {}/{batches} 批同步配置上传失败: {}, 原因: {}",
                        index + 1,
                        batch.join(", "),
                        error.message
                    ),
                );
                invalid.extend(batch.iter().cloned());
            }
            Ok(response) => {
                let mut batch_failed = false;
                let mut log_response = true;
                for item in items.iter_mut().filter(|item| {
                    truthy(&item["sync"]) && truthy(&item["source"]) && batch.contains(&name(item))
                }) {
                    match job(
                        store,
                        "artifact-upload-result",
                        json!({"body":response["body"],"name":item["name"],"logResponse":log_response}),
                    )
                    .await
                    {
                        Ok(result) if truthy(&result["url"]) => {
                            mark(item, resources::now(), &result);
                            uploaded += 1;
                        }
                        Ok(_) => {
                            logs::error(&mut store.lock().unwrap(), &format!("同步配置 {} 上传成功但响应中未找到文件链接", name(item)));
                            invalid.push(name(item));
                        }
                        Err(error) => {
                            logs::error(&mut store.lock().unwrap(), &format!("第 {}/{batches} 批同步配置上传失败: {}, 原因: {}", index+1, batch.join(", "), error.message));
                            batch_failed = true;
                            break;
                        }
                    }
                    log_response = false;
                }
                if batch_failed {
                    invalid.extend(batch.iter().cloned());
                }
            }
        }
    }
    {
        let mut store = store.lock().unwrap();
        let mut next = store.data.clone();
        next["artifacts"] = json!(items);
        store.commit(next).map_err(Error::state)?;
    }
    logs::info(&mut store.lock().unwrap(), "同步配置执行完成");
    if invalid.is_empty() {
        logs::info(
            &mut store.lock().unwrap(),
            &format!("同步配置成功 {} 个", uploaded + local.len()),
        );
        Ok(())
    } else {
        Err(Error::plain(format!(
            "同步配置成功 {} 个, 失败 {} 个, 详情请查看日志",
            uploaded + local.len(),
            invalid.len()
        )))
    }
}

pub async fn dispatch(
    method: &Method,
    path: &str,
    store: &Arc<Mutex<Store>>,
) -> Option<api::Reply> {
    if method != Method::GET {
        return None;
    }
    if path == "/api/sync/artifacts" {
        logs::info(&mut store.lock().unwrap(), "开始同步所有远程配置...");
        let result = all(store, false).await;
        if let Err(error) = &result {
            logs::error(
                &mut store.lock().unwrap(),
                &format!("同步配置失败，原因：{}", error.message),
            );
        }
        return Some(match result {
            Ok(()) => api::success(None, 200),
            Err(error) if error.code == "STATE_WRITE_FAILED" => error.reply(500),
            Err(error) => Error::internal(
                "FAILED_TO_SYNC_ARTIFACTS",
                "Failed to sync all artifacts",
                error.message,
            )
            .reply(500),
        });
    }
    let name = path.strip_prefix("/api/sync/artifact/")?;
    let result = one(store, name).await;
    if let Err(error) = &result {
        logs::error(
            &mut store.lock().unwrap(),
            &format!("远程配置 {name} 发生错误: {}", error.message),
        );
    }
    Some(match result {
        Ok(data) => api::success(Some(data), 200),
        Err(error) if error.kind == "ResourceNotFoundError" => error.reply(404),
        Err(error) if error.code == "STATE_WRITE_FAILED" => error.reply(500),
        Err(error) => Error::internal(
            "FAILED_TO_SYNC_ARTIFACT",
            &format!("Failed to sync artifact {name}"),
            error.message,
        )
        .reply(500),
    })
}
