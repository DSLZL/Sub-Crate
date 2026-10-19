//! Download output stages share the live script context with source processing.
use crate::{js_language::Js, script, script_network as network, script_pipeline as pipeline};
use pipeline::{find, object, optional, read, utility};
use rquickjs::Value;

pub(crate) async fn node_info<'js>(js: &Js<'js>, input: Value<'js>) -> rquickjs::Result<Value<'js>> {
    let proxy = js.get(&input, "body")?;
    let projected = crate::js_values::project(&js.ctx, proxy.clone())?;
    let share = crate::conversion::uri(&projected);
    let result = async {
        let server = js.text(js.get(&proxy, "server")?)?;
        let server = crate::json::trim(&server).strip_prefix('[').unwrap_or(crate::json::trim(&server));
        let server = server.strip_suffix(']').unwrap_or(server);
        let encoder: rquickjs::Function = js.ctx.globals().get("encodeURIComponent")?;
        let server: String = encoder.call((server,))?;
        let query = js.get(&input, "query")?;
        let lang = network::prefer(js, [js.get(&query, "lang")?, js.value("zh-CN")?])?;
        let request = object(js)?;
        js.set(&request, "url", js.value(format!("http://ip-api.com/json/{server}?lang={}", js.text(lang)?))?)?;
        let headers = object(js)?;
        js.set(&headers, "User-Agent", js.value("Mozilla/5.0 (Macintosh; Intel Mac OS X 12_4) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/15.4 Safari/605.1.15")?)?;
        js.set(&request, "headers", headers)?;
        let response = network::request(js, "get", request).await?;
        let info: Value = js.call("jsonParse", (js.get(&response, "body")?,))?;
        if !js.equal(&js.get(&info, "status")?, &js.value("success")?)? {
            let message = js.get(&info, "message")?;
            return Err(js.error(&if message.is_undefined() { String::new() } else { js.text(message)? }));
        }
        js.call::<(), _>("remove", (info.clone(), "status"))?;
        let data = object(js)?;
        if let Some(share) = share { js.set(&data, "shareUrl", js.value(share)?)?; }
        js.set(&data, "info", info)?;
        Ok(data)
    }.await;
    let wrapped = object(js)?;
    match result {
        Ok(data) => js.set(&wrapped, "data", data)?,
        Err(error) => {
            let error = crate::script_sync::reason(js, error);
            let reason = script::message(&js.ctx, error);
            let failure = object(js)?;
            js.set(&failure, "code", js.value("FAILED_TO_GET_NODE_INFO")?)?;
            js.set(&failure, "type", js.value("NetworkError")?)?;
            js.set(&failure, "message", js.value("Failed to get node info")?)?;
            js.set(&failure, "details", js.value(format!("Reason: {reason}"))?)?;
            js.set(&wrapped, "error", failure)?;
        }
    }
    Ok(wrapped)
}

async fn flow_call<'js>(js: &Js<'js>, args: Vec<Value<'js>>) -> rquickjs::Result<Value<'js>> {
    let flow: Value = js.ctx.globals().get("flowUtils")?;
    script::settled(js, js.method(flow, "getFlowHeaders", args)?).await
}

fn logged<'js>(js: &Js<'js>, result: rquickjs::Result<Value<'js>>) -> Value<'js> {
    result.unwrap_or_else(|error| {
        script::log(js, "error", &format!("flow headers: {}", script::message(&js.ctx, error)));
        js.undefined()
    })
}

async fn remote_flow<'js>(
    js: &Js<'js>,
    url: Value<'js>,
    proxy: Value<'js>,
    collection: bool,
) -> rquickjs::Result<Value<'js>> {
    let lines = js.method(
        js.value(js.text(url)?)?,
        "split",
        vec![js.call("regex", (r"[\r\n]+", ""))?],
    )?;
    let scope = js.clone();
    let lines = js.map(lines, move |value| scope.method(value, "trim", vec![]))?;
    let scope = js.clone();
    let lines = js.filter(lines, move |value, _| {
        scope.truthy(&scope.get(&value, "length")?)
    })?;
    let first = network::prefer(js, [js.get(&lines, "0")?, js.value("")?])?;
    let (url, args) = network::fragment(js, first)?;
    if js.truthy(&optional(js, &args, "noFlow")?)?
        || !js.regex_test(
            js.call(
                "regex",
                (if collection { r"^https?:" } else { r"^https?" }, ""),
            )?,
            url.clone(),
        )?
    {
        return Ok(js.undefined());
    }
    let url = if js.truthy(&optional(js, &args, "insecure")?)? {
        js.value(format!("{}#insecure", js.text(url)?))?
    } else {
        url
    };
    flow_call(
        js,
        vec![
            url,
            optional(js, &args, "flowUserAgent")?,
            js.undefined(),
            proxy,
            optional(js, &args, "flowUrl")?,
            optional(js, &args, "flowHeaders")?,
        ],
    )
    .await
}

async fn custom_flow<'js>(
    js: &Js<'js>,
    value: Value<'js>,
    proxy: Value<'js>,
) -> rquickjs::Result<Value<'js>> {
    if js.regex_test(js.call("regex", (r"^https?://", ""))?, value.clone())? {
        Ok(logged(
            js,
            flow_call(
                js,
                vec![js.undefined(), js.undefined(), js.undefined(), proxy, value],
            )
            .await,
        ))
    } else {
        Ok(value)
    }
}

fn joined<'js>(
    js: &Js<'js>,
    values: Vec<Value<'js>>,
    separator: &str,
) -> rquickjs::Result<Value<'js>> {
    let scope = js.clone();
    let values = js.filter(js.array(values)?, move |value, _| scope.truthy(&value))?;
    js.method(values, "join", vec![js.value(separator)?])
}

fn flow_headers<'js>(
    js: &Js<'js>,
    headers: &Value<'js>,
    value: Value<'js>,
) -> rquickjs::Result<()> {
    let flow: Value = js.ctx.globals().get("flowUtils")?;
    let value = js.method(flow, "normalizeFlowHeader", vec![value, js.value(true)?])?;
    for key in ["subscription-userinfo", "profile-web-page-url", "plan-name"] {
        let value = optional(js, &value, key)?;
        if js.truthy(&value)? {
            js.set(headers, key, value)?;
        }
    }
    Ok(())
}

async fn flow<'js>(
    js: &Js<'js>,
    item: &Value<'js>,
    options: &Value<'js>,
    headers: &Value<'js>,
    collection: bool,
) -> rquickjs::Result<()> {
    if js.truthy(&js.get(options, "noFlow")?)? {
        return Ok(());
    }
    let proxy = js.get(options, "proxy")?;
    let mut inherited = js.undefined();
    let sub = if collection && !js.equal(&js.get(item, "firstSubFlow")?, &js.value(false)?)? {
        let names = js.get(item, "subscriptions")?;
        if js.truthy(&js.get(&names, "length")?)? {
            Some(find(js, read(js, "subs")?, js.get(&names, "0")?)?)
        } else {
            None
        }
    } else if collection {
        None
    } else {
        Some(item.clone())
    };
    if let Some(sub) = sub
        && (!collection || !js.truthy(&js.get(&sub, "noFlow")?)?)
    {
        let merge = js.get(&sub, "mergeSources")?;
        if !js.equal(&js.get(&sub, "source")?, &js.value("local")?)?
            || js.equal(&merge, &js.value("localFirst")?)?
            || js.equal(&merge, &js.value("remoteFirst")?)?
        {
            let url = if collection {
                js.get(&sub, "url")?
            } else {
                network::prefer(js, [js.get(options, "url")?, js.get(&sub, "url")?])?
            };
            let proxy = if collection {
                network::prefer(
                    js,
                    [
                        proxy.clone(),
                        js.get(&sub, "proxy")?,
                        js.get(item, "proxy")?,
                    ],
                )?
            } else {
                network::prefer(js, [proxy.clone(), js.get(&sub, "proxy")?])?
            };
            inherited = logged(js, remote_flow(js, url, proxy, collection).await);
            if !collection && js.truthy(&inherited)? {
                flow_headers(js, headers, inherited.clone())?;
            }
        }
        let custom = js.get(&sub, "subUserinfo")?;
        if js.truthy(&custom)? {
            let custom = custom_flow(
                js,
                custom,
                network::prefer(js, [proxy.clone(), js.get(&sub, "proxy")?])?,
            )
            .await?;
            inherited = joined(
                js,
                vec![custom, inherited],
                if collection { "; " } else { ";" },
            )?;
            if !collection {
                flow_headers(js, headers, inherited.clone())?;
            }
        }
    }
    if collection {
        let own = custom_flow(
            js,
            js.get(item, "subUserinfo")?,
            network::prefer(js, [proxy, js.get(item, "proxy")?])?,
        )
        .await?;
        let value = joined(js, vec![own, inherited], "; ")?;
        if js.truthy(&value)? {
            flow_headers(js, headers, value)?;
        }
    }
    Ok(())
}

fn set_headers<'js>(
    js: &Js<'js>,
    headers: &Value<'js>,
    changes: Value<'js>,
) -> rquickjs::Result<()> {
    let entries: Vec<Value> =
        js.call("iterable", (js.call::<Value, _>("entries", (changes,))?,))?;
    for pair in entries {
        let key = js.method(js.get(&pair, "0")?, "toLowerCase", vec![])?;
        let key = js.text(key)?;
        let value = js.get(&pair, "1")?;
        if value.is_null() || value.is_undefined() {
            js.call::<(), _>("remove", (headers.clone(), key))?;
        } else {
            js.set(headers, &key, value)?;
        }
    }
    Ok(())
}

fn request_options<'js>(js: &Js<'js>, request: &Value<'js>) -> rquickjs::Result<Value<'js>> {
    let query = js.get(request, "query")?;
    let supplied = object(js)?;
    let metadata = js.call(
        "except",
        (request.clone(), js.array([js.value("responseHeaders")?])?),
    )?;
    js.set(&supplied, "_req", metadata)?;
    let raw = js.get(&query, "$options")?;
    if js.truthy(&raw)? {
        let (_, parsed) = network::fragment(js, js.value(format!("#{}", js.text(raw)?))?)?;
        js.call::<Value, _>("assign", (supplied.clone(), parsed))?;
    }
    Ok(supplied)
}

pub(crate) async fn download<'js>(
    js: &Js<'js>,
    collection: bool,
    item: Value<'js>,
    options: Value<'js>,
    request: Value<'js>,
) -> rquickjs::Result<Value<'js>> {
    let query = js.get(&request, "query")?;
    let supplied = request_options(js, &request)?;
    js.set(&options, "$options", supplied.clone())?;
    let no_flow = network::prefer(js, [js.get(&query, "noFlow")?, js.get(&item, "noFlow")?])?;
    js.set(&options, "noFlow", no_flow.clone())?;
    if collection || js.truthy(&js.get(&item, "passThroughUA")?)? {
        js.set(
            &options,
            "ua",
            optional(js, &js.get(&request, "headers")?, "user-agent")?,
        )?;
    }
    let output = pipeline::generate(
        js,
        if collection { "collection" } else { "sub" },
        item.clone(),
        options.clone(),
    )
    .await?;
    let headers = js.get(&request, "responseHeaders")?;
    flow(js, &item, &options, &headers, collection).await?;
    let target = js.get(&options, "platform")?;
    js.set(
        &headers,
        "content-type",
        js.value(if js.equal(&target, &js.value("JSON")?)? {
            "application/json;charset=utf-8"
        } else {
            "text/plain; charset=utf-8"
        })?,
    )?;
    let input = object(js)?;
    js.set(&input, "headers", headers)?;
    js.set(&input, "body", output)?;
    js.set(
        &input,
        "agePublicKey",
        optional(js, &request, "outputAgePublicKey")?,
    )?;
    let source = object(js)?;
    js.set(
        &source,
        if collection {
            "_collection".to_owned()
        } else {
            js.text(js.get(&item, "name")?)?
        }
        .as_str(),
        item.clone(),
    )?;
    let execution = object(js)?;
    js.set(&execution, "noFlow", no_flow)?;
    response(js, input, item, target, source, supplied, execution).await
}

async fn response<'js>(
    js: &Js<'js>,
    input: Value<'js>,
    item: Value<'js>,
    target: Value<'js>,
    source: Value<'js>,
    supplied: Value<'js>,
    execution: Value<'js>,
) -> rquickjs::Result<Value<'js>> {
    let share_key = optional(js, &input, "agePublicKey")?;
    let headers = js.get(&input, "headers")?;
    let res = optional(js, &supplied, "_res")?;
    let override_headers = optional(js, &res, "headers")?;
    if js.truthy(&override_headers)? {
        set_headers(js, &headers, override_headers)?;
    }
    js.set(
        &input,
        "status",
        network::prefer(js, [optional(js, &res, "status")?, js.value(200)?])?,
    )?;
    let current: Value = js.call("spread", (object(js)?, headers.clone()))?;
    js.set(&input, "headers", current.clone())?;
    let transformed = utility(
        js,
        "processResponse",
        vec![
            input,
            network::prefer(js, [js.get(&item, "process")?, js.array([])?])?,
            target,
            source,
            supplied,
            execution,
        ],
    )
    .await?;
    let next = js.get(&transformed, "headers")?;
    let entries: Vec<Value> =
        js.call("iterable", (js.call::<Value, _>("entries", (current,))?,))?;
    for pair in entries {
        let key = js.get(&pair, "0")?;
        if !js.call::<bool, _>("own", (next.clone(), key.clone()))? {
            js.call::<(), _>("remove", (headers.clone(), key))?;
        }
    }
    set_headers(js, &headers, next)?;
    js.set(&transformed, "headers", headers.clone())?;
    let key = if share_key.is_null() || share_key.is_undefined() {
        optional(js, &item, "age-public-key")?
    } else {
        share_key
    };
    let key = js.method(
        js.value(if key.is_null() || key.is_undefined() {
            String::new()
        } else {
            js.text(key)?
        })?,
        "trim",
        vec![],
    )?;
    if js.truthy(&key)? {
        let utils: Value = js.ctx.globals().get("ProxyUtils")?;
        let pending = js.method(
            js.get(&utils, "age")?,
            "encrypt",
            vec![js.get(&transformed, "body")?, key],
        )?;
        js.set(&transformed, "body", script::settled(js, pending).await?)?;
        js.set(
            &headers,
            "content-type",
            js.value("text/plain; charset=utf-8")?,
        )?;
    }
    let status: f64 = js.call("number", (js.get(&transformed, "status")?,))?;
    js.set(
        &transformed,
        "status",
        js.value(
            if status.is_finite() && status.fract() == 0.0 && (100.0..=599.0).contains(&status) {
                status
            } else {
                200.0
            },
        )?,
    )?;
    let buffer: Value = js.ctx.globals().get("Buffer")?;
    let body = js.get(&transformed, "body")?;
    if js.truthy(&js.method(buffer, "isBuffer", vec![body.clone()])?)? {
        js.set(
            &transformed,
            "body",
            js.method(body, "toString", vec![js.value("base64")?])?,
        )?;
        js.set(&transformed, "binary", js.value(true)?)?;
    }
    Ok(transformed)
}

pub(crate) async fn file<'js>(
    js: &Js<'js>,
    item: Value<'js>,
    request: Value<'js>,
) -> rquickjs::Result<Value<'js>> {
    let query = js.get(&request, "query")?;
    let supplied = request_options(js, &request)?;
    let options = object(js)?;
    for key in [
        "url",
        "ua",
        "content",
        "mergeSources",
        "ignoreFailedRemoteFile",
        "proxy",
        "noCache",
        "produceType",
    ] {
        js.set(&options, key, js.get(&query, key)?)?;
    }
    js.set(&options, "$options", supplied.clone())?;
    js.set(&options, "all", js.value(true)?)?;
    let output = pipeline::file(js, item.clone(), options, false).await?;
    let headers = js.get(&request, "responseHeaders")?;
    if js.truthy(&js.get(&item, "subInfoUrl")?)? {
        let flow = flow_call(
            js,
            vec![
                js.get(&item, "subInfoUrl")?,
                js.get(&item, "subInfoUserAgent")?,
                js.undefined(),
                js.get(&item, "proxy")?,
            ],
        )
        .await;
        let value = logged(js, flow);
        if js.truthy(&value)?
            && let Err(error) = flow_headers(js, &headers, value)
        {
            logged(js, Err(error));
        }
    }
    if js.truthy(&js.get(&item, "download")?)? {
        let name = network::prefer(js, [js.get(&item, "displayName")?, js.get(&item, "name")?])?;
        let encode: rquickjs::Function = js.ctx.globals().get("encodeURIComponent")?;
        let name: String = encode.call((name,))?;
        js.set(
            &headers,
            "content-disposition",
            js.value(format!("attachment; filename*=UTF-8''{name}"))?,
        )?;
    }
    js.set(
        &headers,
        "content-type",
        js.value("text/plain; charset=utf-8")?,
    )?;
    let input = object(js)?;
    js.set(&input, "headers", headers)?;
    let body = optional(js, &output, "$content")?;
    js.set(
        &input,
        "agePublicKey",
        optional(js, &request, "outputAgePublicKey")?,
    )?;
    js.set(
        &input,
        "body",
        if body.is_null() || body.is_undefined() {
            js.value("")?
        } else {
            body
        },
    )?;
    let source = object(js)?;
    js.set(&source, "$file", item.clone())?;
    let supplied = optional(js, &output, "$options")?;
    let supplied = if supplied.is_null() || supplied.is_undefined() {
        request_options(js, &request)?
    } else {
        supplied
    };
    response(
        js,
        input,
        item,
        js.undefined(),
        source,
        supplied,
        object(js)?,
    )
    .await
}

pub(crate) async fn parse<'js>(js: &Js<'js>, fields: Value<'js>) -> rquickjs::Result<Value<'js>> {
    let download = js.get(&fields, "download")?;
    let raw = if js.truthy(&download)? {
        utility(js, "download", js.call("iterable", (download,))?).await?
    } else {
        let data = js.get(&fields, "data")?;
        if data.is_null() || data.is_undefined() {
            js.get(&fields, "content")?
        } else {
            data
        }
    };
    let parsed = utility(js, "parse", vec![raw]).await?;
    let client = js.get(&fields, "client")?;
    let client = if client.is_null() || client.is_undefined() {
        js.get(&fields, "platform")?
    } else {
        client
    };
    utility(
        js,
        "produce",
        vec![
            parsed,
            client,
            js.get(&fields, "type")?,
            js.get(&fields, "produce")?,
        ],
    )
    .await
}
