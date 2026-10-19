//! Existing subscription, collection and generated-file pipeline over live JS values.
use crate::{js_language::Js, script, script_download, script_network as network};
use rquickjs::{
    Ctx, Function, Object, Promise, Value,
    prelude::{Async, Func},
};

pub(crate) fn object<'js>(js: &Js<'js>) -> rquickjs::Result<Value<'js>> {
    Ok(Object::new(js.ctx.clone())?.into_value())
}

pub(crate) fn optional<'js>(
    js: &Js<'js>,
    value: &Value<'js>,
    key: &str,
) -> rquickjs::Result<Value<'js>> {
    js.call("optional", (value.clone(), key))
}

fn nullish(value: &Value<'_>) -> bool {
    value.is_null() || value.is_undefined()
}

fn array<'js>(js: &Js<'js>, value: Value<'js>) -> rquickjs::Result<Value<'js>> {
    if value.is_array() {
        Ok(value)
    } else {
        js.array([value])
    }
}

fn merge_mode<'js>(js: &Js<'js>, value: Value<'js>) -> rquickjs::Result<bool> {
    Ok(js.equal(&value, &js.value("localFirst")?)?
        || js.equal(&value, &js.value("remoteFirst")?)?)
}

fn mode<'js>(
    js: &Js<'js>,
    values: impl IntoIterator<Item = Value<'js>>,
) -> rquickjs::Result<String> {
    for value in values {
        if nullish(&value) || js.equal(&value, &js.value("")?)? {
            continue;
        }
        if js.equal(&value, &js.value(true)?)? {
            return Ok("quiet".to_owned());
        }
        if js.equal(&value, &js.value(false)?)? {
            return Ok("disabled".to_owned());
        }
        return js.text(value);
    }
    Ok("disabled".to_owned())
}

fn fallback(mode: &str) -> bool {
    matches!(mode, "fallbackNotify" | "fallbackQuiet")
}

pub(crate) fn failure_text(
    js: &Js<'_>,
    error: rquickjs::Error,
) -> rquickjs::Result<(String, String)> {
    if !error.is_exception() {
        let text = error.to_string();
        return Ok((text.clone(), text));
    }
    let thrown = js.ctx.catch();
    let full = js.text(thrown.clone())?;
    let reason = script::message(&js.ctx, js.ctx.throw(thrown));
    Ok((reason, full))
}

fn notify(js: &Js<'_>, title: &str, name: &str, message: &str) -> rquickjs::Result<()> {
    js.method(
        network::host(js)?,
        "notify",
        vec![
            js.value(title)?,
            js.value(format!("❌ {name}"))?,
            js.value(message)?,
        ],
    )?;
    Ok(())
}

fn failed(js: &Js<'_>, mode: &str, message: &str, title: &str, name: &str) -> rquickjs::Result<()> {
    if mode == "disabled" || fallback(mode) {
        return Err(js.error(message));
    }
    if mode == "enabled" {
        notify(js, title, name, message)?;
    }
    Ok(())
}

fn warn_duplicate<'js>(
    js: &Js<'js>,
    proxies: &Value<'js>,
    options: &Value<'js>,
    kind: &str,
) -> rquickjs::Result<()> {
    let seen = object(js)?;
    let proxies: Vec<Value> = js.call("iterable", (proxies.clone(),))?;
    for proxy in proxies {
        let name = js.text(js.get(&proxy, "name")?)?;
        if js.truthy(&js.get(&seen, &name)?)? {
            let resource = js.text(optional(js, options, "name")?)?;
            let media = object(js)?;
            js.set(&media, "media-url", js.value("https://cdn3.iconfinder.com/data/icons/seo-outline-1/512/25_code_program_programming_develop_bug_search_developer-512.png")?)?;
            js.method(
                network::host(js)?,
                "notify",
                vec![
                    js.value("🌍 Sub-Store")?,
                    js.value(format!("⚠️ {kind} {resource} 包含重复节点 {name}！"))?,
                    js.value("请仔细检测配置！")?,
                    media,
                ],
            )?;
            break;
        }
        js.set(&seen, &name, js.value(true)?)?;
    }
    Ok(())
}

pub(crate) fn read<'js>(js: &Js<'js>, key: &str) -> rquickjs::Result<Value<'js>> {
    js.method(network::host(js)?, "read", vec![js.value(key)?])
}

pub(crate) fn find<'js>(
    js: &Js<'js>,
    items: Value<'js>,
    name: Value<'js>,
) -> rquickjs::Result<Value<'js>> {
    let scope = js.clone();
    let predicate = Function::new(js.ctx.clone(), move |item: Value<'js>| {
        scope.equal(&scope.get(&item, "name")?, &name)
    })?;
    js.method(items, "find", vec![predicate.into_value()])
}

pub(crate) async fn utility<'js>(
    js: &Js<'js>,
    name: &str,
    args: Vec<Value<'js>>,
) -> rquickjs::Result<Value<'js>> {
    let utils: Value = js.ctx.globals().get("ProxyUtils")?;
    script::settled(js, js.method(utils, name, args)?).await
}

async fn all<'js>(js: &Js<'js>, promises: Vec<Value<'js>>) -> rquickjs::Result<Value<'js>> {
    let constructor: Value = js.ctx.globals().get("Promise")?;
    script::settled(
        js,
        js.method(constructor, "all", vec![js.array(promises)?])?,
    )
    .await
}

async fn sources<'js>(
    js: &Js<'js>,
    sub: Value<'js>,
    options: Value<'js>,
    preview: bool,
    saved_raw: &mut Value<'js>,
) -> rquickjs::Result<(Value<'js>, Value<'js>)> {
    let content = optional(js, &options, "content")?;
    let override_merge = optional(js, &options, "mergeSources")?;
    let url = optional(js, &options, "url")?;
    if !preview && js.truthy(&content)? && !merge_mode(js, override_merge.clone())? {
        return Ok((content.clone(), content));
    }
    let override_url = !preview && js.truthy(&url)?;
    let merge = if override_url {
        override_merge
    } else {
        js.get(&sub, "mergeSources")?
    };
    if !override_url
        && js.equal(&js.get(&sub, "source")?, &js.value("local")?)?
        && !merge_mode(js, merge.clone())?
    {
        let content = js.get(&sub, "content")?;
        return Ok((content.clone(), content));
    }
    let url = if override_url {
        url
    } else {
        js.get(&sub, "url")?
    };
    let lines = js.method(url, "split", vec![js.call("regex", (r"[\r\n]+", ""))?])?;
    let trim = js.clone();
    let lines = js.map(lines, move |value| trim.method(value, "trim", vec![]))?;
    let trim = js.clone();
    let lines = js.filter(lines, move |value, _| {
        trim.truthy(&trim.get(&value, "length")?)
    })?;
    let lines: Vec<Value> = js.call("iterable", (lines,))?;
    let errors = object(js)?;
    let ua = network::prefer(js, [optional(js, &options, "ua")?, js.get(&sub, "ua")?])?;
    let proxy = network::prefer(
        js,
        [optional(js, &options, "proxy")?, js.get(&sub, "proxy")?],
    )?;
    let no_cache = network::prefer(
        js,
        [optional(js, &options, "noCache")?, js.get(&sub, "noCache")?],
    )?;
    let await_cache = optional(js, &options, "awaitCustomCache")?;
    let no_flow = if preview {
        js.undefined()
    } else {
        network::prefer(
            js,
            [optional(js, &options, "noFlow")?, js.get(&sub, "noFlow")?],
        )?
    };
    let download_options = object(js)?;
    js.set(&download_options, "returnRaw", js.value(true)?)?;
    if !preview {
        js.set(&download_options, "noFlow", no_flow)?;
    }
    let mut pending = Vec::new();
    for url in lines {
        let worker = js.clone();
        let errors = errors.clone();
        let sub = sub.clone();
        let args = vec![
            url.clone(),
            ua.clone(),
            js.undefined(),
            proxy.clone(),
            js.undefined(),
            await_cache.clone(),
            no_cache.clone(),
            js.value(true)?,
            download_options.clone(),
        ];
        let promise = Promise::wrap_future(&js.ctx, async move {
            match utility(&worker, "download", args).await {
                Ok(value) => Ok(value),
                Err(error) => {
                    let (_, full) = failure_text(&worker, error)?;
                    script::log(
                        &worker,
                        "error",
                        &format!(
                            "订阅 {} 的远程订阅 {} 发生错误: {full}",
                            worker.text(worker.get(&sub, "name")?)?,
                            script_download::mask_url(&worker, url.clone())?,
                        ),
                    );
                    worker.set(&errors, &worker.text(url)?, worker.value(true)?)?;
                    worker.value("")
                }
            }
        })?;
        pending.push(promise.into_value());
    }
    let downloaded = all(js, pending).await?;
    let raw_scope = js.clone();
    *saved_raw = js.map(downloaded.clone(), move |value| {
        let raw = raw_scope.get(&value, "raw")?;
        Ok(if nullish(&raw) { value } else { raw })
    })?;
    let count: usize = js.call(
        "length",
        (js.call::<Value, _>("entries", (errors.clone(),))?,),
    )?;
    if count > 0 {
        let entries: Vec<Value> =
            js.call("iterable", (js.call::<Value, _>("entries", (errors,))?,))?;
        let mut urls = Vec::new();
        for entry in entries {
            urls.push(script_download::mask_url(js, js.get(&entry, "0")?)?);
        }
        let resolved = mode(
            js,
            [
                optional(js, &options, "ignoreFailedRemoteSub")?,
                js.get(&sub, "ignoreFailedRemoteSub")?,
            ],
        )?;
        failed(
            js,
            &resolved,
            &format!(
                "订阅 {} 的远程订阅 {} 发生错误, 请查看日志",
                js.text(js.get(&sub, "name")?)?,
                urls.join(", ")
            ),
            if preview {
                "🌍 Sub-Store 预览订阅失败"
            } else {
                "🌍 Sub-Store 处理订阅失败"
            },
            &js.text(js.get(&sub, "name")?)?,
        )?;
    }
    let result_scope = js.clone();
    let raw = js.map(downloaded.clone(), move |value| {
        let result = result_scope.get(&value, "result")?;
        Ok(if nullish(&result) { value } else { result })
    })?;
    let raw_scope = js.clone();
    let source_raw = js.map(downloaded, move |value| {
        let raw = raw_scope.get(&value, "raw")?;
        Ok(if nullish(&raw) { value } else { raw })
    })?;
    let content = if override_url {
        content
    } else {
        js.get(&sub, "content")?
    };
    for value in [&raw, &source_raw] {
        if js.equal(&merge, &js.value("localFirst")?)? {
            js.method(value.clone(), "unshift", vec![content.clone()])?;
        } else if js.equal(&merge, &js.value("remoteFirst")?)? {
            js.method(value.clone(), "push", vec![content.clone()])?;
        }
    }
    Ok((raw, source_raw))
}

fn parse<'js>(js: &Js<'js>, raw: Value<'js>) -> rquickjs::Result<Value<'js>> {
    let scope = js.clone();
    let parsed = js.map(array(js, raw)?, move |value| {
        let utils: Value = scope.ctx.globals().get("ProxyUtils")?;
        scope.method(utils, "parse", vec![value])
    })?;
    js.method(parsed, "flat", vec![])
}

fn tag<'js>(
    js: &Js<'js>,
    values: Value<'js>,
    sub: Option<Value<'js>>,
    collection: Option<Value<'js>>,
    preview: bool,
) -> rquickjs::Result<()> {
    let items: Vec<Value> = js.call("iterable", (values,))?;
    for (index, item) in items.into_iter().enumerate() {
        if preview {
            js.set(&item, "id", js.value(index)?)?;
        }
        if let Some(sub) = &sub {
            js.set(&item, "_subName", js.get(sub, "name")?)?;
            js.set(&item, "_subDisplayName", js.get(sub, "displayName")?)?;
        }
        if let Some(collection) = &collection {
            js.set(&item, "_collectionName", js.get(collection, "name")?)?;
            js.set(
                &item,
                "_collectionDisplayName",
                js.get(collection, "displayName")?,
            )?;
        }
    }
    Ok(())
}

fn packaged<'js>(
    js: &Js<'js>,
    result: Value<'js>,
    raw: Value<'js>,
    options: &Value<'js>,
) -> rquickjs::Result<Value<'js>> {
    if !js.truthy(&optional(js, options, "includeRaw")?)? {
        return Ok(result);
    }
    let value = object(js)?;
    js.set(&value, "result", result)?;
    js.set(&value, "raw", raw)?;
    Ok(value)
}

fn preview_result<'js>(
    js: &Js<'js>,
    original: Value<'js>,
    processed: Value<'js>,
) -> rquickjs::Result<Value<'js>> {
    let result = object(js)?;
    js.set(&result, "original", original)?;
    js.set(&result, "processed", processed)?;
    Ok(result)
}

async fn empty<'js>(js: &Js<'js>, options: &Value<'js>) -> rquickjs::Result<Value<'js>> {
    if js.equal(&optional(js, options, "produceType")?, &js.value("raw")?)? {
        return js.value("[]");
    }
    utility(
        js,
        "produce",
        vec![
            js.array([])?,
            network::prefer(js, [optional(js, options, "platform")?, js.value("JSON")?])?,
            optional(js, options, "produceType")?,
            optional(js, options, "produceOpts")?,
        ],
    )
    .await
}

pub(crate) fn mihomo<'js>(js: &Js<'js>, file: &Value<'js>) -> rquickjs::Result<bool> {
    let kind = optional(js, file, "type")?;
    Ok(js.equal(&kind, &js.value("mihomoConfig")?)?
        || js.equal(&kind, &js.value("mihomoProfile")?)?)
}

fn yaml<'js>(js: &Js<'js>, method: &str, value: Value<'js>) -> rquickjs::Result<Value<'js>> {
    let utils: Value = js.ctx.globals().get("ProxyUtils")?;
    js.method(js.get(&utils, "yaml")?, method, vec![value])
}

pub(crate) fn normalize_yaml<'js>(js: &Js<'js>, raw: Value<'js>) -> rquickjs::Result<Value<'js>> {
    if !raw.is_string() {
        return Ok(raw);
    }
    let text = js.text(raw.clone())?;
    if !text.contains("proxies:") || !text.contains("short-id:") {
        return Ok(raw);
    }
    let valid = (|| {
        let parsed = yaml(js, "safeLoad", raw.clone())?;
        let proxies = js.get(&parsed, "proxies")?;
        Ok::<_, rquickjs::Error>(proxies.is_array() && js.truthy(&js.get(&proxies, "length")?)?)
    })();
    match valid {
        Ok(true) => js.value(crate::clash::short_ids(&text)),
        Ok(false) => Ok(raw),
        Err(_) => {
            js.ctx.catch();
            Ok(raw)
        }
    }
}

fn plain<'js>(js: &Js<'js>, value: Value<'js>) -> rquickjs::Result<bool> {
    let lodash: Value = js.ctx.globals().get("lodash")?;
    js.truthy(&js.method(lodash, "isPlainObject", vec![value])?)
}

pub(crate) fn merge<'js>(
    js: &Js<'js>,
    target: Value<'js>,
    other: Value<'js>,
) -> rquickjs::Result<Value<'js>> {
    let other = if other.is_string() {
        js.call("jsonParse", (other,))?
    } else {
        other
    };
    let keys: Vec<String> = js.call("keysIn", (other.clone(),))?;
    let trim = |key: &str| {
        key.strip_prefix('<')
            .and_then(|key| key.strip_suffix('>'))
            .unwrap_or(key)
            .to_owned()
    };
    for key in keys {
        let value = js.get(&other, &key)?;
        if plain(js, value.clone())? {
            if let Some(key) = key.strip_suffix('!') {
                js.set(&target, &trim(key), value)?;
            } else {
                let key = trim(&key);
                let old = network::prefer(js, [js.get(&target, &key)?, object(js)?])?;
                js.set(&target, &key, old.clone())?;
                merge(js, old, value)?;
            }
        } else if value.is_array() {
            let front = key.starts_with('+');
            let back = key.ends_with('+');
            let actual = trim(if front {
                &key[1..]
            } else if back {
                &key[..key.len() - 1]
            } else {
                &key
            });
            let value = if front || back {
                let old = network::prefer(js, [js.get(&target, &actual)?, js.array([])?])?;
                let a: Vec<Value> = js.call(
                    "iterable",
                    (if front { value.clone() } else { old.clone() },),
                )?;
                let b: Vec<Value> = js.call("iterable", (if front { old } else { value },))?;
                js.array(a.into_iter().chain(b))?
            } else {
                value
            };
            js.set(&target, &actual, value)?;
        } else {
            js.set(&target, &key, value)?;
        }
    }
    Ok(target)
}

pub(crate) fn patch<'js>(
    js: &Js<'js>,
    input: &Value<'js>,
    script: Value<'js>,
) -> rquickjs::Result<Value<'js>> {
    let patch = yaml(js, "safeLoad", script)?;
    if js.call::<String, _>("typeOf", (patch.clone(),))? != "object" {
        return Err(js.error("patch is not an object"));
    }
    let content = optional(js, input, "$content")?;
    let mut config = object(js)?;
    if js.truthy(&content)? {
        match yaml(js, "safeLoad", content) {
            Ok(value) => config = network::prefer(js, [value, object(js)?])?,
            Err(error) => script::log(js, "error", &script::message(&js.ctx, error)),
        }
    }
    let merged = merge(js, config, patch)?;
    js.set(input, "$content", yaml(js, "safeDump", merged)?)?;
    Ok(input.clone())
}

async fn file_raw<'js>(
    js: &Js<'js>,
    file: Value<'js>,
    options: Value<'js>,
) -> rquickjs::Result<Value<'js>> {
    let content = optional(js, &options, "content")?;
    let override_merge = optional(js, &options, "mergeSources")?;
    if js.truthy(&content)? && !merge_mode(js, override_merge.clone())? {
        return Ok(content);
    }
    let url = optional(js, &options, "url")?;
    let override_url = js.truthy(&url)?;
    let merge = if override_url {
        override_merge
    } else {
        js.get(&file, "mergeSources")?
    };
    let source = if mihomo(js, &file)? {
        let source = js.get(&file, "sourceType")?;
        if matches!(
            js.text(source.clone())?.as_str(),
            "collection" | "local" | "none" | "remote" | "subscription"
        ) {
            source
        } else {
            js.value("collection")?
        }
    } else {
        js.get(&file, "source")?
    };
    if !override_url && js.equal(&source, &js.value("local")?)? && !merge_mode(js, merge.clone())? {
        return js.get(&file, "content");
    }
    let raw_url = network::prefer(
        js,
        [
            if override_url {
                url
            } else {
                js.get(&file, "url")?
            },
            js.value("")?,
        ],
    )?;
    let lines = js.method(
        js.value(js.text(raw_url)?)?,
        "split",
        vec![js.call("regex", (r"[\r\n]+", ""))?],
    )?;
    let scope = js.clone();
    let lines = js.map(lines, move |value| scope.method(value, "trim", vec![]))?;
    let scope = js.clone();
    let lines = js.filter(lines, move |value, _| scope.truthy(&value))?;
    if !js.truthy(&js.get(&lines, "length")?)? {
        return Err(js.error(&format!(
            "文件 {} 未配置远程文件 URL",
            js.text(js.get(&file, "name")?)?
        )));
    }
    let errors = object(js)?;
    let mut pending = Vec::new();
    let lines: Vec<Value> = js.call("iterable", (lines,))?;
    for url in lines {
        let worker = js.clone();
        let errors = errors.clone();
        let file = file.clone();
        let download_options = object(js)?;
        js.set(
            &download_options,
            "noFlow",
            optional(js, &options, "noFlow")?,
        )?;
        let args = vec![
            url.clone(),
            network::prefer(js, [optional(js, &options, "ua")?, js.get(&file, "ua")?])?,
            js.undefined(),
            network::prefer(
                js,
                [js.get(&file, "proxy")?, optional(js, &options, "proxy")?],
            )?,
            js.undefined(),
            js.undefined(),
            network::prefer(
                js,
                [
                    optional(js, &options, "noCache")?,
                    js.get(&file, "noCache")?,
                ],
            )?,
            js.undefined(),
            download_options,
        ];
        pending.push(
            Promise::wrap_future(&js.ctx, async move {
                match utility(&worker, "download", args).await {
                    Ok(value) => Ok(value),
                    Err(error) => {
                        let (_, full) = failure_text(&worker, error)?;
                        script::log(
                            &worker,
                            "error",
                            &format!(
                                "文件 {} 的远程文件 {} 发生错误: {full}",
                                worker.text(worker.get(&file, "name")?)?,
                                script_download::mask_url(&worker, url.clone())?,
                            ),
                        );
                        worker.set(&errors, &worker.text(url)?, worker.value(true)?)?;
                        worker.value("")
                    }
                }
            })?
            .into_value(),
        );
    }
    let raw = all(js, pending).await?;
    let entries: Vec<Value> = js.call("iterable", (js.call::<Value, _>("entries", (errors,))?,))?;
    if !entries.is_empty() {
        let override_ignore = optional(js, &options, "ignoreFailedRemoteFile")?;
        let ignore = if nullish(&override_ignore) || js.equal(&override_ignore, &js.value("")?)? {
            js.get(&file, "ignoreFailedRemoteFile")?
        } else {
            override_ignore
        };
        let mut urls = Vec::new();
        for entry in entries {
            urls.push(script_download::mask_url(js, js.get(&entry, "0")?)?);
        }
        let message = format!(
            "文件 {} 的远程文件 {} 发生错误, 请查看日志",
            js.text(js.get(&file, "name")?)?,
            urls.join(", ")
        );
        if !js.truthy(&ignore)? {
            return Err(js.error(&message));
        }
        if js.equal(&ignore, &js.value("enabled")?)? {
            notify(
                js,
                &js.text(network::prefer(
                    js,
                    [
                        optional(js, &options, "notifyTitle")?,
                        js.value("🌍 Sub-Store 处理文件失败")?,
                    ],
                )?)?,
                &js.text(js.get(&file, "name")?)?,
                &format!("远程文件 {} 发生错误, 请查看日志", urls.join(", ")),
            )?;
        }
    }
    let content = if override_url {
        content
    } else {
        js.get(&file, "content")?
    };
    if js.equal(&merge, &js.value("localFirst")?)? {
        js.method(raw.clone(), "unshift", vec![content])?;
    } else if js.equal(&merge, &js.value("remoteFirst")?)? {
        js.method(raw.clone(), "push", vec![content])?;
    }
    Ok(raw)
}

fn raw_files<'js>(js: &Js<'js>, raw: Value<'js>) -> rquickjs::Result<Value<'js>> {
    js.method(array(js, raw)?, "flat", vec![])
}

fn join_files<'js>(js: &Js<'js>, raw: Value<'js>) -> rquickjs::Result<Value<'js>> {
    let scope = js.clone();
    let files = js.filter(raw_files(js, raw)?, move |value, _| {
        Ok(!nullish(&value) && !scope.equal(&value, &scope.value("")?)?)
    })?;
    js.method(files, "join", vec![js.value("\n")?])
}

pub(crate) async fn file<'js>(
    js: &Js<'js>,
    file: Value<'js>,
    options: Value<'js>,
    preview: bool,
) -> rquickjs::Result<Value<'js>> {
    if preview {
        js.set(
            &options,
            "notifyTitle",
            js.value("🌍 Sub-Store 预览文件失败")?,
        )?;
    }
    let raw = if mihomo(js, &file)? {
        let source = js.text(js.get(&file, "sourceType")?)?;
        let source = if matches!(
            source.as_str(),
            "collection" | "local" | "none" | "remote" | "subscription"
        ) {
            source.as_str()
        } else {
            "collection"
        };
        let config = object(js)?;
        if source == "none" {
            yaml(js, "safeDump", config)?
        } else if matches!(source, "local" | "remote") {
            let raw = file_raw(js, file.clone(), options.clone()).await?;
            if !js.equal(&js.get(&file, "mode")?, &js.value("proxy")?)? {
                join_files(js, raw)?
            } else {
                let proxies = parse(js, raw_files(js, raw)?)?;
                if !js.truthy(&js.get(&proxies, "length")?)? {
                    return Err(js.error(&format!(
                        "文件 {} 中不含有效节点",
                        js.text(js.get(&file, "name")?)?
                    )));
                }
                let produce_options = object(js)?;
                js.set(
                    &produce_options,
                    "delete-underscore-fields",
                    js.value(true)?,
                )?;
                js.set(
                    &produce_options,
                    "include-unsupported-proxy",
                    js.get(&file, "includeUnsupportedProxy")?,
                )?;
                js.set(
                    &config,
                    "proxies",
                    utility(
                        js,
                        "produce",
                        vec![
                            proxies,
                            js.value("mihomo")?,
                            js.value("internal")?,
                            produce_options,
                        ],
                    )
                    .await?,
                )?;
                yaml(js, "safeDump", config)?
            }
        } else {
            let args = object(js)?;
            for (name, value) in [
                ("type", js.value(source)?),
                ("name", js.get(&file, "sourceName")?),
                ("platform", js.value("mihomo")?),
                ("produceType", js.value("internal")?),
                ("noFlow", optional(js, &options, "noFlow")?),
            ] {
                js.set(&args, name, value)?;
            }
            let produce_options = object(js)?;
            js.set(
                &produce_options,
                "delete-underscore-fields",
                js.value(true)?,
            )?;
            js.set(
                &produce_options,
                "include-unsupported-proxy",
                js.get(&file, "includeUnsupportedProxy")?,
            )?;
            js.set(&args, "produceOpts", produce_options)?;
            let generate: Function = js.ctx.globals().get("produceArtifact")?;
            let proxies = script::settled(js, generate.call((args,))?).await?;
            js.set(&config, "proxies", proxies)?;
            yaml(js, "safeDump", config)?
        }
    } else {
        file_raw(js, file.clone(), options.clone()).await?
    };
    let files = raw_files(js, raw)?;
    if js.equal(&optional(js, &options, "produceType")?, &js.value("raw")?)? {
        return Ok(js
            .ctx
            .json_stringify(files)?
            .ok_or_else(|| js.error("Cannot stringify raw files"))?
            .into_value());
    }
    let input = object(js)?;
    js.set(&input, "$files", files.clone())?;
    let original = join_files(js, files)?;
    js.set(&input, "$content", original.clone())?;
    if !preview {
        js.set(&input, "$options", optional(js, &options, "$options")?)?;
    }
    let items = js.get(&file, "process")?;
    let processed = if items.is_array() && js.truthy(&js.get(&items, "length")?)? {
        js.set(&input, "$file", file)?;
        let execution = object(js)?;
        js.set(&execution, "noFlow", optional(js, &options, "noFlow")?)?;
        utility(
            js,
            "process",
            vec![
                input,
                items,
                js.undefined(),
                js.undefined(),
                js.undefined(),
                js.undefined(),
                execution,
            ],
        )
        .await?
    } else {
        let plain = object(js)?;
        js.set(&plain, "$content", js.get(&input, "$content")?)?;
        js.set(&plain, "$files", js.get(&input, "$files")?)?;
        if !preview {
            js.set(&plain, "$options", optional(js, &options, "$options")?)?;
        }
        plain
    };
    let content = optional(js, &processed, "$content")?;
    js.set(
        &processed,
        "$content",
        normalize_yaml(
            js,
            if nullish(&content) {
                js.value("")?
            } else {
                content
            },
        )?,
    )?;
    if preview {
        let output = object(js)?;
        js.set(&output, "original", original)?;
        js.set(&output, "processed", js.get(&processed, "$content")?)?;
        return Ok(output);
    }
    if js.truthy(&optional(js, &options, "all")?)? {
        Ok(processed)
    } else {
        optional(js, &processed, "$content")
    }
}

async fn file_main<'js>(
    ctx: Ctx<'js>,
    main: Function<'js>,
    content: Value<'js>,
) -> rquickjs::Result<Value<'js>> {
    let js = Js::new(ctx);
    let config = if js.truthy(&content)? {
        match yaml(&js, "safeLoad", content) {
            Ok(value) => network::prefer(&js, [value, object(&js)?])?,
            Err(error) => {
                script::log(&js, "error", &script::message(&js.ctx, error));
                object(&js)?
            }
        }
    } else {
        object(&js)?
    };
    let value = script::settled(&js, main.call((config,))?).await?;
    yaml(&js, "safeDump", value)
}

async fn subscription<'js>(
    js: &Js<'js>,
    sub: Value<'js>,
    options: Value<'js>,
    preview: bool,
) -> rquickjs::Result<Value<'js>> {
    let resolved = mode(
        js,
        [
            optional(js, &options, "ignoreFailedRemoteSub")?,
            js.get(&sub, "ignoreFailedRemoteSub")?,
        ],
    )?;
    let mut source_raw = js.array([])?;
    let result = async {
        let (raw, source) =
            sources(js, sub.clone(), options.clone(), preview, &mut source_raw).await?;
        source_raw = source;
        if !preview && js.equal(&optional(js, &options, "produceType")?, &js.value("raw")?)? {
            let result = js
                .ctx
                .json_stringify(js.method(array(js, raw)?, "flat", vec![])?)?
                .ok_or_else(|| js.error("Cannot stringify raw sources"))?
                .into_value();
            return packaged(js, result, array(js, source_raw.clone())?, &options);
        }
        let original = parse(js, raw)?;
        tag(js, original.clone(), Some(sub.clone()), None, preview)?;
        let source = object(js)?;
        js.set(&source, &js.text(js.get(&sub, "name")?)?, sub.clone())?;
        let target = network::prefer(js, [optional(js, &options, "platform")?, js.value("JSON")?])?;
        let execution = object(js)?;
        if !preview {
            js.set(
                &execution,
                "noFlow",
                network::prefer(
                    js,
                    [optional(js, &options, "noFlow")?, js.get(&sub, "noFlow")?],
                )?,
            )?;
        }
        let processed = utility(
            js,
            "process",
            vec![
                original.clone(),
                network::prefer(js, [js.get(&sub, "process")?, js.array([])?])?,
                target.clone(),
                source,
                optional(js, &options, "$options")?,
                source_raw.clone(),
                execution,
            ],
        )
        .await?;
        if preview {
            return preview_result(js, original, processed);
        }
        if js.equal(&js.get(&processed, "length")?, &js.value(0)?)? {
            return Err(js.error(&format!(
                "订阅 {} 中不含有效节点",
                js.text(optional(js, &options, "name")?)?
            )));
        }
        warn_duplicate(js, &processed, &options, "订阅")?;
        let result = utility(
            js,
            "produce",
            vec![
                processed,
                target,
                optional(js, &options, "produceType")?,
                optional(js, &options, "produceOpts")?,
            ],
        )
        .await?;
        packaged(js, result, array(js, source_raw.clone())?, &options)
    }
    .await;
    match result {
        Ok(value) => Ok(value),
        Err(error) if fallback(&resolved) => {
            let reason = script::message(&js.ctx, error);
            let name = js.text(js.get(&sub, "name")?)?;
            if resolved == "fallbackNotify" {
                notify(
                    js,
                    if preview {
                        "🌍 Sub-Store 预览订阅失败"
                    } else {
                        "🌍 Sub-Store 处理订阅失败"
                    },
                    &name,
                    &format!("🤔 原因：{reason}"),
                )?;
            }
            script::log(
                js,
                "error",
                &format!(
                    "订阅 {name} {}启用兜底后返回空结果: {reason}",
                    if preview { "预览" } else { "" }
                ),
            );
            if preview {
                preview_result(js, js.array([])?, js.array([])?)
            } else {
                packaged(
                    js,
                    empty(js, &options).await?,
                    array(js, source_raw)?,
                    &options,
                )
            }
        }
        Err(error) => Err(error),
    }
}

async fn collection<'js>(
    js: &Js<'js>,
    collection: Value<'js>,
    options: Value<'js>,
    preview: bool,
) -> rquickjs::Result<Value<'js>> {
    let resolved = mode(
        js,
        [
            optional(js, &options, "ignoreFailedRemoteSub")?,
            js.get(&collection, "ignoreFailedRemoteSub")?,
        ],
    )?;
    let raw_results = object(js)?;
    let result = async {
        let all_subs = read(js, "subs")?;
        let mut names: Vec<Value> =
            js.call("iterable", (js.get(&collection, "subscriptions")?,))?;
        let tags = js.get(&collection, "subscriptionTags")?;
        if tags.is_array() && js.truthy(&js.get(&tags, "length")?)? {
            let subs: Vec<Value> = js.call("iterable", (all_subs.clone(),))?;
            for sub in subs {
                let sub_tags = js.get(&sub, "tag")?;
                if !sub_tags.is_array() {
                    continue;
                }
                let name = js.get(&sub, "name")?;
                let mut exists = false;
                for prior in &names {
                    exists |= js.equal(prior, &name)?;
                }
                if exists {
                    continue;
                }
                let values: Vec<Value> = js.call("iterable", (sub_tags,))?;
                for tag in values {
                    if js.truthy(&js.method(tags.clone(), "includes", vec![tag])?)? {
                        names.push(name.clone());
                        break;
                    }
                }
            }
        }
        let results = object(js)?;
        let errors = object(js)?;
        let target = if preview {
            js.value("JSON")?
        } else {
            network::prefer(js, [optional(js, &options, "platform")?, js.value("JSON")?])?
        };
        let mut pending = Vec::new();
        let processed = std::rc::Rc::new(std::cell::Cell::new(0usize));
        let total = names.len() as f64;
        for name in &names {
            let worker = js.clone();
            let name = name.clone();
            let all_subs = all_subs.clone();
            let collection = collection.clone();
            let options = options.clone();
            let results = results.clone();
            let raw_results = raw_results.clone();
            let errors = errors.clone();
            let target = target.clone();
            let processed = processed.clone();
            let promise = Promise::wrap_future(&js.ctx, async move {
                let sub = find(&worker, all_subs, name.clone())?;
                // The source reads sub.ignoreFailedRemoteSub before its per-child try.
                let child_mode = mode(&worker, [worker.get(&sub, "ignoreFailedRemoteSub")?])?;
                let key = worker.text(name)?;
                let percent = || -> rquickjs::Result<String> {
                    let fixed = worker.method(worker.value(processed.get() as f64 / total)?, "toFixed", vec![worker.value(1)?])?;
                    let number: f64 = worker.call("number", (fixed,))?;
                    worker.text(worker.value(100.0 * number)?)
                };
                let child = async {
                    let child_options = object(&worker)?;
                    if !preview {
                        let ua = if worker.truthy(&worker.get(&sub, "passThroughUA")?)? {
                            script::log(&worker, "info", &format!("订阅开启了透传 User-Agent, 使用请求的 User-Agent: {}", worker.text(optional(&worker, &options, "ua")?)?));
                            optional(&worker, &options, "ua")?
                        } else {
                            worker.get(&sub, "ua")?
                        };
                        worker.set(&child_options, "ua", ua)?;
                        worker.set(
                            &child_options,
                            "proxy",
                            network::prefer(
                                &worker,
                                [
                                    optional(&worker, &options, "proxy")?,
                                    worker.get(&sub, "proxy")?,
                                    worker.get(&collection, "proxy")?,
                                ],
                            )?,
                        )?;
                        worker.set(
                            &child_options,
                            "noCache",
                            optional(&worker, &options, "noCache")?,
                        )?;
                        worker.set(
                            &child_options,
                            "noFlow",
                            network::prefer(
                                &worker,
                                [
                                    optional(&worker, &options, "noFlow")?,
                                    worker.get(&collection, "noFlow")?,
                                ],
                            )?,
                        )?;
                    }
                    if !preview {
                        script::log(&worker, "info", &format!("正在处理子订阅：{}...", worker.text(worker.get(&sub, "name")?)?));
                    }
                    let mut saved_raw = worker.undefined();
                    let (raw, source_raw) = sources(
                        &worker,
                        sub.clone(),
                        child_options.clone(),
                        preview,
                        &mut saved_raw,
                    )
                    .await?;
                    let original = parse(&worker, raw)?;
                    tag(
                        &worker,
                        original.clone(),
                        Some(sub.clone()),
                        Some(collection.clone()),
                        false,
                    )?;
                    let source = object(&worker)?;
                    worker.set(&source, &key, sub.clone())?;
                    worker.set(&source, "_collection", collection.clone())?;
                    if !preview {
                        worker.set(
                            &source,
                            "$options",
                            optional(&worker, &options, "$options")?,
                        )?;
                    }
                    let execution = object(&worker)?;
                    if !preview {
                        worker.set(
                            &execution,
                            "noFlow",
                            network::prefer(
                                &worker,
                                [
                                    optional(&worker, &child_options, "noFlow")?,
                                    worker.get(&sub, "noFlow")?,
                                ],
                            )?,
                        )?;
                    }
                    let raw = array(&worker, source_raw)?;
                    let output = utility(
                        &worker,
                        "process",
                        vec![
                            original,
                            network::prefer(
                                &worker,
                                [worker.get(&sub, "process")?, worker.array([])?],
                            )?,
                            target,
                            source,
                            worker.undefined(),
                            raw.clone(),
                            execution,
                        ],
                    )
                    .await?;
                    worker.set(&results, &key, output)?;
                    worker.set(&raw_results, &key, raw)?;
                    if !preview {
                        processed.set(processed.get()+1);
                        script::log(&worker, "info", &format!("✅ 子订阅：{}加载成功，进度--{}% ", worker.text(worker.get(&sub, "name")?)?, percent()?));
                    }
                    Ok::<_, rquickjs::Error>(())
                }
                .await;
                if let Err(error) = child {
                    let (reason, full) = failure_text(&worker, error)?;
                    let sub_name = worker.text(worker.get(&sub, "name")?)?;
                    if fallback(&child_mode) {
                        if child_mode == "fallbackNotify" {
                            notify(
                                &worker,
                                if preview {
                                    "🌍 Sub-Store 预览订阅失败"
                                } else {
                                    "🌍 Sub-Store 处理订阅失败"
                                },
                                &sub_name,
                                &format!("🤔 原因：{reason}"),
                            )?;
                        }
                        script::log(&worker, "error", &format!("订阅 {sub_name} 在组合订阅{}中启用兜底后返回空结果: {reason}", if preview { "预览" } else { "处理" }));
                        worker.set(&results, &key, worker.array([])?)?;
                        worker.set(&raw_results, &key, worker.array([])?)?;
                    } else {
                        let message = if preview {
                            format!("❌ 处理组合订阅 {} 中的子订阅: {sub_name} 时出现错误：{full}！", worker.text(worker.get(&collection, "name")?)?)
                        } else {
                            format!("❌ 处理组合订阅中的子订阅: {sub_name}时出现错误：{full}！进度--{}%", percent()?)
                        };
                        script::log(&worker, "error", &message);
                        worker.set(&errors, &key, worker.value(true)?)?;
                        worker.set(&raw_results, &key, worker.undefined())?;
                    }
                }
                Ok::<_, rquickjs::Error>(worker.undefined())
            })?;
            pending.push(promise.into_value());
        }
        all(js, pending).await?;
        let errors: Vec<Value> =
            js.call("iterable", (js.call::<Value, _>("entries", (errors,))?,))?;
        if !errors.is_empty() {
            let count: usize = js.call(
                "length",
                (js.call::<Value, _>("entries", (results.clone(),))?,),
            )?;
            if count > 0 && fallback(&resolved) {
                for entry in errors {
                    js.set(&raw_results, &js.text(js.get(&entry, "0")?)?, js.array([])?)?;
                }
            } else {
                let mut failed_names = Vec::new();
                for entry in errors {
                    failed_names.push(js.text(js.get(&entry, "0")?)?);
                }
                failed(
                    js,
                    &resolved,
                    &format!(
                        "组合订阅 {} 的子订阅 {} 发生错误, 请查看日志",
                        js.text(js.get(&collection, "name")?)?,
                        failed_names.join(", ")
                    ),
                    if preview {
                        "🌍 Sub-Store 预览组合订阅失败"
                    } else {
                        "🌍 Sub-Store 处理组合订阅失败"
                    },
                    &js.text(js.get(&collection, "name")?)?,
                )?;
            }
        }
        let mut ordered = Vec::new();
        for name in names {
            ordered.push(network::prefer(
                js,
                [js.get(&results, &js.text(name)?)?, js.array([])?],
            )?);
        }
        let original = js.method(js.array(ordered)?, "flat", vec![])?;
        tag(
            js,
            original.clone(),
            None,
            Some(collection.clone()),
            preview,
        )?;
        let source = object(js)?;
        js.set(&source, "_collection", collection.clone())?;
        let execution = object(js)?;
        if !preview {
            js.set(
                &execution,
                "noFlow",
                network::prefer(
                    js,
                    [
                        optional(js, &options, "noFlow")?,
                        js.get(&collection, "noFlow")?,
                    ],
                )?,
            )?;
        }
        let processed = utility(
            js,
            "process",
            vec![
                original.clone(),
                network::prefer(js, [js.get(&collection, "process")?, js.array([])?])?,
                target.clone(),
                source,
                optional(js, &options, "$options")?,
                raw_results.clone(),
                execution,
            ],
        )
        .await?;
        if preview {
            return preview_result(js, original, processed);
        }
        if js.equal(&js.get(&processed, "length")?, &js.value(0)?)? {
            return Err(js.error(&format!(
                "组合订阅 {} 中不含有效节点",
                js.text(optional(js, &options, "name")?)?
            )));
        }
        warn_duplicate(js, &processed, &options, "组合订阅")?;
        let result = utility(
            js,
            "produce",
            vec![
                processed,
                target,
                optional(js, &options, "produceType")?,
                optional(js, &options, "produceOpts")?,
            ],
        )
        .await?;
        packaged(js, result, raw_results.clone(), &options)
    }
    .await;
    match result {
        Ok(value) => Ok(value),
        Err(error) if fallback(&resolved) => {
            let reason = script::message(&js.ctx, error);
            let name = js.text(js.get(&collection, "name")?)?;
            if resolved == "fallbackNotify" {
                notify(
                    js,
                    if preview {
                        "🌍 Sub-Store 预览组合订阅失败"
                    } else {
                        "🌍 Sub-Store 处理组合订阅失败"
                    },
                    &name,
                    &format!("🤔 原因：{reason}"),
                )?;
            }
            script::log(
                js,
                "error",
                &format!(
                    "组合订阅 {name} {}启用兜底后返回空结果: {reason}",
                    if preview { "预览" } else { "" }
                ),
            );
            if preview {
                preview_result(js, js.array([])?, js.array([])?)
            } else {
                packaged(js, empty(js, &options).await?, raw_results, &options)
            }
        }
        Err(error) => Err(error),
    }
}

pub(crate) async fn preview<'js>(
    js: &Js<'js>,
    kind: &str,
    item: Value<'js>,
    target: Value<'js>,
) -> rquickjs::Result<Value<'js>> {
    let options = object(js)?;
    js.set(&options, "platform", target)?;
    if kind == "collection" {
        collection(js, item, options, true).await
    } else {
        subscription(js, item, options, true).await
    }
}

pub(crate) async fn generate<'js>(
    js: &Js<'js>,
    kind: &str,
    item: Value<'js>,
    options: Value<'js>,
) -> rquickjs::Result<Value<'js>> {
    if kind == "collection" {
        collection(js, item, options, false).await
    } else {
        subscription(js, item, options, false).await
    }
}

pub(crate) async fn add<'js>(
    js: &Js<'js>,
    input: Value<'js>,
    args: Value<'js>,
    execution: Value<'js>,
) -> rquickjs::Result<Value<'js>> {
    if !mihomo(js, &optional(js, &input, "$file")?)? {
        return Ok(input);
    }
    let content = js.get(&input, "$content")?;
    let config = if js.truthy(&content)? {
        let value = yaml(js, "safeLoad", content)?;
        if plain(js, value.clone())? {
            value
        } else {
            object(js)?
        }
    } else {
        object(js)?
    };
    let existing = optional(js, &config, "proxies")?;
    let existing = if existing.is_array() {
        existing
    } else {
        js.array([])?
    };
    let options = object(js)?;
    let kind = js.get(&args, "sourceType")?;
    js.set(
        &options,
        "type",
        if kind.is_undefined() {
            js.value("subscription")?
        } else {
            kind
        },
    )?;
    js.set(&options, "name", js.get(&args, "sourceName")?)?;
    js.set(&options, "platform", js.value("mihomo")?)?;
    js.set(&options, "produceType", js.value("internal")?)?;
    js.set(&options, "noFlow", optional(js, &execution, "noFlow")?)?;
    let produce = object(js)?;
    js.set(&produce, "delete-underscore-fields", js.value(true)?)?;
    js.set(
        &produce,
        "include-unsupported-proxy",
        js.get(&args, "includeUnsupportedProxy")?,
    )?;
    js.set(&options, "produceOpts", produce)?;
    let generate: Function = js.ctx.globals().get("produceArtifact")?;
    let proxies = script::settled(js, generate.call((options,))?).await?;
    let position = js.text(js.get(&args, "position")?)?;
    let result = if matches!(position.as_str(), "front" | "back") {
        let a: Vec<Value> = js.call(
            "iterable",
            (if position == "front" {
                proxies.clone()
            } else {
                existing.clone()
            },),
        )?;
        let b: Vec<Value> = js.call(
            "iterable",
            (if position == "front" {
                existing
            } else {
                proxies
            },),
        )?;
        js.array(a.into_iter().chain(b))?
    } else {
        proxies
    };
    js.set(&config, "proxies", result)?;
    js.set(&input, "$content", yaml(js, "safeDump", config)?)?;
    Ok(input)
}

pub(crate) async fn artifact<'js>(
    ctx: Ctx<'js>,
    options: Value<'js>,
) -> rquickjs::Result<Value<'js>> {
    let js = Js::new(ctx);
    let kind = js.text(js.get(&options, "type")?)?;
    let name = js.get(&options, "name")?;
    match kind.as_str() {
        "sub" | "subscription" => {
            let sub = if js.truthy(&name)? {
                let sub = find(&js, read(&js, "subs")?, name.clone())?;
                if !js.truthy(&sub)? {
                    return Err(js.error(&format!("找不到订阅 {}", js.text(name)?)));
                }
                sub
            } else {
                let sub = js.get(&options, "subscription")?;
                if !js.truthy(&sub)? {
                    return Err(js.error("未提供订阅名称或订阅数据"));
                }
                sub
            };
            subscription(&js, sub, options, false).await
        }
        "col" | "collection" => {
            let col = find(&js, read(&js, "collections")?, name.clone())?;
            if !js.truthy(&col)? {
                return Err(js.error(&format!("找不到组合订阅 {}", js.text(name)?)));
            }
            collection(&js, col, options, false).await
        }
        "file" => {
            let file = js.get(&options, "file")?;
            let file = if js.truthy(&file)? {
                file
            } else {
                find(&js, read(&js, "files")?, name.clone())?
            };
            if !js.truthy(&file)? {
                return Err(js.error(&format!("找不到文件 {}", js.text(name)?)));
            }
            let file = if js.equal(&js.get(&file, "type")?, &js.value("mihomoProfile")?)? {
                let copy = js.call("spread", (object(&js)?, file))?;
                js.set(&copy, "type", js.value("mihomoConfig")?)?;
                copy
            } else {
                file
            };
            self::file(&js, file, options, false).await
        }
        "rule" => {
            let rule = find(&js, read(&js, "rules")?, name.clone())?;
            if !js.truthy(&rule)? {
                return Err(js.error(&format!("找不到规则 {}", js.text(name)?)));
            }
            let urls = js.get(&rule, "urls")?;
            let length: f64 = js.call("number", (js.get(&urls, "length")?,))?;
            let mut index = 0u64;
            while (index as f64) < length {
                let url = js.get(&urls, &index.to_string())?;
                if script_download::source(&js, url, object(&js)?)
                    .await
                    .is_err()
                {
                    js.ctx.catch();
                }
                index += 1;
            }
            // The frozen RuleUtils exports no process method; preserve its failure after downloads.
            Err(rquickjs::Exception::throw_type(
                &js.ctx,
                "RuleUtils.process is not a function",
            ))
        }
        _ => Ok(js.undefined()),
    }
}

pub(crate) fn initialize<'js>(ctx: &Ctx<'js>) -> rquickjs::Result<()> {
    ctx.globals()
        .set("__fileMain", Func::from(Async(file_main)))?;
    ctx.globals()
        .set("produceArtifact", Func::from(Async(artifact)))
}
