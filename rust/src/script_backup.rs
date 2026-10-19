//! Backup data rules use the same locked age/base64/lodash utilities as user scripts.
use crate::{
    js_language::Js,
    script, script_age,
    script_pipeline::{object, optional},
};
use rquickjs::Value;

fn nullish(value: &Value<'_>) -> bool {
    value.is_null() || value.is_undefined()
}

fn base64<'js>(js: &Js<'js>) -> rquickjs::Result<Value<'js>> {
    let utils: Value = js.ctx.globals().get("ProxyUtils")?;
    js.get(&utils, "Base64")
}

pub(crate) fn serialize<'js>(js: &Js<'js>, input: Value<'js>) -> rquickjs::Result<Value<'js>> {
    let content = js.get(&input, "content")?;
    let settings = js.get(&content, "settings")?;
    let encoding = js.text(js.get(&input, "encoding")?)?;
    if encoding != "age" && js.truthy(&optional(js, &settings, "age-secret-key")?)? {
        js.call::<(), _>("remove", (settings.clone(), "age-secret-key"))?;
    }
    if encoding == "plaintext" {
        let settings = if js.truthy(&settings)? {
            settings
        } else {
            object(js)?
        };
        js.set(
            &settings,
            "gistToken",
            js.value("恢复后请重新设置 GitHub Token")?,
        )?;
        js.set(&content, "settings", settings)?;
    }
    let text = crate::js_values::stringify(&js.ctx, content, 2)?;
    if encoding == "plaintext" {
        Ok(text)
    } else {
        js.method(base64(js)?, "encode", vec![text])
    }
}

pub(crate) async fn crypt<'js>(
    js: &Js<'js>,
    input: Value<'js>,
    encrypt: bool,
) -> rquickjs::Result<Value<'js>> {
    let body = js.get(&input, "body")?;
    if !js.equal(&js.get(&input, "encoding")?, &js.value("age")?)? {
        return Ok(body);
    }
    let settings = js.get(&input, "settings")?;
    let secret = optional(js, &settings, "age-secret-key")?;
    let secret = if nullish(&secret) {
        String::new()
    } else {
        js.text(secret)?
    };
    let secret = crate::json::trim(&secret);
    if secret.is_empty() {
        return Err(js.error("age 加密模式需要配置 age 解密私钥"));
    }
    if !encrypt {
        script::log(js, "info", "尝试使用 age 解密 Gist 备份内容");
        return script_age::decrypt(js, body, js.value(secret)?).await;
    }
    script::log(js, "info", "使用 age 加密 Gist 备份内容");
    let utils: Value = js.ctx.globals().get("ProxyUtils")?;
    let age = js.get(&utils, "age")?;
    let recipient = script::settled(
        js,
        js.method(age.clone(), "derivePublicKey", vec![js.value(secret)?])?,
    )
    .await?;
    script::settled(js, js.method(age, "encrypt", vec![body, recipient])?).await
}

pub(crate) async fn settings<'js>(
    js: &Js<'js>,
    settings: Value<'js>,
) -> rquickjs::Result<Value<'js>> {
    let result = async {
        let raw = js.get(&settings, "age-secret-key")?;
        let key = if nullish(&raw) {
            String::new()
        } else {
            js.text(raw)?
        };
        let key = crate::json::trim(&key);
        if key.is_empty() {
            js.call::<(), _>("remove", (settings.clone(), "age-secret-key"))?;
            let error = object(js)?;
            js.set(&error, "code", js.value("INVALID_GIST_AGE_KEYS")?)?;
            js.set(&error, "type", js.value("RequestInvalidError")?)?;
            js.set(
                &error,
                "message",
                js.value("age 加密模式需要配置 age-secret-key")?,
            )?;
            return Err(js.ctx.throw(error));
        }
        let key = script_age::identity(js, js.value(key)?).await?;
        js.set(&settings, "age-secret-key", key)?;
        Ok(settings)
    }
    .await;
    let wrapped = object(js)?;
    match result {
        Ok(settings) => js.set(&wrapped, "settings", settings)?,
        Err(error) if error.is_exception() => {
            let thrown = js.ctx.catch();
            if optional(js, &thrown, "type")?.as_string().is_some() {
                js.set(&wrapped, "error", thrown)?;
            } else {
                return Err(js.ctx.throw(thrown));
            }
        }
        Err(error) => return Err(error),
    }
    Ok(wrapped)
}

pub(crate) fn decode<'js>(js: &Js<'js>, input: Value<'js>) -> rquickjs::Result<Value<'js>> {
    let mut content = js.get(&input, "body")?;
    let parse = (|| {
        content = js.call(
            "jsonParse",
            (js.method(base64(js)?, "decode", vec![content.clone()])?,),
        )?;
        let constructor: Value = js.ctx.globals().get("Object")?;
        js.method(constructor, "keys", vec![js.get(&content, "settings")?])?;
        Ok::<_, rquickjs::Error>(())
    })();
    if let Err(error) = parse {
        if error.is_exception() {
            js.ctx.catch();
        }
        let fallback = (|| {
            content = js.call("jsonParse", (content.clone(),))?;
            let constructor: Value = js.ctx.globals().get("Object")?;
            js.method(constructor, "keys", vec![js.get(&content, "settings")?])?;
            Ok::<_, rquickjs::Error>(())
        })();
        if let Err(error) = fallback {
            if error.is_exception() {
                js.ctx.catch();
            }
            return Err(js.error("Gist 备份文件校验失败, 无法还原"));
        }
    }
    let keep = js.get(&input, "keep")?;
    let current = js.get(&input, "current")?;
    let lodash: Value = js.ctx.globals().get("lodash")?;
    let paths: Vec<Value> = js.call("iterable", (keep,))?;
    for path in paths {
        if js.truthy(&js.method(lodash.clone(), "has", vec![current.clone(), path.clone()])?)? {
            let value = js.method(lodash.clone(), "get", vec![current.clone(), path.clone()])?;
            js.method(lodash.clone(), "set", vec![content.clone(), path, value])?;
        } else {
            js.method(lodash.clone(), "unset", vec![content.clone(), path])?;
        }
    }
    Ok(content)
}

pub(crate) fn restored_artifacts<'js>(
    js: &Js<'js>,
    gist: Value<'js>,
) -> rquickjs::Result<Value<'js>> {
    let files = optional(js, &gist, "files")?;
    if !js.truthy(&files)? {
        return Err(js.error("找不到 Sub-Store Gist 文件列表"));
    }
    let items = crate::script_pipeline::read(js, "artifacts")?;
    let constructor: Value = js.ctx.globals().get("Object")?;
    let keys: Vec<Value> = js.call(
        "iterable",
        (js.method(constructor, "keys", vec![files.clone()])?,),
    )?;
    let encoder: rquickjs::Function = js.ctx.globals().get("encodeURIComponent")?;
    for key in keys {
        let file: Value = js.call("get", (files.clone(), key))?;
        let name = optional(js, &file, "filename")?;
        if !js.truthy(&name)? {
            continue;
        }
        if js.equal(&name, &js.value(".sub-store-placeholder")?)? {
            script::log(
                js,
                "info",
                &format!("忽略 Gist 占位文件: {}", js.text(name)?),
            );
            continue;
        }
        let encoded: Value = encoder.call((name.clone(),))?;
        if !js.equal(&encoded, &name)? {
            script::log(
                js,
                "error",
                &format!("文件名 {} 未编码 不保存", js.text(name)?),
            );
            continue;
        }
        let old = crate::script_pipeline::find(js, items.clone(), name.clone())?;
        let item = if js.truthy(&old)? {
            js.call("spread", (object(js)?, old))?
        } else {
            let item = object(js)?;
            js.set(&item, "name", js.value(js.text(name)?)?)?;
            item
        };
        let pattern: Value = js.call("regex", (r"/raw/[^/]*/(.*)", ""))?;
        let url = js.method(
            js.get(&file, "raw_url")?,
            "replace",
            vec![pattern, js.value("/raw/$1")?],
        )?;
        js.set(&item, "url", url)?;
        // Updating the existing object preserves its position and unknown fields.
        let length: usize = js.call("length", (items.clone(),))?;
        let mut replaced = false;
        for index in 0..length {
            let current: Value = js.call("get", (items.clone(), index))?;
            if js.equal(&js.get(&current, "name")?, &js.get(&item, "name")?)? {
                js.call::<(), _>("set", (items.clone(), index, item.clone()))?;
                replaced = true;
                break;
            }
        }
        if !replaced {
            js.method(items.clone(), "push", vec![item])?;
        }
    }
    Ok(items)
}
