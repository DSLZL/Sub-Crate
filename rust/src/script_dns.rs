//! Native resolver dispatch and ordered domain processing over live script values.
use crate::{
    dns_transport,
    js_language::Js,
    script, script_helpers, script_network as network,
    script_pipeline::{object, optional},
};
use rquickjs::{
    Ctx, Function, Object, Value,
    prelude::{Async, Func, Rest},
};
use std::{
    cell::{Cell, RefCell},
    collections::{HashMap, HashSet},
    rc::Rc,
};

fn arg<'js>(js: &Js<'js>, args: &[Value<'js>], index: usize) -> Value<'js> {
    args.get(index).cloned().unwrap_or_else(|| js.undefined())
}

fn text<'js>(js: &Js<'js>, value: Value<'js>) -> rquickjs::Result<String> {
    js.text(network::prefer(js, [value, js.value("")?])?)
}

fn blank<'js>(js: &Js<'js>, value: &Value<'js>) -> rquickjs::Result<bool> {
    Ok(value.is_null()
        || value.is_undefined()
        || (value.is_string() && js.text(value.clone())?.trim().is_empty()))
}

fn positive<'js>(
    js: &Js<'js>,
    value: Value<'js>,
    default: f64,
    message: &str,
) -> rquickjs::Result<f64> {
    if blank(js, &value)? {
        return Ok(default);
    }
    let number: f64 = js.call("number", (value,))?;
    if number.is_finite() && number.fract() == 0.0 && number >= 1.0 {
        Ok(number)
    } else {
        Err(js.error(message))
    }
}

fn ips<'js>(js: &Js<'js>, kind: &str, value: Value<'js>) -> rquickjs::Result<bool> {
    let utils = js.ctx.globals().get("ProxyUtils")?;
    js.truthy(&js.method(utils, kind, vec![value])?)
}

fn urls<'js>(js: &Js<'js>, value: Value<'js>) -> rquickjs::Result<Vec<String>> {
    let raw = text(js, value)?;
    let urls: Vec<String> = raw
        .split('\n')
        .map(str::trim)
        .filter(|url| !url.is_empty())
        .map(str::to_owned)
        .collect();
    if urls.is_empty() {
        Err(js.error("自定义 DNS 不能为空"))
    } else {
        Ok(urls)
    }
}

fn resolver<'js>(js: &Js<'js>, value: Value<'js>) -> rquickjs::Result<Value<'js>> {
    let raw = text(js, value)?.trim().to_owned();
    if raw.is_empty() {
        return Err(js.error("自定义 DNS 不能为空"));
    }
    let lower = raw.to_ascii_lowercase();
    if lower.starts_with("https://") || lower.starts_with("http://") {
        let result = object(js)?;
        js.set(&result, "protocol", js.value("doh")?)?;
        js.set(&result, "url", js.value(raw)?)?;
        return Ok(result);
    }
    let expression = js.call("regex", (r"^([a-z][a-z\d+.-]*):\/\/", "i"))?;
    let matched = js.method(js.value(raw.clone())?, "match", vec![expression])?;
    let protocol = if matched.is_null() {
        String::new()
    } else {
        js.text(js.get(&matched, "1")?)?
    };
    let normalized = protocol.to_ascii_lowercase();
    if !normalized.is_empty() && !matches!(normalized.as_str(), "udp" | "tcp" | "tls") {
        return Err(js.error(&format!("自定义 DNS 不支持 {protocol} 协议")));
    }
    let value = if normalized.is_empty() {
        if ips(js, "isIPv6", js.value(raw.clone())?)? {
            format!("udp://[{raw}]")
        } else {
            format!("udp://{raw}")
        }
    } else {
        raw.clone()
    };
    let parsed = dns_transport::parse_url(js.ctx.clone(), value);
    let parsed = match parsed {
        Ok(parsed) => js.ctx.json_parse(parsed)?,
        Err(_) => {
            js.ctx.catch();
            return Err(js.error(&format!("自定义 DNS 地址格式无效: {raw}")));
        }
    };
    for field in ["username", "password", "search", "hash", "pathname"] {
        let value = js.text(js.get(&parsed, field)?)?;
        if !value.is_empty() && !(field == "pathname" && value == "/") {
            return Err(js.error(&format!("自定义 DNS 地址格式无效: {raw}")));
        }
    }
    let host = js.text(js.get(&parsed, "hostname")?)?;
    let host = host
        .strip_prefix('[')
        .and_then(|s| s.strip_suffix(']'))
        .unwrap_or(&host);
    if host.is_empty() {
        return Err(js.error(&format!("自定义 DNS 地址格式无效: {raw}")));
    }
    let port = js.get(&parsed, "port")?;
    let port: f64 = if js.truthy(&port)? {
        js.call("number", (port,))?
    } else if normalized == "tls" {
        853.0
    } else {
        53.0
    };
    if !(1.0..=65535.0).contains(&port) || port.fract() != 0.0 {
        return Err(js.error("自定义 DNS 端口号应为 1-65535 的整数"));
    }
    let result = object(js)?;
    js.set(
        &result,
        "protocol",
        js.value(if normalized.is_empty() {
            "udp"
        } else {
            &normalized
        })?,
    )?;
    js.set(&result, "host", js.value(host)?)?;
    js.set(&result, "port", js.value(port)?)?;
    Ok(result)
}

fn packet<'js>(js: &Js<'js>) -> rquickjs::Result<Value<'js>> {
    let module: Value = js.ctx.globals().get("SubStoreDnsUtilities")?;
    js.get(&module, "dnsPacket")
}

fn buffer<'js>(js: &Js<'js>, value: Value<'js>) -> rquickjs::Result<Value<'js>> {
    let buffer = js.ctx.globals().get("Buffer")?;
    js.method(buffer, "from", vec![value])
}

fn query_packet<'js>(js: &Js<'js>, options: &Value<'js>) -> rquickjs::Result<Value<'js>> {
    let query = object(js)?;
    js.set(&query, "type", js.value("query")?)?;
    js.set(&query, "id", js.value(0)?)?;
    js.set(&query, "flags", js.get(&packet(js)?, "RECURSION_DESIRED")?)?;
    let question = object(js)?;
    let kind = js.get(options, "type")?;
    js.set(
        &question,
        "type",
        if kind.is_undefined() {
            js.value("A")?
        } else {
            kind
        },
    )?;
    js.set(&question, "name", js.get(options, "domain")?)?;
    js.set(&query, "questions", js.array([question])?)?;
    let additional = js.array([])?;
    let edns = js.get(options, "edns")?;
    if js.truthy(&edns)? {
        let subnet = object(js)?;
        js.set(&subnet, "code", js.value("CLIENT_SUBNET")?)?;
        js.set(&subnet, "ip", edns.clone())?;
        js.set(
            &subnet,
            "sourcePrefixLength",
            js.value(if ips(js, "isIPv4", edns)? { 24 } else { 56 })?,
        )?;
        js.set(&subnet, "scopePrefixLength", js.value(0)?)?;
        let entry = object(js)?;
        for (key, value) in [
            ("type", js.value("OPT")?),
            ("name", js.value(".")?),
            ("udpPayloadSize", js.value(4096)?),
            ("flags", js.value(0)?),
            ("options", js.array([subnet])?),
        ] {
            js.set(&entry, key, value)?;
        }
        js.method(additional.clone(), "push", vec![entry])?;
    }
    js.set(&query, "additionals", additional)?;
    js.method(packet(js)?, "encode", vec![query])
}

async fn http<'js>(js: &Js<'js>, options: Value<'js>) -> rquickjs::Result<Value<'js>> {
    script::settled(
        js,
        js.method(js.get(&network::host(js)?, "http")?, "get", vec![options])?,
    )
    .await
}

async fn doh<'js>(js: &Js<'js>, options: Value<'js>) -> rquickjs::Result<Value<'js>> {
    let payload = js
        .text(js.method(
            query_packet(js, &options)?,
            "toString",
            vec![js.value("base64")?],
        )?)?
        .replace('+', "-")
        .replace('/', "_")
        .trim_end_matches('=')
        .to_owned();
    let request = object(js)?;
    js.set(
        &request,
        "url",
        js.value(format!(
            "{}?dns={payload}",
            js.text(js.get(&options, "url")?)?
        ))?,
    )?;
    js.set(&request, "timeout", js.get(&options, "timeout")?)?;
    js.set(&request, "encoding", Value::new_null(js.ctx.clone()))?;
    js.set(&request, "binary-mode", js.value(true)?)?;
    js.set(
        &request,
        "insecure",
        js.value(js.equal(&js.get(&options, "skipCertVerify")?, &js.value(true)?)?)?,
    )?;
    let headers = object(js)?;
    js.set(&headers, "Accept", js.value("application/dns-message")?)?;
    js.set(&request, "headers", headers)?;
    let response = http(js, request).await?;
    js.method(
        packet(js)?,
        "decode",
        vec![buffer(js, js.get(&response, "body")?)?],
    )
}

async fn resolve_dns<'js>(js: &Js<'js>, options: Value<'js>) -> rquickjs::Result<Value<'js>> {
    let server = resolver(js, js.get(&options, "url")?)?;
    if js.equal(&js.get(&server, "protocol")?, &js.value("doh")?)? {
        return doh(js, options).await;
    }
    let timeout = network::prefer(js, [js.get(&options, "timeout")?, js.value(8000)?])?;
    let timeout: f64 = js.call("number", (timeout,))?;
    let bytes: Value = js.call("iterable", (query_packet(js, &options)?,))?;
    let raw = dns_transport::query(
        js.ctx.clone(),
        script::serialized(&js.ctx, server)?,
        script::serialized(&js.ctx, bytes)?,
        timeout,
        js.equal(&js.get(&options, "skipCertVerify")?, &js.value(true)?)?,
    )
    .await?;
    js.method(
        packet(js)?,
        "decode",
        vec![buffer(js, js.ctx.json_parse(raw)?)?],
    )
}

fn cache<'js>(js: &Js<'js>) -> rquickjs::Result<Value<'js>> {
    js.ctx.globals().get("__resourceCache")
}

fn cache_id<'js>(
    js: &Js<'js>,
    provider: &str,
    domain: Value<'js>,
    kind: Value<'js>,
    url: Value<'js>,
    insecure: bool,
) -> rquickjs::Result<String> {
    let domain = js.text(domain)?;
    let kind = js.text(kind)?;
    let prefix = match provider {
        "Custom" => format!(
            "CUSTOM:{}{}:{domain}:{kind}",
            if insecure { "INSECURE:" } else { "" },
            js.text(url)?
        ),
        "IP-API" => format!("IP-API:{domain}"),
        _ => format!("{}:{domain}:{kind}", provider.to_uppercase()),
    };
    script_helpers::md5(
        js.ctx.clone(),
        rquickjs::prelude::Opt(Some(js.value(prefix)?)),
    )
}

fn annotate<'js>(js: &Js<'js>, value: Value<'js>, url: Value<'js>) -> rquickjs::Result<Value<'js>> {
    if !value.is_null() && js.call::<String, _>("typeOf", (value.clone(),))? == "object" {
        let descriptor = object(js)?;
        js.set(&descriptor, "value", url)?;
        js.set(&descriptor, "enumerable", js.value(false)?)?;
        js.set(&descriptor, "configurable", js.value(true)?)?;
        let object = js.ctx.globals().get("Object")?;
        js.method(
            object,
            "defineProperty",
            vec![value.clone(), js.value("_resolverUrl")?, descriptor],
        )?;
    }
    Ok(value)
}

fn unpack<'js>(js: &Js<'js>, value: Value<'js>) -> rquickjs::Result<Value<'js>> {
    if !value.is_array() && optional(js, &value, "result")?.is_array() {
        annotate(
            js,
            js.get(&value, "result")?,
            js.get(&value, "resolverUrl")?,
        )
    } else {
        Ok(value)
    }
}

fn answers<'js>(
    js: &Js<'js>,
    response: Value<'js>,
    kind: Value<'js>,
) -> rquickjs::Result<Value<'js>> {
    let values = js.get(&response, "answers")?;
    if !values.is_array() {
        return Err(js.error("No answers"));
    }
    let scope = js.clone();
    let selected = js.filter(values, move |value, _| {
        scope.equal(&optional(&scope, &value, "type")?, &kind)
    })?;
    let scope = js.clone();
    let selected = js.map(selected, move |value| optional(&scope, &value, "data"))?;
    let scope = js.clone();
    let selected = js.filter(selected, move |value, _| scope.truthy(&value))?;
    if !js.truthy(&js.get(&selected, "length")?)? {
        return Err(js.error("No answers"));
    }
    Ok(selected)
}

async fn custom_one<'js>(
    js: &Js<'js>,
    options: Value<'js>,
    url: String,
) -> rquickjs::Result<Value<'js>> {
    let options: Value = js.call("spread", (object(js)?, options))?;
    js.set(&options, "url", js.value(url)?)?;
    let kind = js.get(&options, "type")?;
    answers(js, resolve_dns(js, options).await?, kind)
}

async fn custom_pool<'js>(
    js: &Js<'js>,
    options: Value<'js>,
    urls: Vec<String>,
    concurrency: f64,
) -> rquickjs::Result<(Value<'js>, String)> {
    if urls.len() == 1 {
        return Ok((
            custom_one(js, options, urls[0].clone()).await?,
            urls[0].clone(),
        ));
    }
    let mut pending = Vec::<(usize, Value<'js>)>::new();
    let mut next = 0;
    let mut errors = Vec::new();
    loop {
        while next < urls.len() && (pending.len() as f64) < concurrency {
            let index = next;
            let scope = js.clone();
            let request = options.clone();
            let url = urls[index].clone();
            let send = Function::new(
                js.ctx.clone(),
                Async(move || {
                    let js = scope.clone();
                    let request = request.clone();
                    let url = url.clone();
                    async move { custom_one(&js, request, url).await }
                }),
            )?;
            let promise: Value = send.call(())?;
            let scope = js.clone();
            let ok = Function::new(js.ctx.clone(), move |value: Value<'js>| {
                let output = object(&scope)?;
                scope.set(&output, "index", scope.value(index)?)?;
                scope.set(&output, "result", value)?;
                Ok::<_, rquickjs::Error>(output)
            })?;
            let scope = js.clone();
            let error = Function::new(js.ctx.clone(), move |value: Value<'js>| {
                let output = object(&scope)?;
                scope.set(&output, "index", scope.value(index)?)?;
                scope.set(&output, "error", value)?;
                Ok::<_, rquickjs::Error>(output)
            })?;
            pending.push((
                index,
                js.method(promise, "then", vec![ok.into_value(), error.into_value()])?,
            ));
            next += 1;
        }
        let constructor = js.ctx.globals().get("Promise")?;
        let selected = script::settled(
            js,
            js.method(
                constructor,
                "race",
                vec![js.array(pending.iter().map(|(_, value)| value.clone()))?],
            )?,
        )
        .await?;
        let index: usize = js.call("number", (js.get(&selected, "index")?,))?;
        if js.call::<bool, _>("own", (selected.clone(), "result"))? {
            return Ok((js.get(&selected, "result")?, urls[index].clone()));
        }
        errors.push(format!(
            "{}: {}",
            urls[index],
            js.text(js.get(&selected, "error")?)?
        ));
        pending.retain(|(candidate, _)| *candidate != index);
        if pending.is_empty() && next == urls.len() {
            return Err(js.error(&errors.join("; ")));
        }
    }
}

async fn provider<'js>(
    js: &Js<'js>,
    name: &str,
    args: Vec<Value<'js>>,
) -> rquickjs::Result<Value<'js>> {
    let domain = arg(js, &args, 0);
    let kind = arg(js, &args, 1);
    let no_cache = arg(js, &args, 2);
    let timeout = arg(js, &args, 3);
    let edns = arg(js, &args, 4);
    let mut url = arg(js, &args, 5);
    let insecure = arg(js, &args, 6);
    let ttl = arg(js, &args, 8);
    if name == "IP-API" && js.equal(&kind, &js.value("IPv6")?)? {
        return Err(js.error("域名解析服务提供方 IP-API 不支持 IPv6"));
    }
    let custom_urls = if name == "Custom" {
        let values = urls(js, url)?;
        url = js.value(values.join("\n"))?;
        values
    } else {
        Vec::new()
    };
    let concurrency = if name == "Custom" {
        positive(js, arg(js, &args, 7), 2.0, "多 DNS 并发数应为大于 0 的整数")?
    } else {
        1.0
    };
    let id = cache_id(
        js,
        name,
        domain.clone(),
        kind.clone(),
        url.clone(),
        js.truthy(&insecure)?,
    )?;
    let cached = js.method(cache(js)?, "get", vec![js.value(id.clone())?])?;
    let cached = if name == "Custom" {
        unpack(js, cached)?
    } else {
        cached
    };
    if !js.truthy(&no_cache)? && js.truthy(&cached)? {
        return Ok(cached);
    }
    let answer_type = js.value(if js.equal(&kind, &js.value("IPv6")?)? {
        "AAAA"
    } else {
        "A"
    })?;
    let options = object(js)?;
    for (key, value) in [
        ("domain", domain.clone()),
        ("type", answer_type.clone()),
        ("timeout", timeout.clone()),
        ("edns", edns.clone()),
        (
            "skipCertVerify",
            if name == "Custom" {
                insecure
            } else {
                js.undefined()
            },
        ),
    ] {
        js.set(&options, key, value)?;
    }
    let mut resolver_url = None;
    let result = if name == "Custom" {
        let (result, winner) = custom_pool(js, options, custom_urls, concurrency).await?;
        resolver_url = Some(winner);
        result
    } else if matches!(name, "Google" | "Cloudflare") {
        js.set(
            &options,
            "url",
            js.value(if name == "Google" {
                "https://8.8.4.4/dns-query"
            } else {
                "https://1.0.0.1/dns-query"
            })?,
        )?;
        answers(js, resolve_dns(js, options).await?, answer_type.clone())?
    } else {
        let encode: Function = js.ctx.globals().get("encodeURIComponent")?;
        let encoded: String = encode.call((domain,))?;
        let address = match name {
            "IP-API" => format!("http://ip-api.com/json/{encoded}?lang=zh-CN"),
            "Ali" => format!(
                "http://223.6.6.6/resolve?edns_client_subnet={}/{}&name={encoded}&type={}&short=1",
                js.text(edns.clone())?,
                if ips(js, "isIPv4", edns.clone())? {
                    24
                } else {
                    56
                },
                js.text(answer_type.clone())?
            ),
            "Tencent" => format!(
                "http://119.28.28.28/d?ip={}&type={}&dn={encoded}",
                js.text(edns)?,
                js.text(answer_type)?
            ),
            _ => return Err(js.error(&format!("找不到域名解析服务提供方: {name}"))),
        };
        let request = object(js)?;
        js.set(&request, "url", js.value(address)?)?;
        js.set(&request, "timeout", timeout)?;
        if name != "IP-API" {
            let headers = object(js)?;
            js.set(&headers, "accept", js.value("application/dns-json")?)?;
            js.set(&request, "headers", headers)?;
        }
        let response = http(js, request).await?;
        let body = js.get(&response, "body")?;
        if name == "IP-API" {
            let body = js.call("jsonParse", (body,))?;
            let status = js.get(&body, "status")?;
            if !js.equal(&status, &js.value("success")?)? {
                return Err(js.error(&format!("Status is {}", js.text(status)?)));
            }
            let query = js.get(&body, "query")?;
            if !js.truthy(&query)? {
                return Err(js.error("No answers"));
            }
            js.array([query])?
        } else if name == "Ali" {
            js.call("jsonParse", (body,))?
        } else {
            let values = js.method(body, "split", vec![js.value(";")?])?;
            let scope = js.clone();
            js.map(values, move |value| {
                scope.get(&scope.method(value, "split", vec![scope.value(",")?])?, "0")
            })?
        }
    };
    if !result.is_array()
        || !js.truthy(&js.get(&result, "length")?)?
        || (name == "Tencent" && js.text(result.clone())? == "0")
    {
        return Err(js.error("No answers"));
    }
    let saved = if let Some(winner) = &resolver_url {
        let value = object(js)?;
        js.set(&value, "result", result.clone())?;
        js.set(&value, "resolverUrl", js.value(winner.as_str())?)?;
        value
    } else {
        result.clone()
    };
    js.method(cache(js)?, "set", vec![js.value(id)?, saved, ttl])?;
    if let Some(winner) = resolver_url {
        annotate(js, result, js.value(winner)?)
    } else {
        Ok(result)
    }
}

pub(crate) async fn apply<'js>(
    js: &Js<'js>,
    input: Value<'js>,
    options: Value<'js>,
) -> rquickjs::Result<Value<'js>> {
    let provider_value = js.get(&options, "provider")?;
    let name = js.text(provider_value.clone())?;
    let requested = js.get(&options, "type")?;
    if name == "IP-API"
        && (js.equal(&requested, &js.value("IPv6")?)?
            || js.equal(&requested, &js.value("IP4P")?)?)
    {
        return Err(js.error(&format!(
            "域名解析服务提供方 {name} 不支持 {}",
            js.text(requested)?
        )));
    }
    let default_timeout: f64 = js.call(
        "number",
        (optional(js, &network::settings(js)?, "defaultTimeout")?,),
    )?;
    let timeout = positive(
        js,
        js.get(&options, "timeout")?,
        if default_timeout.is_finite() && default_timeout > 0.0 {
            default_timeout
        } else {
            8000.0
        },
        "DNS 超时应为大于 0 的整数",
    )?;
    let cache_ttl = js.get(&options, "cacheTtl")?;
    let ttl = if blank(js, &cache_ttl)? {
        js.undefined()
    } else {
        js.value(positive(js, cache_ttl, 0.0, "DNS 缓存时间应为大于 0 的整数")? * 1000.0)?
    };
    let kind = js.value(
        if js.equal(&requested, &js.value("IPv6")?)? || js.equal(&requested, &js.value("IP4P")?)? {
            "IPv6"
        } else {
            "IPv4"
        },
    )?;
    let resolvers: Value = js.ctx.globals().get("DOMAIN_RESOLVERS")?;
    if !js.truthy(&js.get(&resolvers, &name)?)? {
        return Err(js.error(&format!("找不到域名解析服务提供方: {name}")));
    }
    let edns = network::prefer(js, [js.get(&options, "edns")?, js.value("223.6.6.6")?])?;
    if !ips(js, "isIP", edns.clone())? {
        return Err(js.error("域名解析 EDNS 应为 IP"));
    }
    let concurrency = positive(
        js,
        js.get(&options, "concurrency")?,
        10.0,
        "域名解析并发数应为大于 0 的整数",
    )?;
    let raw_url = js.get(&options, "url")?;
    let url = if name == "Custom" {
        js.value(urls(js, raw_url)?.join("\n"))?
    } else {
        raw_url
    };
    let dns_concurrency = if name == "Custom" {
        positive(
            js,
            js.get(&options, "dnsConcurrency")?,
            2.0,
            "多 DNS 并发数应为大于 0 的整数",
        )?
    } else {
        1.0
    };
    let insecure_value = js.get(&options, "tlsSkipCertVerify")?;
    let insecure = name == "Custom"
        && (js.equal(&insecure_value, &js.value(true)?)?
            || js.equal(&insecure_value, &js.value("enabled")?)?);
    let no_cache = js.equal(&js.get(&options, "cache")?, &js.value("disabled")?)?;
    let input = js.json_clone(input)?;
    let proxies: Vec<Value> = js.call("iterable", (input.clone(),))?;
    let mut seen = HashSet::new();
    let mut domains = Vec::new();
    let mut found = HashMap::new();
    for proxy in &proxies {
        if !js.truthy(&js.get(proxy, "_no-resolve")?)?
            && js.truthy(&js.get(proxy, "no-resolve")?)?
        {
            js.set(proxy, "_no-resolve", js.get(proxy, "no-resolve")?)?;
        }
        let domain = js.get(proxy, "server")?;
        if ips(js, "isIP", domain.clone())? || js.truthy(&js.get(proxy, "_no-resolve")?)? {
            continue;
        }
        let id = cache_id(
            js,
            &name,
            domain.clone(),
            kind.clone(),
            url.clone(),
            insecure,
        )?;
        if !seen.insert(id.clone()) {
            continue;
        }
        let cached = if no_cache {
            js.undefined()
        } else {
            js.method(cache(js)?, "get", vec![js.value(id.clone())?])?
        };
        let cached = if name == "Custom" {
            unpack(js, cached)?
        } else {
            cached
        };
        if js.truthy(&cached)? {
            found.insert(id, cached);
        } else {
            domains.push((id, domain));
        }
    }
    let results = Rc::new(RefCell::new(found));
    let domains = Rc::new(domains);
    let next = Rc::new(Cell::new(0));
    let mut workers = Vec::new();
    let base_args = vec![
        kind.clone(),
        js.value(no_cache)?,
        js.value(timeout)?,
        edns,
        url.clone(),
        js.value(insecure)?,
        js.value(dns_concurrency)?,
        ttl,
    ];
    for _ in 0..domains.len().min(concurrency as usize) {
        let scope = js.clone();
        let domains = domains.clone();
        let next = next.clone();
        let results = results.clone();
        let args = base_args.clone();
        let name = name.clone();
        let resolvers = resolvers.clone();
        let worker = Function::new(
            js.ctx.clone(),
            Async(move || {
                let js = scope.clone();
                let domains = domains.clone();
                let next = next.clone();
                let results = results.clone();
                let args = args.clone();
                let name = name.clone();
                let resolvers = resolvers.clone();
                async move {
                    while next.get() < domains.len() {
                        let index = next.get();
                        next.set(index + 1);
                        let (id, domain) = &domains[index];
                        let mut parameters = vec![domain.clone()];
                        parameters.extend(args.clone());
                        let result = match js.method(resolvers.clone(), &name, parameters) {
                            Ok(pending) => script::settled(&js, pending).await,
                            Err(error) => Err(error),
                        };
                        match result {
                            Ok(value) => {
                                results.borrow_mut().insert(id.clone(), value);
                            }
                            Err(error) => eprintln!(
                                "[sub-store:error] Failed to resolve domain: {}: {}",
                                js.text(domain.clone())?,
                                script::message(&js.ctx, error)
                            ),
                        }
                    }
                    Ok::<_, rquickjs::Error>(())
                }
            }),
        )?;
        workers.push(worker.call::<_, Value>(())?);
    }
    let promise = js.ctx.globals().get("Promise")?;
    script::settled(js, js.method(promise, "all", vec![js.array(workers)?])?).await?;
    for proxy in &proxies {
        if js.truthy(&js.get(proxy, "_no-resolve")?)? {
            continue;
        }
        let domain = js.get(proxy, "server")?;
        let id = cache_id(
            js,
            &name,
            domain.clone(),
            kind.clone(),
            url.clone(),
            insecure,
        )?;
        let result = results.borrow().get(&id).cloned();
        if let Some(result) = result.filter(|value| js.truthy(value).unwrap_or(false)) {
            js.set(proxy, "_resolved_ips", result.clone())?;
            let mut ip = if result.is_array() {
                let length: f64 = js.call("number", (js.get(&result, "length")?,))?;
                let draw: f64 = js.call("random", ())?;
                js.call("get", (result, (draw * length).floor()))?
            } else {
                result
            };
            if js.equal(&kind, &js.value("IPv6")?)? && ips(js, "isIPv6", ip.clone())? {
                let ip_address: Value = js.ctx.globals().get("ipAddress")?;
                let constructor = js.get(&ip_address, "Address6")?;
                let address = js.call("construct", (constructor, js.array([ip.clone()])?))?;
                ip = js.method(address, "correctForm", vec![])?;
                if js.regex_test(
                    js.call("regex", (r"^2001::[^:]+:[^:]+:[^:]+$", ""))?,
                    ip.clone(),
                )? {
                    js.set(proxy, "_IP4P", ip.clone())?;
                    let parts = js.method(ip, "split", vec![js.value(":")?])?;
                    let parse: Function = js.ctx.globals().get("parseInt")?;
                    let port: f64 = parse.call((js.get(&parts, "2")?, 16))?;
                    let ab: f64 = parse.call((js.get(&parts, "3")?, 16))?;
                    let cd: f64 = parse.call((js.get(&parts, "4")?, 16))?;
                    let ip4 = js.value(format!(
                        "{}.{}.{}.{}",
                        (ab as i32) >> 8,
                        (ab as i32) & 255,
                        (cd as i32) >> 8,
                        (cd as i32) & 255
                    ))?;
                    if js.truthy(&ip4)? && port != 0.0 && !port.is_nan() {
                        js.set(proxy, "_domain", domain)?;
                        js.set(proxy, "server", ip4.clone())?;
                        js.set(proxy, "port", js.value(port)?)?;
                        js.set(proxy, "resolved", js.value(true)?)?;
                        js.set(proxy, "_IPv4", ip4.clone())?;
                        if !ips(js, "isIP", js.get(proxy, "_IP")?)? {
                            js.set(proxy, "_IP", ip4)?;
                        }
                    } else if !js.truthy(&js.get(proxy, "resolved")?)? {
                        js.set(proxy, "resolved", js.value(false)?)?;
                    }
                    continue;
                }
            }
            js.set(proxy, "_domain", domain)?;
            js.set(proxy, "server", ip.clone())?;
            js.set(proxy, "resolved", js.value(true)?)?;
            js.set(proxy, &format!("_{}", js.text(kind.clone())?), ip.clone())?;
            if !ips(js, "isIP", js.get(proxy, "_IP")?)? {
                js.set(proxy, "_IP", ip)?;
            }
        } else if !js.truthy(&js.get(proxy, "resolved")?)? {
            js.set(proxy, "resolved", js.value(false)?)?;
        }
    }
    let filter = js.text(js.get(&options, "filter")?)?;
    let scope = js.clone();
    js.filter(input, move |proxy, _| {
        let ip = scope.get(&proxy, "server")?;
        match filter.as_str() {
            "removeFailed" => Ok(ips(&scope, "isIP", ip)?
                || scope.truthy(&scope.get(&proxy, "_no-resolve")?)?
                || scope.truthy(&scope.get(&proxy, "resolved")?)?),
            "IPOnly" => ips(&scope, "isIP", ip),
            "IPv4Only" => ips(&scope, "isIPv4", ip),
            "IPv6Only" => ips(&scope, "isIPv6", ip),
            _ => Ok(true),
        }
    })
}

pub(crate) fn initialize<'js>(ctx: &Ctx<'js>, utils: &Object<'js>) -> rquickjs::Result<()> {
    crate::script_bytecode::DNS.eval(ctx)?;
    utils.set(
        "doh",
        Func::from(Async(|ctx: Ctx<'js>, options: Value<'js>| async move {
            doh(&Js::new(ctx), options).await
        })),
    )?;
    let providers = Object::new(ctx.clone())?;
    for name in ["Custom", "Google", "IP-API", "Cloudflare", "Ali", "Tencent"] {
        providers.set(
            name,
            Func::from(Async(
                move |ctx: Ctx<'js>, Rest(args): Rest<Value<'js>>| async move {
                    provider(&Js::new(ctx), name, args).await
                },
            )),
        )?;
    }
    ctx.globals().set("DOMAIN_RESOLVERS", providers)
}
