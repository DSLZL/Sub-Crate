//! Existing subscription/script download algorithm, using native storage and I/O.
use crate::{js_language::Js, script, script_network as network};
use rquickjs::{
    Ctx, Function, Object, Value,
    prelude::{Async, Func, Rest},
};

#[derive(Clone)]
struct Download<'js> {
    url: Value<'js>,
    ua: Value<'js>,
    timeout: Value<'js>,
    proxy: Value<'js>,
    skip_custom: Value<'js>,
    await_custom: Value<'js>,
    no_cache: Value<'js>,
    preprocess: Value<'js>,
    options: Value<'js>,
}

struct Finalized<'js> {
    result: Value<'js>,
    raw: Value<'js>,
    cache: bool,
}

fn optional<'js>(js: &Js<'js>, value: &Value<'js>, key: &str) -> rquickjs::Result<Value<'js>> {
    js.call("optional", (value.clone(), key))
}

fn cached<'js>(js: &Js<'js>, id: Value<'js>) -> rquickjs::Result<Value<'js>> {
    let cache = js.ctx.globals().get("__resourceCache")?;
    js.method(cache, "get", vec![id])
}

fn put<'js>(
    js: &Js<'js>,
    name: &str,
    id: Value<'js>,
    value: Value<'js>,
    seconds: Value<'js>,
) -> rquickjs::Result<()> {
    let ttl = if js.truthy(&seconds)? {
        js.call("multiply", (seconds, 1000))?
    } else {
        js.undefined()
    };
    let cache = js.ctx.globals().get(name)?;
    js.method(cache, "set", vec![id, value, ttl])?;
    Ok(())
}

fn format<'js>(
    js: &Js<'js>,
    result: Value<'js>,
    raw: Value<'js>,
    return_raw: bool,
) -> rquickjs::Result<Value<'js>> {
    if !return_raw {
        return Ok(result);
    }
    let output = Object::new(js.ctx.clone())?;
    output.set("result", result)?;
    output.set("raw", raw)?;
    Ok(output.into_value())
}

fn mask_url<'js>(js: &Js<'js>, value: Value<'js>) -> rquickjs::Result<String> {
    let pattern: Value = js.call("regex", (r"([?#&]age-secret-key=)[^&#\s]+", "gi"))?;
    let value = js.method(value, "replace", vec![pattern, js.value("$1***")?])?;
    let pattern: Value = js.call(
        "regex",
        (
            r"\bAGE-SECRET-KEY(?:-PQ)?-1[023456789ACDEFGHJKLMNPQRSTUVWXYZ]+\b",
            "g",
        ),
    )?;
    let replace = Function::new(js.ctx.clone(), |value: String| {
        let prefix = if value.starts_with("AGE-SECRET-KEY-PQ-1") {
            "AGE-SECRET-KEY-PQ-1"
        } else {
            "AGE-SECRET-KEY-1"
        };
        format!("{prefix}...{}", &value[value.len().saturating_sub(6)..])
    })?;
    js.text(js.method(value, "replace", vec![pattern, replace.into_value()])?)
}

fn prefixed<'js>(
    js: &Js<'js>,
    url: Value<'js>,
    settings: &Value<'js>,
) -> rquickjs::Result<Value<'js>> {
    let proxy = js.get(settings, "githubProxy")?;
    let pattern = js.get(settings, "githubProxyRegex")?;
    if !js.truthy(&proxy)? || !js.truthy(&pattern)? || !url.is_string() {
        return Ok(url);
    }
    let http: Value = js.call("regex", (r"^https?://", "i"))?;
    if !js.regex_test(http, url.clone())? {
        return Ok(url);
    }
    let prefix = format!("{}/", js.text(proxy)?);
    if js.truthy(&js.method(url.clone(), "startsWith", vec![js.value(prefix.clone())?])?)? {
        return Ok(url);
    }
    let matched = (|| {
        let pattern = js.text(pattern)?;
        let pattern = pattern.trim();
        if pattern.is_empty() {
            return Ok(false);
        }
        let expression = js.call("regex", (pattern, "i"))?;
        js.regex_test(expression, url.clone())
    })();
    match matched {
        Ok(true) => js.value(format!("{prefix}{}", js.text(url)?)),
        Ok(false) => Ok(url),
        Err(error) => {
            eprintln!(
                "[sub-store:error] GitHub 加速代理匹配正则无效: {}",
                script::message(&js.ctx, error)
            );
            Ok(url)
        }
    }
}

async fn finalize<'js>(
    js: &Js<'js>,
    body: Value<'js>,
    secret: Value<'js>,
    preprocess: bool,
) -> rquickjs::Result<Finalized<'js>> {
    let utils: Value = js.ctx.globals().get("ProxyUtils")?;
    let mut result = if js.truthy(&secret)? {
        crate::script_age::decrypt(js, body, secret).await?
    } else {
        body
    };
    let raw = result.clone();
    if preprocess {
        if let Ok(parser) = crate::clash::Parser::new()
            && let Some(lines) = parser.preprocess(&js.text(result.clone())?)
        {
            let mut output = String::from("proxies:\n");
            for line in lines.split_terminator('\n') {
                output.push_str("  - ");
                output.push_str(line);
                output.push('\n');
            }
            result = js.value(output)?;
        }
        let parsed = js.method(utils, "parse", vec![result.clone()]);
        let cache = match parsed {
            Ok(value) => value.as_array().is_some_and(|proxies| !proxies.is_empty()),
            Err(error) => {
                eprintln!(
                    "[sub-store:error] Downloaded resource cannot be parsed: {}",
                    script::message(&js.ctx, error)
                );
                false
            }
        };
        return Ok(Finalized { result, raw, cache });
    }
    Ok(Finalized {
        result,
        raw,
        cache: true,
    })
}

async fn finish<'js>(
    js: &Js<'js>,
    body: Value<'js>,
    secret: Value<'js>,
    preprocess: bool,
    return_raw: bool,
) -> rquickjs::Result<Value<'js>> {
    let value = finalize(js, body, secret, preprocess).await?;
    format(js, value.result, value.raw, return_raw)
}

async fn run<'js>(js: &Js<'js>, input: Download<'js>) -> rquickjs::Result<Value<'js>> {
    let pattern: Value = js.call("regex", ("#noFlow$", ""))?;
    let clean = js.method(input.url.clone(), "replace", vec![pattern, js.value("")?])?;
    let (url, arguments) = network::fragment(js, clean)?;
    let options_object = !input.options.is_null()
        && js.call::<String, _>("typeOf", (input.options.clone(),))? == "object";
    let secret = network::prefer(
        js,
        [
            if options_object {
                network::prefer(
                    js,
                    [
                        optional(js, &input.options, "age-secret-key")?,
                        optional(js, &input.options, "ageSecretKey")?,
                    ],
                )?
            } else {
                js.undefined()
            },
            optional(js, &arguments, "age-secret-key")?,
        ],
    )?;
    let return_raw = options_object && js.truthy(&optional(js, &input.options, "returnRaw")?)?;
    let preprocess = js.truthy(&input.preprocess)?;
    let settings = network::settings(js)?;
    let proxy = network::prefer(
        js,
        [
            input.proxy.clone(),
            js.get(&settings, "defaultProxy")?,
            std::env::var("SUB_STORE_BACKEND_DEFAULT_PROXY")
                .ok()
                .map(|value| js.value(value))
                .transpose()?
                .unwrap_or_else(|| js.undefined()),
        ],
    )?;
    let user_agent = network::prefer(
        js,
        [
            input.ua.clone(),
            js.get(&settings, "defaultUserAgent")?,
            js.value("clash.meta/v1.19.23")?,
        ],
    )?;
    let custom = optional(js, &arguments, "headers")?;
    let custom_headers = if js.truthy(&custom)? {
        let parsed = (|| {
            let value = js.call("jsonParse", (custom,))?;
            network::headers(js, value, user_agent.clone())
        })();
        match parsed {
            Ok(value) => value,
            Err(error) => {
                eprintln!(
                    "[sub-store:error] 解析自定义 headers 失败: {}",
                    script::message(&js.ctx, error)
                );
                js.undefined()
            }
        }
    } else {
        js.undefined()
    };
    let timeout = network::prefer(
        js,
        [
            input.timeout.clone(),
            js.get(&settings, "defaultTimeout")?,
            js.value(8000)?,
        ],
    )?;
    let url = prefixed(js, url, &settings)?;
    let safe_url = mask_url(js, url.clone())?;
    let identity = if js.truthy(&custom_headers)? {
        script::serialized(&js.ctx, custom_headers.clone())?
    } else {
        js.text(user_agent.clone())?
    };
    let utils: Value = js.ctx.globals().get("ProxyUtils")?;
    let id = js.method(
        utils,
        "hex_md5",
        vec![js.value(format!("{identity}{}", js.text(url.clone())?))?],
    )?;
    let cache_key = optional(js, &arguments, "cacheKey")?;
    let custom_key = if js.equal(&cache_key, &js.value(true)?)? {
        eprintln!("[sub-store:error] 使用自定义缓存时 cacheKey 的值不能为空");
        None
    } else if js.truthy(&cache_key)? {
        Some(js.value(format!("#sub-store-cached-custom-{}", js.text(cache_key)?))?)
    } else {
        None
    };
    let bypass = js.truthy(&input.no_cache)? || js.truthy(&optional(js, &arguments, "noCache")?)?;
    if let Some(key) = &custom_key
        && !js.truthy(&input.skip_custom)?
    {
        let custom = js.method(network::host(js)?, "read", vec![key.clone()])?;
        let cached = cached(js, id.clone())?;
        if !bypass && js.truthy(&cached)? {
            return finish(js, cached, secret, preprocess, return_raw).await;
        }
        if js.truthy(&custom)? {
            let pattern = js.call("regex", (r"(\?|&)cacheKey=.*?(&|$)", ""))?;
            let mut refresh = input.clone();
            refresh.url = js.method(input.url.clone(), "replace", vec![pattern, js.value("")?])?;
            refresh.proxy = proxy.clone();
            refresh.skip_custom = js.value(true)?;
            refresh.await_custom = js.undefined();
            refresh.no_cache = js.undefined();
            if js.truthy(&input.await_custom)? {
                if let Err(error) = Box::pin(run(js, refresh)).await {
                    eprintln!(
                        "[sub-store:error] 乐观缓存: URL {safe_url} 更新缓存发生错误 {}",
                        script::message(&js.ctx, error)
                    );
                    let cache = js.ctx.globals().get("__resourceCache")?;
                    js.method(cache, "set", vec![id, custom.clone()])?;
                }
            } else {
                let worker = js.clone();
                js.ctx.spawn(async move {
                    if let Err(error) = Box::pin(run(&worker, refresh)).await {
                        eprintln!(
                            "[sub-store:error] 乐观缓存: URL {safe_url} 异步更新缓存发生错误 {}",
                            script::message(&worker.ctx, error)
                        );
                    }
                });
            }
            return finish(js, custom, secret, preprocess, return_raw).await;
        }
    }
    let internal: Value = js.call("regex", (r"^/api/(file|module)/(.+)", ""))?;
    let matched = js.method(url.clone(), "match", vec![internal])?;
    if js.truthy(&matched)? {
        let kind = js.get(&matched, "1")?;
        let type_name = js.text(kind.clone())?;
        let found = async {
            let encoded = js.get(&matched, "2")?;
            let decode: Function = js.ctx.globals().get("decodeURIComponent")?;
            let name: Value = decode.call((encoded,))?;
            let key = if type_name == "module" {
                "modules"
            } else {
                "files"
            };
            let values = js.method(network::host(js)?, "read", vec![js.value(key)?])?;
            let selected_name = name.clone();
            let scope = js.clone();
            let predicate = Function::new(js.ctx.clone(), move |value: Value<'js>| {
                scope.equal(&scope.get(&value, "name")?, &selected_name)
            })?;
            let value = js.method(values, "find", vec![predicate.into_value()])?;
            if !js.truthy(&value)? {
                return Err(js.error("Source not found"));
            }
            if type_name == "module" {
                return js.get(&value, "content");
            }
            let options = Object::new(js.ctx.clone())?;
            options.set("type", "file")?;
            options.set("name", name)?;
            options.set("noFlow", optional(js, &input.options, "noFlow")?)?;
            let artifact: Function = js.ctx.globals().get("produceArtifact")?;
            script::settled(js, artifact.call((options,))?).await
        }
        .await;
        return match found {
            Ok(value) => format(js, value.clone(), value, return_raw),
            Err(error) => {
                script::message(&js.ctx, error);
                Err(js.error(&format!("无法加载 {type_name}: {}", js.text(url)?)))
            }
        };
    }
    let path = js.text(url.clone())?;
    if path.starts_with('/') || std::path::Path::new(&path).is_absolute() {
        return match std::fs::read(&path) {
            Ok(bytes) => {
                let value = js.value(String::from_utf8_lossy(&bytes).into_owned())?;
                format(js, value.clone(), value, return_raw)
            }
            Err(_) => Err(js.error(&format!("无法从该路径读取文本内容: {}", js.text(url)?))),
        };
    }
    let value = cached(js, id.clone())?;
    let finalized = if !bypass && js.truthy(&value)? {
        let finalized = finalize(js, value.clone(), secret.clone(), preprocess).await?;
        if let Some(key) = &custom_key {
            js.method(network::host(js)?, "write", vec![value, key.clone()])?;
        }
        finalized
    } else {
        let result = async {
            let options = Object::new(js.ctx.clone())?;
            options.set("url", url.clone())?;
            options.set("timeout", timeout)?;
            let headers = if js.truthy(&custom_headers)? {
                custom_headers
            } else {
                let headers = Object::new(js.ctx.clone())?;
                headers.set("User-Agent", user_agent)?;
                headers.into_value()
            };
            options.set("headers", headers)?;
            options.set("proxy", proxy.clone())?;
            options.set(
                "insecure",
                js.truthy(&optional(js, &arguments, "insecure")?)?,
            )?;
            let response = network::request(js, "get", options.into_value()).await?;
            let status = js.get(&response, "statusCode")?;
            if js.call::<bool, _>("lt", (status.clone(), 200))?
                || js.call::<bool, _>("le", (400, status.clone()))?
            {
                return Err(js.error(&format!("statusCode: {}", js.text(status)?)));
            }
            let headers = js.get(&response, "headers")?;
            if js.truthy(&headers)? {
                let flow: Value = js.ctx.globals().get("flowUtils")?;
                let flow = js.method(flow, "getFlowField", vec![headers])?;
                if js.truthy(&flow)? {
                    put(
                        js,
                        "__headersResourceCache",
                        id.clone(),
                        flow,
                        optional(js, &arguments, "headersCacheTtl")?,
                    )?;
                }
            }
            let body = js.get(&response, "body")?;
            let pattern = js.call("regex", (r"\s", "g"))?;
            let stripped = js.method(body.clone(), "replace", vec![pattern, js.value("")?])?;
            let length: usize = js.call("length", (stripped,))?;
            if length == 0 {
                return Err(js.error("Error: 远程资源内容为空"));
            }
            let finalized = finalize(js, body.clone(), secret.clone(), preprocess).await?;
            let threshold =
                network::prefer(js, [js.get(&settings, "cacheThreshold")?, js.value(1024)?])?;
            let length: f64 = js.call("length", (body.clone(),))?;
            let oversize =
                js.truthy(&threshold)? && js.call::<bool, _>("gt", (length / 1024.0, threshold))?;
            if finalized.cache && !oversize {
                put(
                    js,
                    "__resourceCache",
                    id,
                    body.clone(),
                    optional(js, &arguments, "cacheTtl")?,
                )?;
                if let Some(key) = &custom_key {
                    js.method(network::host(js)?, "write", vec![body, key.clone()])?;
                }
            }
            Ok::<_, rquickjs::Error>(finalized)
        }
        .await;
        match result {
            Ok(finalized) => finalized,
            Err(error) => {
                let message = script::message(&js.ctx, error);
                if let Some(key) = custom_key {
                    let fallback = js.method(network::host(js)?, "read", vec![key])?;
                    if js.truthy(&fallback)? {
                        return finish(js, fallback, secret, preprocess, return_raw).await;
                    }
                }
                return Err(js.error(&format!("无法下载 URL {safe_url}: {message}")));
            }
        }
    };
    if js.truthy(&optional(js, &arguments, "validCheck")?)?
        && !js.truthy(&optional(js, &arguments, "noFlow")?)?
        && !js.truthy(&optional(js, &input.options, "noFlow")?)?
    {
        let flow: Value = js.ctx.globals().get("flowUtils")?;
        let headers = js.method(
            flow.clone(),
            "getFlowHeaders",
            vec![
                url,
                optional(js, &arguments, "flowUserAgent")?,
                js.undefined(),
                proxy,
                optional(js, &arguments, "flowUrl")?,
                optional(js, &arguments, "flowHeaders")?,
            ],
        )?;
        let headers = script::settled(js, headers).await?;
        let parsed = js.method(flow.clone(), "parseFlowHeaders", vec![headers])?;
        js.method(flow, "validCheck", vec![parsed])?;
    }
    format(js, finalized.result, finalized.raw, return_raw)
}

async fn download<'js>(ctx: Ctx<'js>, arguments: Rest<Value<'js>>) -> rquickjs::Result<Value<'js>> {
    let js = Js::new(ctx);
    // Rest retains all nine public arguments beyond the binding's tuple limit.
    let mut arguments = arguments.0.into_iter();
    let input = Download {
        url: arguments
            .next()
            .filter(|value| !value.is_undefined())
            .unwrap_or(js.value("")?),
        ua: arguments.next().unwrap_or_else(|| js.undefined()),
        timeout: arguments.next().unwrap_or_else(|| js.undefined()),
        proxy: arguments.next().unwrap_or_else(|| js.undefined()),
        skip_custom: arguments.next().unwrap_or_else(|| js.undefined()),
        await_custom: arguments.next().unwrap_or_else(|| js.undefined()),
        no_cache: arguments.next().unwrap_or_else(|| js.undefined()),
        preprocess: arguments.next().unwrap_or_else(|| js.undefined()),
        options: arguments
            .next()
            .filter(|value| !value.is_undefined())
            .unwrap_or(Object::new(js.ctx.clone())?.into_value()),
    };
    run(&js, input).await
}

pub(crate) fn initialize<'js>(_ctx: &Ctx<'js>, utils: &Object<'js>) -> rquickjs::Result<()> {
    utils.set("download", Func::from(Async(download)))?;
    utils.set(
        "downloadFile",
        Func::from(Async(crate::script_io::download_file)),
    )
}
