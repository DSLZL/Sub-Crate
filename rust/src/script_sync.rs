//! Native callers of the existing artifact pipeline and Gist response rules.
use crate::{
    js_language::Js,
    script,
    script_pipeline::{self as pipeline, object, optional},
};
use rquickjs::Value;

pub(crate) fn reason(js: &Js<'_>, error: rquickjs::Error) -> rquickjs::Error {
    if !error.is_exception() {
        return error;
    }
    let thrown = js.ctx.catch();
    match js.text(thrown).and_then(|text| js.value(text)) {
        Ok(value) => js.ctx.throw(value),
        Err(error) => error,
    }
}

pub(crate) async fn output<'js>(js: &Js<'js>, item: Value<'js>) -> rquickjs::Result<Value<'js>> {
    let options = object(js)?;
    for (target, source) in [
        ("type", "type"),
        ("name", "source"),
        ("platform", "platform"),
    ] {
        js.set(&options, target, js.get(&item, source)?)?;
    }
    let produce = object(js)?;
    js.set(
        &produce,
        "include-unsupported-proxy",
        js.get(&item, "includeUnsupportedProxy")?,
    )?;
    js.set(&produce, "prettyYaml", js.get(&item, "prettyYaml")?)?;
    js.set(
        &produce,
        "useMihomoExternal",
        js.value(js.equal(&js.get(&item, "platform")?, &js.value("SurgeMac")?)?)?,
    )?;
    js.set(&options, "produceOpts", produce)?;
    let mut body = pipeline::artifact(js.ctx.clone(), options).await?;
    let kind = js.text(js.get(&item, "type")?)?;
    let key = match kind.as_str() {
        "sub" | "subscription" => Some("subs"),
        "col" | "collection" => Some("collections"),
        "file" => Some("files"),
        _ => None,
    };
    let source = js.get(&item, "source")?;
    let source = if js.truthy(&source)?
        && let Some(key) = key
    {
        pipeline::find(js, pipeline::read(js, key)?, source)?
    } else {
        js.undefined()
    };
    for config in [item, source] {
        let key = optional(js, &config, "age-public-key")?;
        let key = if key.is_null() || key.is_undefined() {
            String::new()
        } else {
            js.text(key)?
        };
        let key = crate::json::trim(&key);
        if key.is_empty() {
            continue;
        }
        let utils: Value = js.ctx.globals().get("ProxyUtils")?;
        body = script::settled(
            js,
            js.method(
                js.get(&utils, "age")?,
                "encrypt",
                vec![body, js.value(key)?],
            )?,
        )
        .await?;
        break;
    }
    let result = object(js)?;
    js.set(&result, "content", body)?;
    Ok(result)
}

pub(crate) fn upload_result<'js>(js: &Js<'js>, input: Value<'js>) -> rquickjs::Result<Value<'js>> {
    let body: Value = js.call("jsonParse", (js.get(&input, "body")?,))?;
    for field in ["history", "forks", "owner"] {
        js.call::<(), _>("remove", (body.clone(), field))?;
    }
    let files = js.get(&body, "files")?;
    if js.truthy(&files)? {
        let constructor: Value = js.ctx.globals().get("Object")?;
        let entries: Vec<Value> = js.call(
            "iterable",
            (js.method(constructor, "values", vec![files.clone()])?,),
        )?;
        for file in entries {
            js.call::<(), _>("remove", (file, "content"))?;
        }
    }
    if js.truthy(&optional(js, &input, "logResponse")?)? {
        script::log(js, "info", "上传配置响应:");
        script::log(
            js,
            "info",
            &js.text(crate::js_values::stringify(&js.ctx, body, 2)?)?,
        );
    }
    let gitlab = files.is_array();
    let files = if gitlab {
        let result = object(js)?;
        let entries: Vec<Value> = js.call("iterable", (files,))?;
        for file in entries {
            js.call::<(), _>("set", (result.clone(), js.get(&file, "path")?, file))?;
        }
        result
    } else if js.truthy(&files)? {
        files
    } else {
        object(js)?
    };
    let encoder: rquickjs::Function = js.ctx.globals().get("encodeURIComponent")?;
    let encoded: Value = encoder.call((js.get(&input, "name")?,))?;
    let constructor: Value = js.ctx.globals().get("Object")?;
    let keys = js.method(constructor, "keys", vec![files.clone()])?;
    let file: Value = js.call("get", (files, encoded.clone()))?;
    let mut url = optional(js, &file, "raw_url")?;
    let original = url.clone();
    if !gitlab && !url.is_null() && !url.is_undefined() {
        let pattern: Value = js.ctx.eval(r"/\/raw\/[^/]*\/(.*)/")?;
        url = js.method(url, "replace", vec![pattern, js.value("/raw/$1")?])?;
    }
    script::log(
        js,
        "info",
        &format!(
            "上传配置完成\n文件列表: {}\n当前文件: {}\n响应返回的原始链接: {}\n处理完的新链接: {}",
            js.text(js.method(keys, "join", vec![js.value(", ")?])?)?,
            js.text(encoded)?,
            js.text(original)?,
            js.text(url.clone())?,
        ),
    );
    let result = object(js)?;
    js.set(&result, "url", url)?;
    Ok(result)
}
