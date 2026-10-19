//! Public script helpers backed by native algorithms and existing language utilities.
use crate::js_language::{self, Js};
use rquickjs::{
    Array, Ctx, Function, Object, Value,
    prelude::{Func, Opt, This},
};

const IPV4: &str = r"^((25[0-5]|(2[0-4]|1\d|[1-9]|)\d)(\.(?!$)|$)){4}$";
const IPV6: &str = r"^(([0-9a-fA-F]{1,4}:){7,7}[0-9a-fA-F]{1,4}|([0-9a-fA-F]{1,4}:){1,7}:|([0-9a-fA-F]{1,4}:){1,6}:[0-9a-fA-F]{1,4}|([0-9a-fA-F]{1,4}:){1,5}(:[0-9a-fA-F]{1,4}){1,2}|([0-9a-fA-F]{1,4}:){1,4}(:[0-9a-fA-F]{1,4}){1,3}|([0-9a-fA-F]{1,4}:){1,3}(:[0-9a-fA-F]{1,4}){1,4}|([0-9a-fA-F]{1,4}:){1,2}(:[0-9a-fA-F]{1,4}){1,5}|[0-9a-fA-F]{1,4}:((:[0-9a-fA-F]{1,4}){1,6})|:((:[0-9a-fA-F]{1,4}){1,7}|:)|fe80:(:[0-9a-fA-F]{0,4}){0,4}%[0-9a-zA-Z]{1,}|::(ffff(:0{1,4}){0,1}:){0,1}((25[0-5]|(2[0-4]|1{0,1}[0-9]){0,1}[0-9])\.){3,3}(25[0-5]|(2[0-4]|1{0,1}[0-9]){0,1}[0-9])|([0-9a-fA-F]{1,4}:){1,4}:((25[0-5]|(2[0-4]|1{0,1}[0-9]){0,1}[0-9])\.){3,3}(25[0-5]|(2[0-4]|1{0,1}[0-9]){0,1}[0-9]))$";
const UUID: &str = r"^[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}$";
const CACHE_KEY: &str = "#sub-store-cached-script-resource";

fn matches<'js>(js: &Js<'js>, pattern: &str, value: Value<'js>) -> rquickjs::Result<bool> {
    let expression = js.call("regex", (pattern, ""))?;
    js.regex_test(expression, value)
}

fn random_port<'js>(ctx: Ctx<'js>, input: Opt<Value<'js>>) -> rquickjs::Result<Value<'js>> {
    let js = Js::new(ctx);
    let input = input.0.unwrap_or_else(|| js.undefined());
    let expression = js.call("regex", (",|/", ""))?;
    let parts = js.method(input, "split", vec![expression])?;
    let length: Value = js.call("length", (parts.clone(),))?;
    let draw: Value = js.call("random", ())?;
    let index: f64 = js.call("multiply", (draw, length))?;
    let part: Value = js.call("get", (parts, index.floor()))?;
    if js.truthy(&js.method(part.clone(), "includes", vec![js.value("-")?])?)? {
        let range = js.method(part, "split", vec![js.value("-")?])?;
        let min: Value = js.call("get", (range.clone(), 0))?;
        let max: Value = js.call("get", (range, 1))?;
        let min: f64 = js.call("number", (min,))?;
        let max: f64 = js.call("number", (max,))?;
        let min = min.ceil();
        let max = max.floor();
        let draw: Value = js.call("random", ())?;
        let draw: f64 = js.call("multiply", (draw, max - min + 1.0))?;
        js.value(draw.floor() + min)
    } else {
        js.call("number", (part,))
    }
}

fn get_iso<'js>(ctx: Ctx<'js>, name: Opt<Value<'js>>) -> rquickjs::Result<Value<'js>> {
    let js = Js::new(ctx);
    let flag = js_language::get_flag(&js, name.0.unwrap_or_else(|| js.undefined()))?;
    let data: Object = js.ctx.globals().get("__geo")?;
    let rows: Array = data.get("iso")?;
    for row in rows.iter::<Array>() {
        let row = row?;
        if row.get::<String>(0)? == flag {
            let names: Array = row.get(1)?;
            return names.get(0);
        }
    }
    Ok(js.undefined())
}

fn yaml_parse<'js>(
    ctx: Ctx<'js>,
    input: Opt<Value<'js>>,
    options: Opt<Value<'js>>,
) -> rquickjs::Result<Value<'js>> {
    let js = Js::new(ctx);
    let input = js.json_clone(input.0.unwrap_or_else(|| js.undefined()))?;
    let options = options.0.unwrap_or_else(|| js.undefined());
    let defaults = js.ctx.json_parse(r#"{"logLevel":"error","merge":true}"#)?;
    let options: Value = js.call("spread", (defaults, options))?;
    let utilities: Object = js.ctx.globals().get("SubStoreUtilities")?;
    let parse: Function = utilities.get("parseYaml")?;
    match parse.call((input.clone(), options.clone())) {
        Ok(value) => Ok(value),
        Err(_) => {
            js.ctx.catch();
            let expression: Value = js.call("regex", (r"!<str>\s*", "g"))?;
            let input = js.method(
                input,
                "replace",
                vec![expression, js.value("__SubStoreJSYAMLString__")?],
            )?;
            let value = parse.call((input, options.clone()))?;
            let text = yaml_dump(js.ctx.clone(), Opt(Some(value)), Opt(None))?;
            let expression: Value = js.call("regex", ("__SubStoreJSYAMLString__", "g"))?;
            let text = js.method(text, "replace", vec![expression, js.value("")?])?;
            parse.call((text, options))
        }
    }
}

fn yaml_dump<'js>(
    ctx: Ctx<'js>,
    input: Opt<Value<'js>>,
    options: Opt<Value<'js>>,
) -> rquickjs::Result<Value<'js>> {
    let js = Js::new(ctx);
    let input = js.json_clone(input.0.unwrap_or_else(|| js.undefined()))?;
    let options = options.0.unwrap_or_else(|| js.undefined());
    let options = if js.truthy(&options)? {
        options
    } else {
        Object::new(js.ctx.clone())?.into_value()
    };
    let force = js.get(&options, "forceQuotes")?;
    let width = js.get(&options, "lineWidth")?;
    let indent = js.get(&options, "noArrayIndent")?;
    let refs = js.get(&options, "noRefs")?;
    let quoting = js.get(&options, "quotingType")?;
    let sort = js.get(&options, "sortKeys")?;
    let excluded = js.array(
        [
            "forceQuotes",
            "lineWidth",
            "noArrayIndent",
            "noRefs",
            "quotingType",
            "sortKeys",
        ]
        .map(|key| js.value(key))
        .into_iter()
        .collect::<rquickjs::Result<Vec<_>>>()?,
    )?;
    let normalized: Value = js.call("except", (options, excluded))?;
    if width.is_number() {
        let number: f64 = js.call("number", (width.clone(),))?;
        js.set(
            &normalized,
            "lineWidth",
            if number <= 0.0 { js.value(0)? } else { width },
        )?;
    }
    for (value, key) in [(indent, "indentSeq"), (refs, "aliasDuplicateObjects")] {
        if value.is_bool() {
            js.set(&normalized, key, js.value(!js.truthy(&value)?)?)?;
        }
    }
    if !sort.is_undefined() {
        js.set(&normalized, "sortMapEntries", sort)?;
    }
    if js.equal(&quoting, &js.value("'")?)? {
        js.set(&normalized, "singleQuote", js.value(true)?)?;
    } else if js.equal(&quoting, &js.value("\"")?)? {
        js.set(&normalized, "singleQuote", js.value(false)?)?;
    }
    if js.truthy(&force)? {
        let kind = if js.equal(&quoting, &js.value("'")?)? {
            "QUOTE_SINGLE"
        } else {
            "QUOTE_DOUBLE"
        };
        js.set(&normalized, "defaultStringType", js.value(kind)?)?;
    }
    let utilities: Object = js.ctx.globals().get("SubStoreUtilities")?;
    let stringify: Function = utilities.get("stringifyYaml")?;
    stringify.call((input, normalized))
}

fn normalized_ttl<'js>(js: &Js<'js>, value: Value<'js>) -> rquickjs::Result<Option<f64>> {
    let value: f64 = js.call("number", (value,))?;
    Ok((value.is_finite() && value > 0.0).then_some(value))
}

fn host<'js>(js: &Js<'js>) -> rquickjs::Result<Value<'js>> {
    js.ctx.globals().get("$substore")
}

fn cache_persist<'js>(ctx: Ctx<'js>, This(this): This<Value<'js>>) -> rquickjs::Result<()> {
    let js = Js::new(ctx);
    let value = js.get(&this, "resourceCache")?;
    let text = js
        .ctx
        .json_stringify(value)?
        .map_or_else(|| js.undefined(), |text| text.into_value());
    js.method(host(&js)?, "write", vec![text, js.value(CACHE_KEY)?])?;
    Ok(())
}

fn cache_cleanup<'js>(
    ctx: Ctx<'js>,
    This(this): This<Value<'js>>,
    prefix: Opt<Value<'js>>,
    ttl: Opt<Value<'js>>,
) -> rquickjs::Result<()> {
    let js = Js::new(ctx);
    let ttl = normalized_ttl(&js, ttl.0.unwrap_or_else(|| js.undefined()))?.unwrap_or(0.0);
    let now: Value = js.call("now", ())?;
    let threshold: Value = js.call("add", (now, ttl))?;
    let cache = js.get(&this, "resourceCache")?;
    let entries: Array = js.call("entries", (cache.clone(),))?;
    let prefix = prefix.0.unwrap_or_else(|| js.undefined());
    let mut clear = false;
    for entry in entries.iter::<Array>() {
        let entry = entry?;
        let id: Value = entry.get(0)?;
        let cached: Value = entry.get(1)?;
        let time = js.get(&cached, "time")?;
        let expired = !js.truthy(&time)? || js.call::<bool, _>("lt", (time, threshold.clone()))?;
        if expired
            && (!js.truthy(&prefix)?
                || js.truthy(&js.method(id.clone(), "startsWith", vec![prefix.clone()])?)?)
        {
            js.call::<(), _>("remove", (cache.clone(), id))?;
            clear = true;
        }
    }
    if clear {
        cache_persist(js.ctx.clone(), This(this))?;
    }
    Ok(())
}

fn cache_gettime<'js>(
    ctx: Ctx<'js>,
    This(this): This<Value<'js>>,
    id: Opt<Value<'js>>,
) -> rquickjs::Result<Value<'js>> {
    let js = Js::new(ctx);
    let cache = js.get(&this, "resourceCache")?;
    let cached: Value = js.call("get", (cache, id.0.unwrap_or_else(|| js.undefined())))?;
    if js.truthy(&cached)? {
        let time = js.get(&cached, "time")?;
        if js.truthy(&time)? {
            let now: Value = js.call("dateTime", ())?;
            if js.call::<bool, _>("le", (now, time))? {
                return js.get(&cached, "time");
            }
        }
    }
    Ok(Value::new_null(js.ctx.clone()))
}

fn cache_get<'js>(
    ctx: Ctx<'js>,
    This(this): This<Value<'js>>,
    id: Opt<Value<'js>>,
    ttl: Opt<Value<'js>>,
    remove: Opt<Value<'js>>,
) -> rquickjs::Result<Value<'js>> {
    let js = Js::new(ctx);
    let ttl = normalized_ttl(&js, ttl.0.unwrap_or_else(|| js.undefined()))?.unwrap_or(0.0);
    let cache = js.get(&this, "resourceCache")?;
    let id = id.0.unwrap_or_else(|| js.undefined());
    let cached: Value = js.call("get", (cache.clone(), id.clone()))?;
    if js.truthy(&cached)? {
        let time = js.get(&cached, "time")?;
        if js.truthy(&time)? {
            let now: Value = js.call("now", ())?;
            let threshold: Value = js.call("add", (now, ttl))?;
            if js.call::<bool, _>("le", (threshold, time))? {
                return js.get(&cached, "data");
            }
            if js.truthy(&remove.0.unwrap_or_else(|| js.undefined()))? {
                js.call::<(), _>("remove", (cache, id))?;
                cache_persist(js.ctx.clone(), This(this))?;
            }
        }
    }
    Ok(Value::new_null(js.ctx.clone()))
}

fn cache_set<'js>(
    ctx: Ctx<'js>,
    This(this): This<Value<'js>>,
    id: Opt<Value<'js>>,
    value: Opt<Value<'js>>,
    ttl: Opt<Value<'js>>,
) -> rquickjs::Result<()> {
    let js = Js::new(ctx);
    let ttl = if let Some(ttl) = normalized_ttl(&js, ttl.0.unwrap_or_else(|| js.undefined()))? {
        ttl
    } else {
        let settings = js.method(host(&js)?, "read", vec![js.value("settings")?])?;
        let setting: Value = js.call("optional", (settings, "scriptCacheTtl"))?;
        normalized_ttl(&js, setting)?
            .map(|seconds| seconds * 1000.0)
            .unwrap_or(172800000.0)
    };
    let now: Value = js.call("now", ())?;
    let time: Value = js.call("add", (now, ttl))?;
    let cached = Object::new(js.ctx.clone())?;
    cached.set("time", time)?;
    cached.set("data", value.0.unwrap_or_else(|| js.undefined()))?;
    let cache = js.get(&this, "resourceCache")?;
    js.call::<(), _>(
        "set",
        (cache, id.0.unwrap_or_else(|| js.undefined()), cached),
    )?;
    cache_persist(js.ctx.clone(), This(this))
}

pub fn initialize<'js>(ctx: &Ctx<'js>, utils: &Object<'js>) -> rquickjs::Result<()> {
    for (name, pattern) in [("isIPv4", IPV4), ("isIPv6", IPV6)] {
        utils.set(
            name,
            Func::from(move |ctx: Ctx<'js>, value: Opt<Value<'js>>| {
                let js = Js::new(ctx);
                matches(&js, pattern, value.0.unwrap_or_else(|| js.undefined()))
            }),
        )?;
    }
    utils.set(
        "isIP",
        Func::from(|ctx: Ctx<'js>, value: Opt<Value<'js>>| {
            let js = Js::new(ctx);
            let value = value.0.unwrap_or_else(|| js.undefined());
            Ok::<_, rquickjs::Error>(
                matches(&js, IPV4, value.clone())? || matches(&js, IPV6, value)?,
            )
        }),
    )?;
    utils.set(
        "isValidUUID",
        Func::from(|ctx: Ctx<'js>, value: Opt<Value<'js>>| {
            let js = Js::new(ctx);
            let value = value.0.unwrap_or_else(|| js.undefined());
            Ok::<_, rquickjs::Error>(value.is_string() && matches(&js, UUID, value)?)
        }),
    )?;
    utils.set("getRandomPort", Func::from(random_port))?;
    utils.set(
        "getFlag",
        Func::from(|ctx: Ctx<'js>, name: Opt<Value<'js>>| {
            let js = Js::new(ctx);
            js_language::get_flag(&js, name.0.unwrap_or_else(|| js.undefined()))
        }),
    )?;
    utils.set(
        "removeFlag",
        Func::from(|ctx: Ctx<'js>, name: Opt<Value<'js>>| {
            let js = Js::new(ctx);
            js_language::remove_flag(&js, name.0.unwrap_or_else(|| js.undefined()))
        }),
    )?;
    utils.set("getISO", Func::from(get_iso))?;
    let yaml = Object::new(ctx.clone())?;
    let load = Function::new(ctx.clone(), yaml_parse)?;
    let dump = Function::new(ctx.clone(), yaml_dump)?;
    yaml.set("safeLoad", load.clone())?;
    yaml.set("parse", load)?;
    yaml.set("load", Func::from(yaml_parse))?;
    yaml.set("safeDump", dump.clone())?;
    yaml.set("stringify", dump)?;
    yaml.set("dump", Func::from(yaml_dump))?;
    utils.set("yaml", yaml)?;

    let js = Js::new(ctx.clone());
    let cache = Object::new(ctx.clone())?;
    cache.set("_persist", Func::from(cache_persist))?;
    cache.set("_cleanup", Func::from(cache_cleanup))?;
    cache.set("gettime", Func::from(cache_gettime))?;
    cache.set("get", Func::from(cache_get))?;
    cache.set("set", Func::from(cache_set))?;
    cache.set(
        "revokeAll",
        Func::from(|ctx: Ctx<'js>, This(this): This<Value<'js>>| {
            let js = Js::new(ctx);
            js.set(
                &this,
                "resourceCache",
                Object::new(js.ctx.clone())?.into_value(),
            )?;
            cache_persist(js.ctx.clone(), This(this))
        }),
    )?;
    let mut stored = js.method(host(&js)?, "read", vec![js.value(CACHE_KEY)?])?;
    if !js.truthy(&stored)? {
        stored = js.value("{}")?;
        js.method(
            host(&js)?,
            "write",
            vec![stored.clone(), js.value(CACHE_KEY)?],
        )?;
    }
    let value: Value = match js.call("jsonParse", (stored,)) {
        Ok(value) => value,
        Err(_) => {
            ctx.catch();
            js.method(
                host(&js)?,
                "write",
                vec![js.value("{}")?, js.value(CACHE_KEY)?],
            )?;
            Object::new(ctx.clone())?.into_value()
        }
    };
    cache.set("resourceCache", value)?;
    cache_cleanup(ctx.clone(), This(cache.clone().into_value()), Opt(None), Opt(None))?;
    ctx.globals().set("scriptResourceCache", cache)
}
