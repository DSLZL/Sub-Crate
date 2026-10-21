//! ECMAScript values at the native conversion/script boundary.
use crate::json as codec;
use rquickjs::{Array, Ctx, Function, Object, Value};

pub(crate) fn initialize(ctx: &Ctx<'_>) -> rquickjs::Result<()> {
    let functions: Object = ctx.eval(
        r#"({
      project: value => {
      const paths = [], parents = new WeakMap(); let root = true;
      const text = JSON.stringify(value, function(key, field) {
        const first = root; root = false;
        if (field && typeof field === 'object') {
          const path = first ? [] : [...parents.get(this), key];
          const ancestor = parents.get(field);
          if (ancestor && ancestor.length < path.length && ancestor.every((part,index) => part === path[index])) {
            paths.push([path,path.length-ancestor.length]); return null;
          }
          parents.set(field, path);
        } else if (typeof field === 'number' && (!Number.isFinite(field) || Object.is(field, -0))) {
          paths.push([first ? [] : [...parents.get(this), key], Object.is(field, -0) ? '-0' : String(field)]);
        }
        return field;
      });
      return paths.length === 0 && typeof text === 'string' ? text : [text, JSON.stringify(paths)];
      },
      stringify: (value,space) => JSON.stringify(value,null,space),
      assign: (root,path,value,depth) => {
        if(depth){value=root;for(const key of path.slice(0,path.length-depth))value=value[key];}
        let target=root;
        for(let i=0;i<path.length-1;i++) target=target[path[i]];
        target[path[path.length-1]]=value;
      }
    })"#,
    )?;
    ctx.globals().set("__nativeValues", functions)
}

fn function<'js>(ctx: &Ctx<'js>, name: &str) -> rquickjs::Result<Function<'js>> {
    let functions: Object = ctx.globals().get("__nativeValues")?;
    functions.get(name)
}

pub(crate) fn project<'js>(
    ctx: &Ctx<'js>,
    value: Value<'js>,
) -> rquickjs::Result<serde_json::Value> {
    let stringify = function(ctx, "project")?;
    let result: Value = stringify.call((value,))?;
    let (text, paths) = if let Some(text) = result.as_string() {
        (text.to_string()?, None)
    } else {
        let result = Array::from_value(result)?;
        let text: String = result.get(0)?;
        let paths: String = result.get(1)?;
        (text, Some(paths))
    };
    let mut value = codec::parse(&text)
        .map_err(|error| rquickjs::Exception::throw_message(ctx, &error.to_string()))?;
    let Some(paths) = paths else {
        return Ok(value);
    };
    let paths = codec::parse(&paths)
        .map_err(|error| rquickjs::Exception::throw_message(ctx, &error.to_string()))?;
    for entry in paths.as_array().unwrap() {
        let mut target = &mut value;
        for key in entry[0].as_array().unwrap() {
            let key = key.as_str().unwrap();
            target = if target.is_array() {
                target.get_mut(key.parse::<usize>().unwrap()).unwrap()
            } else {
                target.get_mut(key).unwrap()
            };
        }
        if let Some(depth) = entry[1].as_u64() {
            *target = serde_json::json!({"\u{e000}cycle":depth});
            continue;
        }
        *target = codec::numeric(match entry[1].as_str().unwrap() {
            "Infinity" => f64::INFINITY,
            "-Infinity" => f64::NEG_INFINITY,
            "-0" => -0.0,
            _ => f64::NAN,
        });
    }
    Ok(value)
}

fn cycle(value: &serde_json::Value) -> Option<usize> {
    let object = value.as_object()?;
    (object.len() == 1).then(|| {
        object
            .get("\u{e000}cycle")?
            .as_u64()
            .map(|depth| depth as usize)
    })?
}

pub(crate) fn circular_error(value: &serde_json::Value) -> Option<String> {
    fn find(value: &serde_json::Value, path: &mut Vec<String>) -> Option<(Vec<String>, usize)> {
        if let Some(depth) = cycle(value) {
            return Some((path.clone(), depth));
        }
        if let Some(values) = value.as_array() {
            for (index, value) in values.iter().enumerate() {
                path.push(index.to_string());
                if let Some(found) = find(value, path) {
                    return Some(found);
                }
                path.pop();
            }
        } else if let Some(values) = value.as_object() {
            for (key, value) in values {
                path.push(key.clone());
                if let Some(found) = find(value, path) {
                    return Some(found);
                }
                path.pop();
            }
        }
        None
    }
    let (path, depth) = find(value, &mut Vec::new())?;
    if depth > path.len() {
        return Some("Converting circular structure to JSON".to_owned());
    }
    let mut target = value;
    let mut message = "Converting circular structure to JSON\n    --> starting at ".to_owned();
    for key in &path[..path.len() - depth] {
        target = if target.is_array() {
            &target[key.parse::<usize>().ok()?]
        } else {
            &target[key]
        };
    }
    let description = |value: &serde_json::Value| {
        if value.is_array() {
            "object with constructor 'Array'"
        } else {
            "object with constructor 'Object'"
        }
    };
    message.push_str(description(target));
    for key in &path[path.len() - depth..path.len() - 1] {
        let label = if target.is_array() {
            format!("index {key}")
        } else {
            format!("property '{}'", codec::text(key))
        };
        target = if target.is_array() {
            &target[key.parse::<usize>().ok()?]
        } else {
            &target[key]
        };
        message.push_str(&format!("\n    |     {label} -> {}", description(target)));
    }
    let key = path.last()?;
    let label = if target.is_array() {
        format!("index {key}")
    } else {
        format!("property '{}'", codec::text(key))
    };
    message.push_str(&format!("\n    --- {label} closes the circle"));
    Some(message)
}

fn needs_restore(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::Number(number) => number
            .as_f64()
            .is_some_and(|number| number == 0.0 && number.is_sign_negative()),
        serde_json::Value::String(_) => codec::nonfinite(value).is_some(),
        serde_json::Value::Array(values) => values.iter().any(needs_restore),
        serde_json::Value::Object(values) => {
            cycle(value).is_some() || values.values().any(needs_restore)
        }
        _ => false,
    }
}

pub(crate) fn stringify<'js>(
    ctx: &Ctx<'js>,
    value: Value<'js>,
    space: u8,
) -> rquickjs::Result<Value<'js>> {
    let stringify = function(ctx, "stringify")?;
    match stringify.call((value.clone(), space)) {
        Ok(text) => Ok(text),
        Err(_) => {
            let original = ctx.catch();
            match project(ctx, value) {
                Ok(projected) => {
                    if let Some(message) = circular_error(&projected) {
                        return Err(rquickjs::Exception::throw_type(ctx, &message));
                    }
                }
                Err(_) => {
                    ctx.catch();
                }
            }
            Err(ctx.throw(original))
        }
    }
}

pub(crate) fn decode<'js>(
    ctx: &Ctx<'js>,
    value: &serde_json::Value,
) -> rquickjs::Result<Value<'js>> {
    if let Some(number) = codec::nonfinite(value).or_else(|| {
        value
            .as_f64()
            .filter(|number| *number == 0.0 && number.is_sign_negative())
    }) {
        return Ok(Value::new_float(ctx.clone(), number));
    }
    if !needs_restore(value) {
        // JSON.parse already restores ordinary numbers; only UTF-16 markers need decoding.
        return ctx.json_parse(codec::text(&value.to_string()));
    }
    fn wire(value: &serde_json::Value) -> serde_json::Value {
        if cycle(value).is_some() {
            return serde_json::Value::Null;
        }
        match value {
            serde_json::Value::Array(values) => {
                serde_json::Value::Array(values.iter().map(wire).collect())
            }
            serde_json::Value::Object(values) => serde_json::Value::Object(
                values
                    .iter()
                    .map(|(key, value)| (key.clone(), wire(value)))
                    .collect(),
            ),
            _ => value.clone(),
        }
    }
    let result = ctx.json_parse(codec::stringify(&wire(value)))?;
    let assign = function(ctx, "assign")?;
    fn visit<'js>(
        ctx: &Ctx<'js>,
        assign: &Function<'js>,
        root: &Value<'js>,
        value: &serde_json::Value,
        path: &mut Vec<String>,
    ) -> rquickjs::Result<()> {
        if let Some(depth) = cycle(value) {
            assign.call::<_, ()>((
                root.clone(),
                ctx.json_parse(codec::stringify(&serde_json::json!(path)))?,
                rquickjs::Value::new_null(ctx.clone()),
                depth,
            ))?;
        } else if let Some(number) = codec::nonfinite(value).or_else(|| {
            value
                .as_f64()
                .filter(|number| *number == 0.0 && number.is_sign_negative())
        }) {
            assign.call::<_, ()>((
                root.clone(),
                ctx.json_parse(codec::stringify(&serde_json::json!(path)))?,
                number,
            ))?;
        } else if let Some(values) = value.as_array() {
            for (index, value) in values.iter().enumerate() {
                path.push(index.to_string());
                visit(ctx, assign, root, value, path)?;
                path.pop();
            }
        } else if let Some(values) = value.as_object() {
            for (key, value) in values {
                path.push(key.clone());
                visit(ctx, assign, root, value, path)?;
                path.pop();
            }
        }
        Ok(())
    }
    visit(ctx, &assign, &result, value, &mut Vec::new())?;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_value_bridge_preserves_cycles_numbers_and_private_user_keys() {
        let runtime = rquickjs::Runtime::new().unwrap();
        let context = rquickjs::Context::full(&runtime).unwrap();
        context.with(|ctx| {
            initialize(&ctx).unwrap();
            let value:Value=ctx.eval("(() => {const value={extra:{},number:NaN,zero:-0,literal:{'\\ue000cycle':1}};value.extra.self=value.extra;return value;})() ").unwrap();
            let projected=project(&ctx,value).unwrap();
            assert_eq!(circular_error(&projected).as_deref(),Some("Converting circular structure to JSON\n    --> starting at object with constructor 'Object'\n    --- property 'self' closes the circle"));
            let restored=decode(&ctx,&projected).unwrap();
            ctx.globals().set("restored",restored.clone()).unwrap();
            assert!(ctx.eval::<bool,_>("restored.extra.self === restored.extra && Number.isNaN(restored.number) && Object.is(restored.zero,-0) && restored.literal['\\ue000cycle'] === 1").unwrap());
            assert!(stringify(&ctx,restored,2).is_err());
            let error=ctx.catch();
            assert_eq!(error.as_object().unwrap().get::<_,String>("message").unwrap(),circular_error(&projected).unwrap());
        });
    }

    #[test]
    fn repeated_projection_keeps_to_json_effects_and_context_isolation() {
        let runtime = rquickjs::Runtime::new().unwrap();
        for label in ["first", "second"] {
            let context = rquickjs::Context::full(&runtime).unwrap();
            context.with(|ctx| {
                initialize(&ctx).unwrap();
                ctx.globals().set("label", label).unwrap();
                let value: Value = ctx.eval("({calls:0,toJSON(){return {label,calls:++this.calls,zero:-0,number:Infinity};}})").unwrap();
                for calls in 1..=3 {
                    let projected = project(&ctx, value.clone()).unwrap();
                    assert_eq!(projected["label"], label);
                    assert_eq!(projected["calls"], calls);
                    assert!(codec::to_number(&projected["zero"]).is_sign_negative());
                    assert_eq!(
                        codec::nonfinite(&projected["number"]),
                        Some(f64::INFINITY)
                    );
                    let restored = decode(&ctx, &projected).unwrap();
                    let text = stringify(&ctx, restored, 0)
                        .unwrap()
                        .as_string()
                        .unwrap()
                        .to_string()
                        .unwrap();
                    assert_eq!(
                        codec::parse(&text).unwrap(),
                        serde_json::json!({"label":label,"calls":calls,"zero":0,"number":null})
                    );
                }
            });
        }
    }

    #[test]
    fn ordinary_projection_reads_current_getters_once_per_call() {
        let runtime = rquickjs::Runtime::new().unwrap();
        let context = rquickjs::Context::full(&runtime).unwrap();
        context.with(|ctx| {
            initialize(&ctx).unwrap();
            let source = r#"(() => {
                globalThis.reads = 0;
                return {toJSON() {
                    reads++;
                    return {get value() {return ++reads;}, nested: {ordinary: 0}};
                }};
            })()"#;
            let value: Value = ctx.eval(source).unwrap();
            for call in 1..=3 {
                let projected = project(&ctx, value.clone()).unwrap();
                assert_eq!(
                    projected,
                    serde_json::json!({"value":call * 2,"nested":{"ordinary":0}})
                );
                assert_eq!(ctx.eval::<u32, _>("reads").unwrap(), call * 2);
            }
        });
    }

    #[test]
    fn projection_distinguishes_shared_siblings_from_ancestor_cycles() {
        let runtime = rquickjs::Runtime::new().unwrap();
        let context = rquickjs::Context::full(&runtime).unwrap();
        context.with(|ctx| {
            initialize(&ctx).unwrap();
            let source = r#"(() => {
                const shared = {number: NaN, zero: -0};
                shared.self = shared;
                const root = {first: shared, branch: {child: shared}, last: shared};
                root.array = [shared, {root}, -Infinity];
                return root;
            })()"#;
            let value: Value = ctx.eval(source).unwrap();
            let projected = project(&ctx, value).unwrap();
            let restored = decode(&ctx, &projected).unwrap();
            ctx.globals().set("restored", restored).unwrap();
            let assertion = r#"(() => {
                const copies = [restored.first, restored.branch.child, restored.last, restored.array[0]];
                return copies.every(copy => copy.self === copy && Number.isNaN(copy.number) && Object.is(copy.zero, -0)) &&
                    copies.every((copy, index) => copies.slice(index + 1).every(other => copy !== other)) &&
                    restored.array[1].root === restored && restored.array[2] === -Infinity;
            })()"#;
            assert!(ctx.eval::<bool, _>(assertion).unwrap());
        });
    }

    #[test]
    fn projection_keeps_getter_order_and_recovers_after_a_throw() {
        let runtime = rquickjs::Runtime::new().unwrap();
        let context = rquickjs::Context::full(&runtime).unwrap();
        context.with(|ctx| {
            initialize(&ctx).unwrap();
            let source = r#"(() => {
                globalThis.events = [];
                const child = {toJSON(key) {
                    events.push('json:' + key);
                    return {get zero() {events.push('zero:' + key); return -0;}};
                }};
                return {get first() {events.push('first'); return child;},
                    get second() {events.push('second'); return child;},
                    get last() {events.push('last'); return Infinity;}};
            })()"#;
            let value: Value = ctx.eval(source).unwrap();
            let projected = project(&ctx, value).unwrap();
            assert_eq!(
                ctx.eval::<String, _>("events.join(',')").unwrap(),
                "first,json:first,zero:first,second,json:second,zero:second,last"
            );
            assert!(codec::to_number(&projected["first"]["zero"]).is_sign_negative());
            assert!(codec::to_number(&projected["second"]["zero"]).is_sign_negative());
            assert_eq!(codec::nonfinite(&projected["last"]), Some(f64::INFINITY));
            let source = "({nested: {get broken() {throw new Error('projection getter');}}})";
            let throwing: Value = ctx.eval(source).unwrap();
            assert!(project(&ctx, throwing).is_err());
            let error = ctx.catch();
            assert_eq!(
                error
                    .as_object()
                    .unwrap()
                    .get::<_, String>("message")
                    .unwrap(),
                "projection getter"
            );
            let value: Value = ctx.eval("({ok: {zero: -0}, after: [NaN]})").unwrap();
            let projected = project(&ctx, value).unwrap();
            assert!(codec::to_number(&projected["ok"]["zero"]).is_sign_negative());
            assert!(codec::nonfinite(&projected["after"][0]).unwrap().is_nan());
        });
    }

    #[test]
    fn restored_plain_values_keep_utf16_keys_and_js_number_semantics() {
        let runtime = rquickjs::Runtime::new().unwrap();
        let context = rquickjs::Context::full(&runtime).unwrap();
        context.with(|ctx| {
            initialize(&ctx).unwrap();
            let raw = r#"{"__proto__":{"literal":true},"\ue000cycle":"literal","\ud800":"\udfff","nul":"\u0000","values":[0,1.25,1e-7,1e21,18446744073709551615,-9223372036854775808,5e-324],"nested":["中🙂",{"false":false,"null":null}]}"#;
            let expected = ctx.json_parse(raw).unwrap();
            let restored = decode(&ctx, &codec::parse(raw).unwrap()).unwrap();
            let projected = project(&ctx, expected.clone()).unwrap();
            let projected = decode(&ctx, &projected).unwrap();
            ctx.globals().set("projected", projected).unwrap();
            ctx.globals().set("expected", expected).unwrap();
            ctx.globals().set("restored", restored).unwrap();
            let assertion = r#"
                JSON.stringify(restored) === JSON.stringify(expected) &&
                JSON.stringify(projected) === JSON.stringify(expected) &&
                Object.getPrototypeOf(restored) === Object.prototype &&
                Object.hasOwn(restored, '__proto__') && restored.__proto__.literal === true &&
                restored['\ue000cycle'] === 'literal' &&
                restored['\ud800'].charCodeAt(0) === 0xdfff && restored.nul.charCodeAt(0) === 0 &&
                restored.values.every((value, index) => Object.is(value, expected.values[index]))
            "#;
            assert!(ctx.eval::<bool, _>(assertion).unwrap());
        });
    }
}
