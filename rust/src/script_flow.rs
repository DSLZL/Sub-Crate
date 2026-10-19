//! Existing flow-header and calendar algorithms exposed to application scripts.
use crate::js_language::Js;
use crate::script_network as network;
use rquickjs::{
    Array, Ctx, Function, Object, Value,
    prelude::{Async, Func, Opt},
};

fn global<'js>(js: &Js<'js>, name: &str, value: Value<'js>) -> rquickjs::Result<Value<'js>> {
    let function: Function = js.ctx.globals().get(name)?;
    function.call((value,))
}

fn optional<'js>(js: &Js<'js>, value: Value<'js>, key: &str) -> rquickjs::Result<Value<'js>> {
    js.call("optional", (value, key))
}

fn fallback<'js>(
    js: &Js<'js>,
    value: Value<'js>,
    otherwise: Value<'js>,
) -> rquickjs::Result<Value<'js>> {
    Ok(if js.truthy(&value)? { value } else { otherwise })
}

fn field<'js>(ctx: Ctx<'js>, headers: Opt<Value<'js>>) -> rquickjs::Result<Value<'js>> {
    let js = Js::new(ctx);
    let headers = headers.0.unwrap_or_else(|| js.undefined());
    let object = js.ctx.globals().get("Object")?;
    let keys = js.method(object, "keys", vec![headers.clone()])?;
    let keys: Vec<Value> = js.call("iterable", (keys,))?;
    let mut subscription = js.value("")?;
    let mut web = js.value("")?;
    let mut plan = js.value("")?;
    for key in keys {
        let lower = js.method(key.clone(), "toLowerCase", vec![])?;
        for (name, target) in [
            ("subscription-userinfo", &mut subscription),
            ("profile-web-page-url", &mut web),
            ("plan-name", &mut plan),
        ] {
            if js.equal(&lower, &js.value(name)?)? {
                *target = js.call("get", (headers.clone(), key.clone()))?;
            }
        }
    }
    let mut result = fallback(&js, subscription, js.value("")?)?;
    // Template interpolation coerces the subscription field even without metadata.
    result = js.call("add", (js.value("")?, result))?;
    for (name, value) in [("app_url", web), ("plan_name", plan)] {
        if js.truthy(&value)? {
            let encoded = global(&js, "encodeURIComponent", value)?;
            result = js.call("add", (result, format!("; {name}=")))?;
            result = js.call("add", (result, encoded))?;
        }
    }
    Ok(result)
}

fn capture<'js>(js: &Js<'js>, raw: &Value<'js>, pattern: &str) -> rquickjs::Result<Value<'js>> {
    let expression = js.call("regex", (pattern, ""))?;
    js.method(raw.clone(), "match", vec![expression])
}

fn number_match<'js>(js: &Js<'js>, found: Value<'js>) -> rquickjs::Result<Value<'js>> {
    let sign: Value = js.call("get", (found.clone(), 1))?;
    let digits: Value = js.call("get", (found, 2))?;
    let text: Value = js.call("add", (sign, digits))?;
    js.call("number", (text,))
}

fn valid_body<'js>(js: &Js<'js>, body: Value<'js>) -> rquickjs::Result<bool> {
    let parsed = parse(js.ctx.clone(), Opt(Some(body)))?;
    let usage = optional(js, parsed.clone(), "usage")?;
    for value in [
        optional(js, parsed, "total")?,
        optional(js, usage.clone(), "download")?,
        optional(js, usage, "upload")?,
    ] {
        if !value.is_number() || !js.call::<f64, _>("number", (value,))?.is_finite() {
            return Ok(false);
        }
    }
    Ok(true)
}

fn report(js: &Js<'_>, operation: &str, error: rquickjs::Error) {
    crate::script::log(js, "error", &format!("{operation}: {}", crate::script::message(&js.ctx, error)));
}

async fn fetch<'js>(
    ctx: Ctx<'js>,
    raw: Opt<Value<'js>>,
    agent: Opt<Value<'js>>,
    timeout: Opt<Value<'js>>,
    proxy: Opt<Value<'js>>,
    flow_url: Opt<Value<'js>>,
    flow_headers: Opt<Value<'js>>,
) -> rquickjs::Result<Value<'js>> {
    let js = Js::new(ctx);
    let flow_url = flow_url.0.unwrap_or_else(|| js.undefined());
    let raw = network::prefer(
        &js,
        [
            flow_url.clone(),
            raw.0.unwrap_or_else(|| js.undefined()),
            js.value("")?,
        ],
    )?;
    let (url, arguments) = network::fragment(&js, raw)?;
    if js.truthy(&optional(&js, arguments.clone(), "noFlow")?)?
        || !js.regex_test(js.call("regex", ("^https?", ""))?, url.clone())?
    {
        return Ok(js.undefined());
    }
    let settings = network::settings(&js)?;
    let proxy = network::prefer(
        &js,
        [
            proxy.0.unwrap_or_else(|| js.undefined()),
            js.get(&settings, "defaultProxy")?,
        ],
    )?;
    let agent = network::prefer(
        &js,
        [
            agent.0.unwrap_or_else(|| js.undefined()),
            js.get(&settings, "defaultFlowUserAgent")?,
            js.value("clash.meta/v1.19.23")?,
        ],
    )?;
    let timeout = network::prefer(
        &js,
        [
            timeout.0.unwrap_or_else(|| js.undefined()),
            js.get(&settings, "defaultTimeout")?,
            js.value(8000)?,
        ],
    )?;
    let header_argument = network::prefer(
        &js,
        [
            flow_headers.0.unwrap_or_else(|| js.undefined()),
            optional(&js, arguments.clone(), "flowHeaders")?,
            optional(&js, arguments.clone(), "headers")?,
        ],
    )?;
    let mut headers = js.undefined();
    if js.truthy(&header_argument)? {
        match network::headers(&js, header_argument, agent.clone()) {
            Ok(value) => headers = value,
            Err(error) => report(&js, "解析自定义流量头失败", error),
        }
    }
    let identity = if js.truthy(&headers)? {
        js.ctx
            .json_stringify(headers.clone())?
            .map_or_else(|| js.undefined(), |text| text.into_value())
    } else {
        agent.clone()
    };
    let identity: Value = js.call("add", (identity, url.clone()))?;
    let id = crate::script_helpers::md5(js.ctx.clone(), Opt(Some(identity)))?;
    let cache: Value = js.ctx.globals().get("__headersResourceCache")?;
    let cached = js.method(cache.clone(), "get", vec![js.value(id.clone())?])?;
    if !js.truthy(&optional(&js, arguments.clone(), "noCache")?)? && js.truthy(&cached)? {
        return Ok(cached);
    }
    if !js.truthy(&headers)? {
        headers = Object::new(js.ctx.clone())?.into_value();
        js.set(&headers, "User-Agent", agent)?;
    }
    let options = Object::new(js.ctx.clone())?.into_value();
    js.set(&options, "url", url.clone())?;
    js.set(&options, "headers", headers)?;
    js.set(&options, "timeout", timeout)?;
    if js.truthy(&proxy)? {
        js.set(&options, "proxy", proxy)?;
    }
    if js.truthy(&optional(&js, arguments.clone(), "insecure")?)? {
        js.set(&options, "insecure", js.value(true)?)?;
    }
    let mut flow = js.undefined();
    if js.truthy(&flow_url)? {
        let mut response_headers = js.undefined();
        let response = async {
            let response = network::request(&js, "get", options).await?;
            let status = js.get(&response, "statusCode")?;
            if js.call::<bool, _>("lt", (status.clone(), 200))?
                || js.call::<bool, _>("le", (400, status.clone()))?
            {
                return Err(js.error(&format!("statusCode: {}", js.text(status)?)));
            }
            response_headers = js.get(&response, "headers")?;
            let body = js.get(&response, "body")?;
            if !valid_body(&js, body.clone())? {
                return Err(js.error("响应体中未包含合法的流量信息"));
            }
            Ok(body)
        }
        .await;
        match response {
            Ok(value) => flow = value,
            Err(error) => {
                report(&js, "使用 GET 方法从响应体获取流量信息失败", error);
                if js.truthy(&response_headers)? {
                    let response = (|| {
                        let value = field(js.ctx.clone(), Opt(Some(response_headers)))?;
                        if !valid_body(&js, value.clone())? {
                            return Err(js.error("响应体中未包含合法的流量信息"));
                        }
                        Ok(value)
                    })();
                    match response {
                        Ok(value) => flow = value,
                        Err(error) => report(&js, "使用 GET 方法从响应头获取流量信息失败", error),
                    }
                }
            }
        }
    } else {
        let lines = js.method(url, "split", vec![js.call("regex", (r"[\r\n]+", ""))?])?;
        let lines: Vec<Value> = js.call("iterable", (lines,))?;
        let mut first = js.undefined();
        for line in lines {
            let line = js.method(line, "trim", vec![])?;
            if js.truthy(&line)? {
                first = line;
                break;
            }
        }
        js.set(&options, "url", first)?;
        let response = async {
            let response = network::request(&js, "head", options.clone()).await?;
            field(js.ctx.clone(), Opt(Some(js.get(&response, "headers")?)))
        }
        .await;
        match response {
            Ok(value) => flow = value,
            Err(error) => report(&js, "使用 HEAD 方法从响应头获取流量信息失败", error),
        }
        if !js.truthy(&flow)? {
            let response = network::request(&js, "get", options).await?;
            flow = field(js.ctx.clone(), Opt(Some(js.get(&response, "headers")?)))?;
        }
    }
    if js.truthy(&flow)? {
        flow = js.method(flow, "trim", vec![])?;
    }
    if js.truthy(&flow)? {
        let ttl = optional(&js, arguments, "headersCacheTtl")?;
        let ttl = if js.truthy(&ttl)? {
            js.call("multiply", (ttl, 1000))?
        } else {
            js.undefined()
        };
        js.method(cache, "set", vec![js.value(id)?, flow.clone(), ttl])?;
    }
    Ok(flow)
}

fn parse<'js>(ctx: Ctx<'js>, raw: Opt<Value<'js>>) -> rquickjs::Result<Value<'js>> {
    let js = Js::new(ctx);
    let raw = raw.0.unwrap_or_else(|| js.undefined());
    if !js.truthy(&raw)? {
        return Ok(js.undefined());
    }
    let numeric = |name: &str| {
        capture(
            &js,
            &raw,
            &format!(r"{name}=([-+]?)([0-9]*\.?[0-9]+([eE][-+]?[0-9]+)?)"),
        )
    };
    let upload = numeric("upload")?;
    let upload = if upload.is_null() {
        js.value(0)?
    } else {
        number_match(&js, upload)?
    };
    let download = number_match(&js, numeric("download")?)?;
    let total = number_match(&js, numeric("total")?)?;
    let expires = numeric("expire")?;
    let expires = if js.truthy(&expires)? {
        number_match(&js, expires)?
    } else {
        js.undefined()
    };
    let days = capture(&js, &raw, r"reset_day=([0-9]+)")?;
    let days = if js.truthy(&days)? {
        let value: Value = js.call("get", (days, 1))?;
        js.call("number", (value,))?
    } else {
        js.undefined()
    };
    let result = Object::new(js.ctx.clone())?;
    result.set("expires", expires)?;
    result.set("total", total)?;
    let usage = Object::new(js.ctx.clone())?;
    usage.set("upload", upload)?;
    usage.set("download", download)?;
    result.set("usage", usage)?;
    result.set("remainingDays", days)?;
    for (name, field) in [("app_url", "appUrl"), ("plan_name", "planName")] {
        let found = capture(&js, &raw, &format!(r"{name}=(.*?)\s*?(;|$)"))?;
        let value = if js.truthy(&found)? {
            let value: Value = js.call("get", (found, 1))?;
            global(&js, "decodeURIComponent", value)?
        } else {
            js.undefined()
        };
        result.set(field, value)?;
    }
    Ok(result.into_value())
}

fn transfer<'js>(
    ctx: Ctx<'js>,
    flow: Opt<Value<'js>>,
    unit: Opt<Value<'js>>,
) -> rquickjs::Result<Value<'js>> {
    let js = Js::new(ctx);
    let mut flow = flow.0.unwrap_or_else(|| js.undefined());
    let mut unit = unit
        .0
        .filter(|value| !value.is_undefined())
        .unwrap_or(js.value("B")?);
    let units = ["B", "KB", "MB", "GB", "TB", "PB", "EB", "ZB", "YB"];
    let mut index = -1isize;
    for (position, name) in units.iter().enumerate() {
        if js.equal(&unit, &js.value(*name)?)? {
            index = position as isize;
            break;
        }
    }
    loop {
        if js.call::<bool, _>("lt", (flow.clone(), 1024))? || index == 8 {
            let product: Value = js.call("multiply", (flow, 100))?;
            let math = js.ctx.globals().get("Math")?;
            let rounded = js.method(math, "round", vec![product])?;
            let value: Value = js.call("divide", (rounded, 100))?;
            let result = Object::new(js.ctx.clone())?;
            result.set("value", js.method(value, "toString", vec![])?)?;
            result.set("unit", unit)?;
            return Ok(result.into_value());
        }
        flow = js.call("divide", (flow, 1024))?;
        index += 1;
        unit = js.value(units[index as usize])?;
    }
}

fn date<'js>(js: &Js<'js>, args: Vec<Value<'js>>) -> rquickjs::Result<Value<'js>> {
    let constructor: Value = js.ctx.globals().get("Date")?;
    js.call("construct", (constructor, js.array(args)?))
}

fn valid<'js>(ctx: Ctx<'js>, flow: Opt<Value<'js>>) -> rquickjs::Result<()> {
    let js = Js::new(ctx);
    let flow = flow.0.unwrap_or_else(|| js.undefined());
    if !js.truthy(&flow)? {
        return Err(js.error("没有流量信息"));
    }
    let expires = optional(&js, flow.clone(), "expires")?;
    if js.truthy(&expires)? {
        let expires: Value = js.call("multiply", (expires.clone(), 1000))?;
        let now: Value = js.call("now", ())?;
        if js.call::<bool, _>("lt", (expires.clone(), now))? {
            let value = date(&js, vec![expires])?;
            let value = js.method(value, "toLocaleDateString", vec![])?;
            return Err(js.error(&format!("订阅已过期: {}", js.text(value)?)));
        }
    }
    let total = optional(&js, flow.clone(), "total")?;
    if js.truthy(&total)? {
        let usage = js.get(&flow, "usage")?;
        let upload = fallback(&js, optional(&js, usage.clone(), "upload")?, js.value(0)?)?;
        let download = fallback(&js, optional(&js, usage, "download")?, js.value(0)?)?;
        let remaining: Value = js.call("subtract", (total.clone(), upload.clone()))?;
        let remaining: Value = js.call("subtract", (remaining, download.clone()))?;
        if js.call::<bool, _>("le", (remaining, 0))? {
            let current: Value = js.call("add", (upload, download))?;
            let math = js.ctx.globals().get("Math")?;
            let absolute = js.method(math, "abs", vec![current.clone()])?;
            let current_text = transfer(js.ctx.clone(), Opt(Some(absolute)), Opt(None))?;
            let value = js.get(&current_text, "value")?;
            if js.call::<bool, _>("lt", (current, 0))? {
                let value = js.call("add", ("-", value))?;
                js.set(&current_text, "value", value)?;
            }
            let total_text = transfer(js.ctx.clone(), Opt(Some(total)), Opt(None))?;
            return Err(js.error(&format!(
                "流量已用完: {} {} / {} {}",
                js.text(js.get(&current_text, "value")?)?,
                js.text(js.get(&current_text, "unit")?)?,
                js.text(js.get(&total_text, "value")?)?,
                js.text(js.get(&total_text, "unit")?)?
            )));
        }
    }
    Ok(())
}

fn remaining<'js>(ctx: Ctx<'js>, options: Opt<Value<'js>>) -> rquickjs::Result<Value<'js>> {
    let js = Js::new(ctx);
    let result = remaining_inner(
        &js,
        options
            .0
            .filter(|value| !value.is_undefined())
            .unwrap_or(Object::new(js.ctx.clone())?.into_value()),
    );
    match result {
        Ok(value) => Ok(value),
        Err(error) => {
            crate::script::log(&js, "error", &format!("getRmainingDays failed: {}", crate::script::message(&js.ctx, error)));
            Ok(js.undefined())
        }
    }
}

fn remaining_inner<'js>(js: &Js<'js>, options: Value<'js>) -> rquickjs::Result<Value<'js>> {
    let mut reset = js.get(&options, "resetDay")?;
    let start = js.get(&options, "startDate")?;
    let cycle = js.get(&options, "cycleDays")?;
    if options.is_string() || options.is_number() {
        reset = options;
    }
    if js.truthy(&start)? && js.truthy(&cycle)? {
        let cycle = global(js, "parseInt", cycle)?;
        let number: f64 = js.call("number", (cycle.clone(),))?;
        if number.is_nan() || number <= 0.0 {
            return Err(js.error("重置周期应为正整数"));
        }
        let constructor = js.ctx.globals().get("Date")?;
        let parsed = js.method(constructor, "parse", vec![start.clone()])?;
        if !js.truthy(&parsed)? {
            return Err(js.error("开始日期不合法"));
        }
        let first = date(js, vec![start.clone()])?;
        let today = date(js, vec![])?;
        for value in [&first, &today] {
            js.method(
                value.clone(),
                "setHours",
                vec![js.value(0)?, js.value(0)?, js.value(0)?, js.value(0)?],
            )?;
        }
        let start_time = js.method(first, "getTime", vec![])?;
        let today_time = js.method(today.clone(), "getTime", vec![])?;
        if js.call::<bool, _>("gt", (start_time, today_time.clone()))? {
            return Err(js.error("开始日期应早于现在"));
        }
        let reset = date(js, vec![start])?;
        loop {
            let day = js.method(reset.clone(), "getDate", vec![])?;
            let day = js.call("add", (day, cycle.clone()))?;
            js.method(reset.clone(), "setDate", vec![day])?;
            if !js.call::<bool, _>("lt", (reset.clone(), today.clone()))? {
                break;
            }
        }
        js.method(
            reset.clone(),
            "setHours",
            vec![js.value(0)?, js.value(0)?, js.value(0)?, js.value(0)?],
        )?;
        let reset_time = js.method(reset, "getTime", vec![])?;
        let diff: Value = js.call("subtract", (reset_time, today_time))?;
        let days: f64 = js.call("divide", (diff, 86400000))?;
        return js.value(days.ceil());
    }
    if !js.truthy(&reset)? {
        return Ok(js.undefined());
    }
    let reset = global(js, "parseInt", reset)?;
    let reset: f64 = js.call("number", (reset,))?;
    if reset.is_nan() || reset <= 0.0 || reset > 31.0 {
        return Err(js.error("月重置日应为 1-31 之间的整数"));
    }
    let now = date(js, vec![])?;
    let today = js.method(now.clone(), "getDate", vec![])?;
    let today: f64 = js.call("number", (today,))?;
    let days = if reset > today {
        0.0
    } else {
        let month = js.method(now.clone(), "getMonth", vec![])?;
        let year = js.method(now, "getFullYear", vec![])?;
        let next = js.call("add", (month, 1))?;
        let last = date(js, vec![year, next, js.value(0)?])?;
        let days = js.method(last, "getDate", vec![])?;
        js.call("number", (days,))?
    };
    js.value(days - today + reset)
}

fn normalized_inner<'js>(
    js: &Js<'js>,
    raw: Value<'js>,
    split: Value<'js>,
) -> rquickjs::Result<Value<'js>> {
    let parts = js.method(raw, "split", vec![js.value(";")?])?;
    let parts: Vec<Value> = js.call("iterable", (parts,))?;
    let mut entries: Vec<(Value, Value)> = Vec::new();
    for part in parts {
        let part = js.method(part, "trim", vec![])?;
        if !js.truthy(&part)? {
            continue;
        }
        let equals = js.method(part.clone(), "indexOf", vec![js.value("=")?])?;
        if js.equal(&equals, &js.value(-1)?)? {
            continue;
        }
        let key = js.method(part.clone(), "slice", vec![js.value(0)?, equals.clone()])?;
        let key = js.method(key, "trim", vec![])?;
        let mut seen = false;
        for (old, _) in &entries {
            if js.equal(old, &key)? {
                seen = true;
                break;
            }
        }
        if seen {
            continue;
        }
        let next = js.call("add", (equals, 1))?;
        let encoded = js.method(part, "slice", vec![next])?;
        let encoded = js.method(encoded, "trim", vec![])?;
        let decoded = (|| {
            let mut value = global(js, "decodeURIComponent", encoded.clone())?;
            for name in ["upload", "download", "total", "expire", "reset_day"] {
                if !js.equal(&key, &js.value(name)?)? {
                    continue;
                }
                let number: f64 = js.call("number", (value.clone(),))?;
                value = if ["expire", "reset_day"].contains(&name)
                    && (number <= 0.0 || !number.is_finite())
                {
                    js.value("")?
                } else if ["upload", "download", "total"].contains(&name) && !number.is_finite() {
                    js.value(0)?
                } else {
                    js.method(js.value(number)?, "toFixed", vec![js.value(0)?])?
                };
                break;
            }
            Ok::<_, rquickjs::Error>(value)
        })();
        let value = match decoded {
            Ok(value) => value,
            Err(_) => {
                js.ctx.catch();
                encoded
            }
        };
        entries.push((key, value));
    }
    let headers = js
        .ctx
        .json_parse(r#"{"subscription-userinfo":"","profile-web-page-url":"","plan-name":""}"#)?;
    let user = Object::new(js.ctx.clone())?.into_value();
    for (key, value) in entries {
        if js.truthy(&split)? && js.equal(&key, &js.value("app_url")?)? {
            js.set(&headers, "profile-web-page-url", value)?;
        } else if js.truthy(&split)? && js.equal(&key, &js.value("plan_name")?)? {
            js.set(&headers, "plan-name", value)?;
        } else {
            js.call::<(), _>("set", (user.clone(), key, value))?;
        }
    }
    let pairs: Array = js.call("entries", (user,))?;
    if !pairs.is_empty() {
        let result = Array::new(js.ctx.clone())?;
        for (index, pair) in pairs.iter::<Array>().enumerate() {
            let pair = pair?;
            let key: Value = pair.get(0)?;
            let value: Value = pair.get(1)?;
            let value = global(js, "encodeURIComponent", value)?;
            let key: Value = js.call("add", (key, "="))?;
            result.set(index, js.call::<Value, _>("add", (key, value))?)?;
        }
        js.set(
            &headers,
            "subscription-userinfo",
            js.method(result.into_value(), "join", vec![js.value("; ")?])?,
        )?;
    }
    if js.truthy(&split)? {
        Ok(headers)
    } else {
        js.get(&headers, "subscription-userinfo")
    }
}

fn normalize<'js>(
    ctx: Ctx<'js>,
    raw: Opt<Value<'js>>,
    split: Opt<Value<'js>>,
) -> rquickjs::Result<Value<'js>> {
    let js = Js::new(ctx);
    let raw = raw.0.unwrap_or_else(|| js.undefined());
    let split = split.0.unwrap_or_else(|| js.undefined());
    match normalized_inner(&js, raw.clone(), split.clone()) {
        Ok(value) => Ok(value),
        Err(error) => {
            crate::script::log(&js, "error", &format!("normalizeFlowHeader failed: {}", crate::script::message(&js.ctx, error)));
            if js.truthy(&split)? {
                let headers = Object::new(js.ctx.clone())?;
                headers.set("subscription-userinfo", raw)?;
                Ok(headers.into_value())
            } else {
                Ok(raw)
            }
        }
    }
}

pub fn initialize(ctx: &Ctx<'_>) -> rquickjs::Result<()> {
    let flow = Object::new(ctx.clone())?;
    flow.set("getFlowHeaders", Func::from(Async(fetch)))?;
    flow.set("getFlowField", Func::from(field))?;
    flow.set("parseFlowHeaders", Func::from(parse))?;
    flow.set("flowTransfer", Func::from(transfer))?;
    flow.set("validCheck", Func::from(valid))?;
    flow.set("getRmainingDays", Func::from(remaining))?;
    flow.set("normalizeFlowHeader", Func::from(normalize))?;
    ctx.globals().set("flowUtils", flow)
}
