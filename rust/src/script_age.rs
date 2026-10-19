//! Existing age application validation, using the locked generic crypto library.
use crate::{js_language::Js, script};
use rquickjs::{
    Ctx, Object, Value,
    prelude::{Async, Func, Opt},
};

fn library<'js>(js: &Js<'js>) -> rquickjs::Result<Value<'js>> {
    js.ctx.globals().get("SubStoreAgeUtilities")
}

fn key<'js>(js: &Js<'js>, value: Value<'js>) -> rquickjs::Result<Value<'js>> {
    let value = if value.is_null() || value.is_undefined() {
        js.value("")?
    } else {
        js.value(js.text(value)?)?
    };
    js.method(value, "trim", vec![])
}

fn invalid(js: &Js<'_>, public: bool) -> rquickjs::Result<rquickjs::Error> {
    let error = Object::new(js.ctx.clone())?;
    error.set(
        "code",
        if public {
            "INVALID_AGE_PUBLIC_KEY"
        } else {
            "INVALID_AGE_SECRET_KEY"
        },
    )?;
    error.set("message", if public {
        "age-public-key 仅支持 X25519(age1...) 或 MLKEM768-X25519(age1pq1...) 公钥"
    } else {
        "age-secret-key 仅支持 X25519(AGE-SECRET-KEY-1...) 或 MLKEM768-X25519(AGE-SECRET-KEY-PQ-1...) 私钥"
    })?;
    error.set("details", js.undefined())?;
    error.set("type", "RequestInvalidError")?;
    Ok(js.ctx.throw(error.into_value()))
}

fn runtime(js: &Js<'_>) -> rquickjs::Result<()> {
    let globals = js.ctx.globals();
    let mut missing = Vec::new();
    for name in [
        "TextEncoder",
        "TextDecoder",
        "ReadableStream",
        "TransformStream",
        "Response",
        "Uint8Array",
    ] {
        if globals.get::<_, Value>(name)?.is_undefined() {
            missing.push(name);
        }
    }
    let crypto: Value = globals.get("crypto")?;
    let random: Value = js.call("optional", (crypto.clone(), "getRandomValues"))?;
    if !random.is_function() {
        missing.push("crypto.getRandomValues");
    }
    if !missing.is_empty() {
        return Err(js.error(&format!(
            "当前运行环境缺少 age 加解密所需 Web API: {}",
            missing.join(", ")
        )));
    }
    let subtle: Value = js.call("optional", (crypto.clone(), "subtle"))?;
    let import: Value = js.call("optional", (subtle.clone(), "importKey"))?;
    let derive: Value = js.call("optional", (subtle, "deriveBits"))?;
    if !import.is_function() || !derive.is_function() {
        let fallback: Value = js.ctx.eval("({importKey(){throw new ReferenceError('X25519 WebCrypto is not available')},deriveBits(){throw new ReferenceError('X25519 WebCrypto is not available')}})")?;
        js.set(&crypto, "subtle", fallback)?;
    }
    Ok(())
}

fn instance<'js>(js: &Js<'js>, name: &str) -> rquickjs::Result<Value<'js>> {
    js.call("construct", (js.get(&library(js)?, name)?, js.array([])?))
}

fn recipient<'js>(js: &Js<'js>, input: Value<'js>) -> rquickjs::Result<Value<'js>> {
    let key = key(js, input)?;
    let text = js.text(key.clone())?;
    if !text.starts_with("age1") || text.starts_with("age1tag1") || text.starts_with("age1tagpq1") {
        return Err(invalid(js, true)?);
    }
    if let Err(error) = instance(js, "Encrypter")
        .and_then(|value| js.method(value, "addRecipient", vec![key.clone()]))
    {
        script::message(&js.ctx, error);
        return Err(invalid(js, true)?);
    }
    Ok(key)
}

pub(crate) async fn identity<'js>(js: &Js<'js>, input: Value<'js>) -> rquickjs::Result<Value<'js>> {
    runtime(js)?;
    let key = key(js, input)?;
    let text = js.text(key.clone())?;
    if !text.starts_with("AGE-SECRET-KEY-1") && !text.starts_with("AGE-SECRET-KEY-PQ-1") {
        return Err(invalid(js, false)?);
    }
    let result = async {
        let pending = js.method(library(js)?, "identityToRecipient", vec![key.clone()])?;
        script::settled(js, pending).await
    }
    .await;
    if let Err(error) = result {
        script::message(&js.ctx, error);
        return Err(invalid(js, false)?);
    }
    Ok(key)
}

async fn derive<'js>(ctx: Ctx<'js>, input: Opt<Value<'js>>) -> rquickjs::Result<Value<'js>> {
    let js = Js::new(ctx);
    let key = identity(&js, input.0.unwrap_or_else(|| js.undefined())).await?;
    let pending = js.method(library(&js)?, "identityToRecipient", vec![key])?;
    let public = script::settled(&js, pending).await?;
    recipient(&js, public)
}

async fn encrypt<'js>(
    ctx: Ctx<'js>,
    text: Opt<Value<'js>>,
    public: Opt<Value<'js>>,
) -> rquickjs::Result<Value<'js>> {
    let js = Js::new(ctx);
    runtime(&js)?;
    let public = recipient(&js, public.0.unwrap_or_else(|| js.undefined()))?;
    let encrypter = instance(&js, "Encrypter")?;
    js.method(encrypter.clone(), "addRecipient", vec![public])?;
    let text = text
        .0
        .filter(|value| !value.is_null() && !value.is_undefined())
        .unwrap_or(js.value("")?);
    let pending = js.method(encrypter, "encrypt", vec![text])?;
    let bytes = script::settled(&js, pending).await?;
    js.method(js.get(&library(&js)?, "armor")?, "encode", vec![bytes])
}

pub(crate) async fn decrypt<'js>(
    js: &Js<'js>,
    input: Value<'js>,
    secret: Value<'js>,
) -> rquickjs::Result<Value<'js>> {
    let body = if input.is_null() || input.is_undefined() {
        js.value("")?
    } else {
        input
    };
    let text = if body.is_string() {
        body.clone()
    } else {
        let decoder: Value = js.ctx.globals().get("TextDecoder")?;
        let decoder = js.call("construct", (decoder, js.array([])?))?;
        js.method(decoder, "decode", vec![body.clone()])?
    };
    if !js
        .text(key(js, text.clone())?)?
        .starts_with("-----BEGIN AGE ENCRYPTED FILE-----")
    {
        return Ok(body);
    }
    let secret = identity(js, secret).await?;
    let decrypter = instance(js, "Decrypter")?;
    js.method(decrypter.clone(), "addIdentity", vec![secret])?;
    let result = async {
        let bytes = js.method(js.get(&library(js)?, "armor")?, "decode", vec![text])?;
        let pending = js.method(decrypter, "decrypt", vec![bytes, js.value("text")?])?;
        script::settled(js, pending).await
    }
    .await;
    result.map_err(|error| {
        js.error(&format!(
            "age 解密失败: {}",
            script::message(&js.ctx, error)
        ))
    })
}

async fn decrypt_public<'js>(
    ctx: Ctx<'js>,
    input: Opt<Value<'js>>,
    secret: Opt<Value<'js>>,
) -> rquickjs::Result<Value<'js>> {
    let js = Js::new(ctx);
    decrypt(
        &js,
        input.0.unwrap_or_else(|| js.undefined()),
        secret.0.unwrap_or_else(|| js.undefined()),
    )
    .await
}

pub(crate) async fn rest<'js>(js: &Js<'js>, input: Value<'js>) -> rquickjs::Result<Value<'js>> {
    let body = js.get(&input, "body")?;
    let result = async {
        if js.equal(&js.get(&input, "action")?, &js.value("public-key")?)? {
            let secret = crate::script_pipeline::optional(js, &body, "age-secret-key")?;
            let public = derive(js.ctx.clone(), Opt(Some(secret))).await?;
            let data = crate::script_pipeline::object(js)?;
            js.set(&data, "age-public-key", public)?;
            return Ok(data);
        }
        runtime(js)?;
        let kind = crate::script_pipeline::optional(js, &body, "type")?;
        let kind = if js.truthy(&kind)? { kind } else { js.value("x25519")? };
        let method = if js.equal(&kind, &js.value("x25519")?)? { "generateX25519Identity" }
            else if js.equal(&kind, &js.value("mlkem768-x25519")?)? { "generateHybridIdentity" }
            else { return Err(js.error(&format!("不支持的 age key 类型: {}. 支持: x25519, mlkem768-x25519", js.text(kind)?))); };
        let secret = script::settled(js, js.method(library(js)?, method, vec![])?).await?;
        let public = derive(js.ctx.clone(), Opt(Some(secret.clone()))).await?;
        let data = crate::script_pipeline::object(js)?;
        js.set(&data, "type", kind)?;
        js.set(&data, "age-secret-key", secret)?;
        js.set(&data, "age-public-key", public)?;
        Ok(data)
    }.await;
    let wrapped = crate::script_pipeline::object(js)?;
    match result {
        Ok(data) => js.set(&wrapped, "data", data)?,
        Err(error) if error.is_exception() => {
            let thrown = js.ctx.catch();
            let error = if crate::script_pipeline::optional(js, &thrown, "type")?.is_undefined() {
                let error = crate::script_pipeline::object(js)?;
                js.set(&error, "message", crate::script_pipeline::optional(js, &thrown, "message")?)?;
                error
            } else { thrown };
            js.set(&wrapped, "error", error)?;
        }
        Err(error) => return Err(error),
    }
    Ok(wrapped)
}

pub(crate) fn initialize<'js>(ctx: &Ctx<'js>, utils: &Object<'js>) -> rquickjs::Result<()> {
    ctx.eval::<(), _>(include_bytes!("../vendor/web-utilities.js").as_slice())?;
    ctx.eval::<(), _>(include_str!("script-web.js"))?;
    ctx.eval::<(), _>(include_bytes!("../vendor/age-utilities.js").as_slice())?;
    let age = Object::new(ctx.clone())?;
    age.set("encrypt", Func::from(Async(encrypt)))?;
    age.set("decrypt", Func::from(Async(decrypt_public)))?;
    age.set("derivePublicKey", Func::from(Async(derive)))?;
    utils.set("age", age)
}
