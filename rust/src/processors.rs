//! Existing local built-in processors, implemented over actual engine values.
use crate::js_language::{Js, get_flag, remove_flag};
use rquickjs::{Array, Function, Object, Value};
use std::{cell::Cell, rc::Rc};

pub fn known(name: &str) -> bool {
    matches!(name, "Useless Filter" | "Region Filter" | "Regex Filter" | "Type Filter" | "Conditional Filter" | "Quick Setting Operator" | "Flag Operator" | "Sort Operator" | "Regex Sort Operator" | "Regex Rename Operator" | "Regex Delete Operator" | "Handle Duplicate Operator")
}

fn is_match<'js>(js: &Js<'js>, rule: Value<'js>, proxy: Value<'js>) -> rquickjs::Result<bool> {
    let operator = js.get(&rule, "operator")?;
    if !js.truthy(&operator)? {
        let attr = js.get(&rule, "attr")?;
        let value: Value = js.call("get", (proxy.clone(), attr.clone()))?;
        let expected = js.get(&rule, "value")?;
        return match js.text(js.get(&rule, "proposition")?)?.as_str() {
            "IN" => {
                let index = js.method(expected, "indexOf", vec![value])?;
                Ok(!js.equal(&index, &js.value(-1)?)?)
            }
            "CONTAINS" => {
                if !value.is_string() { return Ok(false); }
                let index = js.method(value, "indexOf", vec![expected])?;
                Ok(!js.equal(&index, &js.value(-1)?)?)
            }
            "EQUALS" => js.equal(&value, &expected),
            "EXISTS" => {
                // Preserve the legacy OR, including a second read after null.
                if !value.is_null() { return Ok(true); }
                let again: Value = js.call("get", (proxy, attr))?;
                Ok(!again.is_undefined())
            }
            _ => Err(js.error(&format!("Unknown proposition: {}", js.text(js.get(&rule, "proposition")?)?))),
        };
    }
    let child = js.get(&rule, "child")?;
    match js.text(operator)?.as_str() {
        "AND" | "OR" => {
            let every = js.equal(&js.get(&rule, "operator")?, &js.value("AND")?)?;
            let scope = js.clone();
            let callback = Function::new(js.ctx.clone(), move |rule: Value<'js>| is_match(&scope, rule, proxy.clone()))?;
            let output = js.method(child, if every { "every" } else { "some" }, vec![callback.into_value()])?;
            js.truthy(&output)
        }
        "NOT" => Ok(!is_match(js, child, proxy)?),
        _ => Err(js.error(&format!("Unknown operator: {}", js.text(js.get(&rule, "operator")?)?))),
    }
}

fn useless<'js>(js: &Js<'js>, proxy: Value<'js>) -> rquickjs::Result<bool> {
    for key in ["cipher", "password"] {
        let value = js.get(&proxy, key)?;
        if js.truthy(&value)? {
            let regex: Value = js.call("regex", (r"^[\x00-\x7F]+$", ""))?;
            if !js.regex_test(regex, value)? { return Ok(false); }
        }
    }
    let network = js.get(&proxy, "network")?;
    if js.truthy(&network)? {
        let options = js.get(&proxy, &format!("{}-opts", js.text(network)?))?;
        let headers: Value = js.call("optional", (options, "headers"))?;
        let upper: Value = js.call("optional", (headers.clone(), "Host"))?;
        let hosts: Value = if js.truthy(&upper)? { upper } else { js.call("optional", (headers, "host"))? };
        let hosts = if hosts.is_array() { hosts } else { js.array([hosts])? };
        let hosts = Array::from_value(hosts)?;
        for host in hosts.iter::<Value>() {
            let host = host?;
            if js.truthy(&host)? {
                let regex: Value = js.call("regex", (r"^[\x00-\x7F]+$", ""))?;
                if !js.regex_test(regex, host)? { return Ok(false); }
            }
        }
    }
    let regex: Value = js.call("regex", ("网址|流量|时间|应急|过期|Bandwidth|expire", ""))?;
    Ok(!js.regex_test(regex, js.get(&proxy, "name")?)?)
}

fn selected<'js>(js: &Js<'js>, name: &str, args: &Value<'js>, proxy: Value<'js>) -> rquickjs::Result<bool> {
    match name {
        "Useless Filter" => useless(js, proxy),
        "Conditional Filter" => is_match(js, js.get(args, "rule")?, proxy),
        "Region Filter" | "Type Filter" => {
            let input: Value = js.call("optional", (args.clone(), "value"))?;
            let input = if js.truthy(&input)? { input } else { args.clone() };
            let keep: Value = js.call("optional", (args.clone(), "keep"))?;
            let keep = keep.is_undefined() || keep.is_null() || js.truthy(&keep)?;
            let mut found = false;
            let flag = if name == "Region Filter" { get_flag(js, js.get(&proxy, "name")?)? } else { String::new() };
            if let Some(values) = input.as_array() {
                for value in values.iter::<Value>() {
                    let value = value?;
                    found = if name == "Type Filter" {
                        js.equal(&value, &js.get(&proxy, "type")?)?
                    } else {
                        let code = js.text(value)?;
                        let mapped = match code.as_str() { "HK" => "🇭🇰", "TW" => "🇹🇼", "US" => "🇺🇸", "SG" => "🇸🇬", "JP" => "🇯🇵", "UK" => "🇬🇧", "DE" => "🇩🇪", "KR" => "🇰🇷", _ => "" };
                        mapped == flag
                    };
                    if found { break; }
                }
            }
            Ok(if keep { found } else { !found })
        }
        "Regex Filter" => {
            let mut regex = js.get(args, "regex")?;
            if regex.is_undefined() { regex = js.array([])?; }
            let keep = js.get(args, "keep")?;
            let keep = keep.is_undefined() || js.truthy(&keep)?;
            let scope = js.clone();
            let predicate = Function::new(js.ctx.clone(), move |pattern: Value<'js>| {
                scope.regex_test(scope.regex(pattern, "")?, scope.get(&proxy, "name")?)
            })?;
            let found = js.method(regex, "some", vec![predicate.into_value()])?;
            let found = js.truthy(&found)?;
            Ok(if keep { found } else { !found })
        }
        _ => unreachable!(),
    }
}

fn setting<'js>(js: &Js<'js>, value: Value<'js>, default: Value<'js>) -> rquickjs::Result<Value<'js>> {
    if js.equal(&value, &js.value("ENABLED")?)? { js.value(true) }
    else if js.equal(&value, &js.value("DISABLED")?)? { js.value(false) }
    else { Ok(default) }
}

fn quick<'js>(js: &Js<'js>, input: Value<'js>, args: Value<'js>) -> rquickjs::Result<Value<'js>> {
    let mut input = input;
    if setting(js, js.get(&args, "useless")?, js.undefined())? == js.value(true)? {
        let scope = js.clone();
        input = js.filter(input, move |proxy, _| {
            let port = scope.get(&proxy, "port")?;
            Ok(useless(&scope, proxy)? && scope.call::<bool, _>("gt", (port.clone(), 0))? && scope.call::<bool, _>("le", (port, 65535))?)
        })?;
    }
    let scope = js.clone();
    js.map(input, move |proxy| {
        for (arg, field) in [("udp", "udp"), ("tfo", "tfo"), ("tfo", "fast-open"), ("scert", "skip-cert-verify")] {
            let value = setting(&scope, scope.get(&args, arg)?, scope.get(&proxy, field)?)?;
            scope.set(&proxy, field, value)?;
        }
        let kind = scope.text(scope.get(&proxy, "type")?)?;
        for (enabled, arg, field) in [(kind == "vmess", "vmess aead", "aead"), (["snell", "anytls", "trusttunnel"].contains(&kind.as_str()), "reuse", "reuse"), (["tuic", "hysteria2"].contains(&kind.as_str()), "ecn", "ecn")] {
            if enabled {
                let value = setting(&scope, scope.get(&args, arg)?, scope.get(&proxy, field)?)?;
                scope.set(&proxy, field, value)?;
            }
        }
        let value = scope.get(&args, "block-quic")?;
        let selected = if ["auto", "on", "off"].contains(&scope.text(value.clone())?.as_str()) { value } else { scope.get(&proxy, "block-quic")? };
        scope.set(&proxy, "block-quic", selected)?;
        let value = scope.get(&args, "ip-version")?;
        let selected = if value.is_null() || value.is_undefined() || scope.equal(&value, &scope.value("")?)? || scope.equal(&value, &scope.value("DEFAULT")?)? { scope.get(&proxy, "ip-version")? } else { value };
        scope.set(&proxy, "ip-version", selected)?;
        Ok(proxy)
    })
}

fn flag<'js>(js: &Js<'js>, input: Value<'js>, args: Value<'js>) -> rquickjs::Result<Value<'js>> {
    let mode = js.get(&args, "mode")?;
    let tw = js.get(&args, "tw")?;
    let scope = js.clone();
    js.map(input, move |proxy| {
        let name = scope.get(&proxy, "name")?;
        let removing = scope.equal(&mode, &scope.value("remove")?)?;
        let country = if removing { String::new() } else { get_flag(&scope, name.clone())? };
        let mut name = remove_flag(&scope, name)?;
        if !removing {
            name = scope.call("add", (format!("{country} "), name))?;
            if !scope.call::<bool, _>("looseEqual", (tw.clone(), "tw"))? {
                let replacement = if scope.call::<bool, _>("looseEqual", (tw.clone(), "ws"))? { "🇼🇸" } else { "🇨🇳" };
                let regex: Value = scope.call("regex", ("🇹🇼", "g"))?;
                name = scope.method(name, "replace", vec![regex, scope.value(replacement)?])?;
            }
        }
        scope.set(&proxy, "name", name)?;
        Ok(proxy)
    })
}

fn name_sort<'js>(js: &Js<'js>, input: Value<'js>, order: Value<'js>) -> rquickjs::Result<Value<'js>> {
    match js.text(order.clone())?.as_str() {
        "asc" | "desc" => {
            let descending = js.equal(&order, &js.value("desc")?)?;
            let scope = js.clone();
            js.sort(input, move |a, b| {
                let greater: bool = scope.call("gt", (scope.get(&a, "name")?, scope.get(&b, "name")?))?;
                Ok((if greater { 1 } else { -1 }) * if descending { -1 } else { 1 })
            })
        }
        "random" => {
            let length: usize = js.call("length", (input.clone(),))?;
            for current in (1..=length).rev() {
                let random: f64 = js.call("random", ())?;
                let index = (random * current as f64).floor() as usize;
                let a: Value = js.call("get", (input.clone(), current - 1))?;
                let b: Value = js.call("get", (input.clone(), index))?;
                js.call::<(), _>("set", (input.clone(), current - 1, b))?;
                js.call::<(), _>("set", (input.clone(), index, a))?;
            }
            Ok(input)
        }
        _ => Err(js.error(&format!("Unknown sort option: {}", js.text(order)?))),
    }
}

fn regex_order<'js>(js: &Js<'js>, expressions: &Value<'js>, name: Value<'js>) -> rquickjs::Result<usize> {
    let length: usize = js.call("length", (expressions.clone(),))?;
    for index in 0..length {
        let expression: Value = js.call("get", (expressions.clone(), index))?;
        if js.regex_test(expression, name.clone())? { return Ok(index + 1); }
    }
    Ok(0)
}

fn regex_sort<'js>(js: &Js<'js>, input: Value<'js>, args: Value<'js>) -> rquickjs::Result<Value<'js>> {
    let order = js.get(&args, "order")?;
    let order = if js.truthy(&order)? { order } else { js.value("asc")? };
    let expressions = if args.is_array() { args } else { js.get(&args, "expressions")? };
    let expressions = if expressions.is_array() { expressions } else { js.array([])? };
    let scope = js.clone();
    let expressions = js.map(expressions, move |pattern| scope.regex(pattern, ""))?;
    let scope = js.clone();
    js.sort(input, move |a, b| {
        let a = scope.get(&a, "name")?;
        let b = scope.get(&b, "name")?;
        let oa = regex_order(&scope, &expressions, a.clone())?;
        let ob = regex_order(&scope, &expressions, b.clone())?;
        if oa > 0 && ob == 0 { return Ok(-1); }
        if ob > 0 && oa == 0 { return Ok(1); }
        if oa > 0 && ob > 0 { return Ok(if oa < ob { -1 } else { 1 }); }
        if scope.equal(&order, &scope.value("original")?)? { return Ok(0); }
        let less: bool = scope.call("lt", (a, b))?;
        let ascending = !scope.equal(&order, &scope.value("desc")?)?;
        Ok(if less == ascending { -1 } else { 1 })
    })
}

fn rename<'js>(js: &Js<'js>, input: Value<'js>, patterns: Value<'js>) -> rquickjs::Result<Value<'js>> {
    let scope = js.clone();
    js.map(input, move |proxy| {
        // for-of visits array holes, unlike the outer map.
        let patterns: Vec<Value> = scope.call("iterable", (patterns.clone(),))?;
        for pattern in patterns {
            let expression = scope.regex(scope.get(&pattern, "expr")?, "g")?;
            let replacement = scope.get(&pattern, "now")?;
            let name = scope.method(scope.get(&proxy, "name")?, "replace", vec![expression, replacement])?;
            let name = scope.method(name, "trim", vec![])?;
            scope.set(&proxy, "name", name)?;
        }
        Ok(proxy)
    })
}

fn duplicate_key<'js>(js: &Js<'js>, fields: Value<'js>, proxy: Value<'js>) -> rquickjs::Result<Value<'js>> {
    let lodash: Object = js.ctx.globals().get("lodash")?;
    let get: Function = lodash.get("get")?;
    let values = js.map(fields, move |field| get.call((proxy.clone(), field, "-")))?;
    js.method(values, "join", vec![js.value("_")?])
}

fn duplicate<'js>(js: &Js<'js>, input: Value<'js>, args: Value<'js>) -> rquickjs::Result<Value<'js>> {
    let defaults = js.ctx.json_parse(r#"{"action":"rename","template":"0 1 2 3 4 5 6 7 8 9","link":"-","position":"back","field":["name"]}"#)?;
    let args: Value = js.call("assign", (defaults, args))?;
    let action = js.get(&args, "action")?;
    let fields = js.get(&args, "field")?;
    let counter = Object::new(js.ctx.clone())?.into_value();
    if js.equal(&action, &js.value("delete")?)? {
        let scope = js.clone();
        return js.filter(input, move |proxy, _| {
            let key = duplicate_key(&scope, fields.clone(), proxy)?;
            let chosen: Value = scope.call("get", (counter.clone(), key.clone()))?;
            if scope.truthy(&chosen)? { return Ok(false); }
            scope.call::<(), _>("set", (counter.clone(), key, true))?;
            Ok(true)
        });
    }
    if !js.equal(&action, &js.value("rename")?)? { return Ok(js.undefined()); }
    let numbers = js.method(js.get(&args, "template")?, "split", vec![js.value(" ")?])?;
    let max_length = Rc::new(Cell::new(0usize));
    let maximum = max_length.clone();
    let counts = counter.clone();
    let key_fields = fields.clone();
    let scope = js.clone();
    js.each(input.clone(), move |proxy| {
        let key = duplicate_key(&scope, key_fields.clone(), proxy)?;
        let previous: Value = scope.call("get", (counts.clone(), key.clone()))?;
        if previous.is_undefined() { scope.call::<(), _>("set", (counts.clone(), key.clone(), 1))?; }
        else { scope.call::<Value, _>("increment", (counts.clone(), key.clone()))?; }
        let count: Value = scope.call("get", (counts.clone(), key))?;
        let string = scope.method(count, "toString", vec![])?;
        let length: usize = scope.call("length", (string,))?;
        maximum.set(maximum.get().max(length));
        Ok(())
    })?;
    let increment = Object::new(js.ctx.clone())?.into_value();
    let link = js.get(&args, "link")?;
    let position = js.get(&args, "position")?;
    let scope = js.clone();
    js.map(input, move |proxy| {
        let key = duplicate_key(&scope, fields.clone(), proxy.clone())?;
        let count: Value = scope.call("get", (counter.clone(), key.clone()))?;
        if !scope.call::<bool, _>("gt", (count, 1))? { return Ok(proxy); }
        let previous: Value = scope.call("get", (increment.clone(), key.clone()))?;
        if previous.is_undefined() { scope.call::<(), _>("set", (increment.clone(), key.clone(), 1))?; }
        let mut count: f64 = scope.call("increment", (increment.clone(), key))?;
        let mut number = scope.value("")?;
        let mut digits = 0;
        while count > 0.0 {
            let digit: Value = scope.call("get", (numbers.clone(), count % 10.0))?;
            number = scope.call("add", (digit, number))?;
            count = (count / 10.0).trunc();
            digits += 1;
        }
        while digits < max_length.get() {
            let zero: Value = scope.call("get", (numbers.clone(), 0))?;
            number = scope.call("add", (zero, number))?;
            digits += 1;
        }
        let name = scope.get(&proxy, "name")?;
        if scope.equal(&position, &scope.value("front")?)? {
            let prefix: Value = scope.call("add", (number, link.clone()))?;
            let name = scope.call("add", (prefix, name))?;
            scope.set(&proxy, "name", name)?;
        } else if scope.equal(&position, &scope.value("back")?)? {
            let prefix: Value = scope.call("add", (name, link.clone()))?;
            let name = scope.call("add", (prefix, number))?;
            scope.set(&proxy, "name", name)?;
        }
        Ok(proxy)
    })
}

pub fn prepare<'js>(js: &Js<'js>, name: &str, args: Value<'js>) -> rquickjs::Result<Value<'js>> {
    if name != "Regex Delete Operator" { return Ok(args); }
    let scope = js.clone();
    js.map(args, move |expression| {
        let pattern = Object::new(scope.ctx.clone())?;
        pattern.set("expr", expression)?;
        pattern.set("now", "")?;
        Ok(pattern.into_value())
    })
}

pub fn apply<'js>(js: &Js<'js>, name: &str, input: Value<'js>, args: Value<'js>) -> rquickjs::Result<Value<'js>> {
    if name.contains("Filter") {
        let scope = js.clone();
        let name = name.to_owned();
        let selected = js.map(input.clone(), move |proxy| scope.value(selected(&scope, &name, &args, proxy)?))?;
        let scope = js.clone();
        return js.filter(input, move |_, index| {
            let selected: Value = scope.call("get", (selected.clone(), index))?;
            scope.truthy(&selected)
        });
    }
    match name {
        "Quick Setting Operator" => quick(js, input, args),
        "Flag Operator" => flag(js, input, args),
        "Sort Operator" => name_sort(js, input, args),
        "Regex Sort Operator" => regex_sort(js, input, args),
        "Regex Rename Operator" | "Regex Delete Operator" => rename(js, input, args),
        "Handle Duplicate Operator" => duplicate(js, input, args),
        _ => unreachable!(),
    }
}
