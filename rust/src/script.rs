//! Owned application script jobs and thin native bindings.
use crate::{conversion, js_language::Js, json as codec, processors, script_io, state::Store};
use rquickjs::{
    AsyncContext, AsyncRuntime, Ctx, Exception, Function, JsLifetime, Object, Promise,
    Value as JsValue,
    function::Args,
    object::Accessor,
    prelude::{Async, Func, Opt, Rest, This},
};
use serde_json::{Value, json};
use std::{
    cell::RefCell,
    rc::Rc,
    sync::{Arc, Mutex},
    time::Duration,
};

#[derive(Clone)]
struct Job {
    store: Arc<Mutex<Store>>,
    baseline: Rc<RefCell<(Value, Value)>>,
}

// Job contains only owned Rust state. It never contains a JS handle or borrow;
// changing the engine lifetime therefore does not change this type.
unsafe impl<'js> JsLifetime<'js> for Job {
    type Changed<'to> = Job;
}

fn job(ctx: &Ctx<'_>) -> rquickjs::Result<Job> {
    ctx.userdata::<Job>()
        .map(|data| data.clone())
        .ok_or_else(|| Exception::throw_message(ctx, "Missing script job"))
}

pub(crate) fn message(ctx: &Ctx<'_>, error: rquickjs::Error) -> String {
    if !error.is_exception() {
        return error.to_string();
    }
    let value = ctx.catch();
    let js = Js::new(ctx.clone());
    let value = value
        .as_object()
        .and_then(|object| object.get::<_, JsValue>("message").ok())
        .filter(|value| !value.is_null() && !value.is_undefined())
        .unwrap_or(value);
    let text = js.text(value).unwrap_or_else(|_| {
        ctx.catch();
        "Script execution failed".to_owned()
    });
    // QuickJS quotes identifiers in ReferenceError messages. Existing processor
    // fallback tests inspect these source names, rather than an engine stack.
    [
        "$server",
        "$content",
        "$files",
        "$res",
        "operator",
        "filter",
        "transformFunction",
    ]
    .iter()
    .fold(text, |text, name| {
        text.replace(
            &format!("'{name}' is not defined"),
            &format!("{name} is not defined"),
        )
    })
}

pub(crate) fn serialized<'js>(ctx: &Ctx<'js>, value: JsValue<'js>) -> rquickjs::Result<String> {
    ctx.json_stringify(value)?
        .ok_or_else(|| Exception::throw_type(ctx, "Value is not JSON serializable"))?
        .to_string()
}

fn store_key<'js>(js: &Js<'js>, key: JsValue<'js>) -> rquickjs::Result<(bool, JsValue<'js>)> {
    if !key.is_string() {
        return Err(Exception::throw_type(
            &js.ctx,
            "t.indexOf is not a function",
        ));
    }
    let index = js.method(key.clone(), "indexOf", vec![js.value("#")?])?;
    let root = !js.equal(&index, &js.value(-1)?)?;
    Ok((
        root,
        if root {
            js.method(key, "substr", vec![js.value(1)?])?
        } else {
            key
        },
    ))
}

fn refresh<'js>(
    js: &Js<'js>,
    this: &JsValue<'js>,
    root: bool,
    key: &JsValue<'js>,
) -> rquickjs::Result<()> {
    let job = job(&js.ctx)?;
    let encoded = serialized(&js.ctx, key.clone())?;
    let key_text = codec::parse(&encoded).map_err(|error| js.error(&error.to_string()))?;
    let key_text = key_text
        .as_str()
        .ok_or_else(|| js.error("Storage key must be a string"))?;
    let baseline = {
        let old = job.baseline.borrow();
        if root { old.1.clone() } else { old.0.clone() }
    };
    let fresh = {
        let store = job
            .store
            .lock()
            .map_err(|_| js.error("State lock poisoned"))?;
        if root {
            store
                .script_root()
                .map_err(|error| js.error(&error.to_string()))?
        } else {
            store.data.clone()
        }
    };
    let before = baseline.get(key_text);
    let after = fresh.get(key_text);
    if before.map(codec::stringify) == after.map(codec::stringify) {
        return Ok(());
    }
    let cache = js.get(this, if root { "root" } else { "cache" })?;
    let own: bool = js.call("own", (cache.clone(), key.clone()))?;
    if own != before.is_some() {
        return Ok(());
    }
    if let Some(before) = before {
        let local: JsValue = js.call("get", (cache.clone(), key.clone()))?;
        let expected = js.ctx.json_parse(codec::stringify(before))?;
        if !js.call::<bool, _>("sameType", (local.clone(), expected))? {
            return Ok(());
        }
        let local = match js.ctx.json_stringify(local) {
            Ok(Some(local)) => local.to_string()?,
            Ok(None) => return Ok(()),
            Err(_) => {
                js.ctx.catch();
                return Ok(());
            }
        };
        if local != codec::stringify(before) {
            return Ok(());
        }
    }
    if let Some(after) = after {
        let value = js.ctx.json_parse(codec::stringify(after))?;
        js.call::<(), _>("set", (cache, key.clone(), value))?;
    } else {
        js.call::<(), _>("remove", (cache, key.clone()))?;
    }
    let mut baseline = job.baseline.borrow_mut();
    let baseline = if root {
        &mut baseline.1
    } else {
        &mut baseline.0
    };
    if let Some(map) = baseline.as_object_mut() {
        if let Some(after) = after {
            map.insert(key_text.to_owned(), after.clone());
        } else {
            map.shift_remove(key_text);
        }
    }
    Ok(())
}

fn read<'js>(
    ctx: Ctx<'js>,
    This(this): This<JsValue<'js>>,
    key: JsValue<'js>,
) -> rquickjs::Result<JsValue<'js>> {
    let js = Js::new(ctx);
    let (root, key) = store_key(&js, key)?;
    refresh(&js, &this, root, &key)?;
    let cache = js.get(&this, if root { "root" } else { "cache" })?;
    js.call("get", (cache, key))
}

fn persist<'js>(ctx: Ctx<'js>, This(this): This<JsValue<'js>>) -> rquickjs::Result<()> {
    let js = Js::new(ctx);
    let job = job(&js.ctx)?;
    for root in [false, true] {
        // Source writes cache first, then stringifies/writes root. A later
        // failure must not roll back an already completed explicit cache write.
        let field = if root { "root" } else { "cache" };
        let next = serialized(&js.ctx, js.get(&this, field)?)?;
        let next = codec::parse(&next).map_err(|error| js.error(&error.to_string()))?;
        let baseline = {
            let old = job.baseline.borrow();
            if root { old.1.clone() } else { old.0.clone() }
        };
        job.store
            .lock()
            .map_err(|_| js.error("State lock poisoned"))?
            .script_commit(root, &baseline, &next)
            .map_err(|error| js.error(&error.to_string()))?;
        let mut baseline = job.baseline.borrow_mut();
        if root {
            baseline.1 = next;
        } else {
            baseline.0 = next;
        }
    }
    Ok(())
}

fn write<'js>(
    ctx: Ctx<'js>,
    This(this): This<JsValue<'js>>,
    value: JsValue<'js>,
    key: JsValue<'js>,
) -> rquickjs::Result<()> {
    let js = Js::new(ctx);
    let (root, key) = store_key(&js, key)?;
    let cache = js.get(&this, if root { "root" } else { "cache" })?;
    js.call::<(), _>("set", (cache, key, value))?;
    js.method(this, "persistCache", vec![])?;
    Ok(())
}

fn delete<'js>(
    ctx: Ctx<'js>,
    This(this): This<JsValue<'js>>,
    key: JsValue<'js>,
) -> rquickjs::Result<()> {
    let js = Js::new(ctx);
    let (root, key) = store_key(&js, key)?;
    let cache = js.get(&this, if root { "root" } else { "cache" })?;
    js.call::<(), _>("remove", (cache, key))?;
    js.method(this, "persistCache", vec![])?;
    Ok(())
}

async fn wait(milliseconds: u64) {
    tokio::time::sleep(Duration::from_millis(milliseconds)).await;
}

fn native_parse<'js>(ctx: Ctx<'js>, raw: JsValue<'js>) -> rquickjs::Result<JsValue<'js>> {
    if !raw.is_string() {
        return Err(Exception::throw_type(&ctx, "raw.split is not a function"));
    }
    let js = Js::new(ctx);
    let raw = js.text(raw)?;
    let proxies = conversion::parse(&json!(codec::encode(&raw)))
        .map_err(|error| js.error(&conversion_message(error)))?;
    js.ctx.json_parse(codec::stringify(&json!(proxies)))
}

fn conversion_message(error: conversion::Error) -> String {
    match error {
        conversion::Error::Legacy(message) => message,
        conversion::Error::Pending(message) => message.to_owned(),
    }
}

#[derive(Clone)]
struct Scope<'js> {
    target: JsValue<'js>,
    source: JsValue<'js>,
    options: JsValue<'js>,
    context: JsValue<'js>,
    execution: JsValue<'js>,
}

pub(crate) async fn settled<'js>(
    js: &Js<'js>,
    value: JsValue<'js>,
) -> rquickjs::Result<JsValue<'js>> {
    let promise: Promise = js.call("resolve", (value,))?;
    promise.into_future().await
}

async fn invoke<'js>(
    js: &Js<'js>,
    entry: &str,
    script: &str,
    arguments: JsValue<'js>,
    scope: &Scope<'js>,
    input: JsValue<'js>,
) -> rquickjs::Result<JsValue<'js>> {
    let constructor: Function = js.ctx.globals().get("Function")?;
    let factory: Function = constructor.call(("$arguments,$options,$substore,lodash,ProxyUtils,yaml,Buffer,b64d,b64e,DOMAIN_RESOLVERS,scriptResourceCache,flowUtils,produceArtifact,require", format!("{script}\nreturn {entry}")))?;
    let globals = js.ctx.globals();
    let utils: Object = globals.get("ProxyUtils")?;
    let base64: Object = utils.get("Base64")?;
    let mut args = Args::new(js.ctx.clone(), 14);
    for value in [
        arguments,
        scope.options.clone(),
        globals.get("$substore")?,
        globals.get("lodash")?,
        utils.clone().into_value(),
        utils.get("yaml")?,
        utils.get("Buffer")?,
        base64.get("decode")?,
        base64.get("encode")?,
        globals.get("DOMAIN_RESOLVERS")?,
        globals.get("scriptResourceCache")?,
        globals.get("flowUtils")?,
        globals.get("produceArtifact")?,
        js.undefined(),
    ] {
        args.push_arg(value)?;
    }
    let function: Function = factory.call_arg(args)?;
    let output: JsValue = if entry == "transformFunction" {
        function.call((input, scope.context.clone()))?
    } else {
        function.call((input, scope.target.clone(), scope.context.clone()))?
    };
    settled(js, output).await
}

async fn apply_script<'js>(
    js: &Js<'js>,
    name: &str,
    item: JsValue<'js>,
    input: JsValue<'js>,
    scope: &Scope<'js>,
) -> rquickjs::Result<JsValue<'js>> {
    let (script, arguments) = crate::script_loading::load(js, &item, &scope.execution).await?;
    let script = js.text(script)?;
    js.set(&scope.context, "source", scope.source.clone())?;
    js.set(
        &scope.context,
        "env",
        js.ctx.globals().get("__scriptEnvironment")?,
    )?;
    let filter = name == "Script Filter";
    let entry = if filter { "filter" } else { "operator" };
    let mut output = if filter {
        input.clone()
    } else {
        js.json_clone(input.clone())?
    };
    if !filter
        && crate::script_pipeline::mihomo(
            js,
            &js.call::<JsValue, _>("optional", (output.clone(), "$file"))?,
        )?
    {
        match crate::script_pipeline::patch(js, &output, js.value(script.clone())?) {
            Ok(value) => return Ok(value),
            Err(_) => {
                js.ctx.catch();
            }
        }
    }
    let first = invoke(js, entry, &script, arguments.clone(), scope, output.clone()).await;
    match first {
        Ok(result) => {
            if filter || js.truthy(&result)? {
                output = result;
            }
        }
        Err(error) => {
            let first = message(&js.ctx, error);
            let file: JsValue = js.call("optional", (output.clone(), "$files"))?;
            let content: JsValue = js.call("optional", (output.clone(), "$content"))?;
            let suppressed = first.contains("$server is not defined")
                || (!filter
                    && (first.contains("$content is not defined")
                        || first.contains("$files is not defined")
                        || js.truthy(&file)?
                        || js.truthy(&content)?));
            let shortcut = if filter {
                format!(
                    "async function filter(input = [], targetPlatform, context) {{ let list = []; const fn = async ($server) => {{ {script}\n }}; for await (let $server of input) list.push(await fn($server)); return list; }}"
                )
            } else {
                let main = if crate::script_pipeline::mihomo(
                    js,
                    &js.call::<JsValue, _>("optional", (output.clone(), "$file"))?,
                )? {
                    "if (typeof main === 'function') $content = await __fileMain(main, $content);"
                } else {
                    ""
                };
                format!(
                    "async function operator(input = [], targetPlatform, context) {{ if (input && (input.$files || input.$content)) {{ let {{$content,$files,$options,$file}} = input; {script}\n {main} return {{$content,$files,$options,$file}}; }} let list = []; for await (let $server of input) {{ {script}\n list.push($server); }} return list; }}"
                )
            };
            match invoke(js, entry, &shortcut, arguments, scope, output.clone()).await {
                Ok(result) => {
                    if filter || js.truthy(&result)? {
                        output = result;
                    }
                }
                Err(error) => {
                    let second = message(&js.ctx, error);
                    let (prefix, shortcut_label, label) = if filter {
                        (
                            "执行 function filter 失败",
                            "执行快捷过滤脚本 失败",
                            "脚本过滤",
                        )
                    } else {
                        (
                            "执行 function operator 失败",
                            "执行快捷脚本 失败",
                            "脚本操作",
                        )
                    };
                    let detail = if !suppressed && first == second {
                        format!("执行失败 {first}")
                    } else {
                        format!(
                            "{}{shortcut_label} {second}",
                            if suppressed {
                                String::new()
                            } else {
                                format!("{prefix} {first}; ")
                            }
                        )
                    };
                    return Err(js.error(&format!("{label} {detail}")));
                }
            }
        }
    }
    if filter {
        let scope = js.clone();
        js.filter(input, move |_, index| {
            let selection: JsValue = scope.call("get", (output.clone(), index))?;
            scope.truthy(&selection)
        })
    } else {
        Ok(output)
    }
}

fn skip<'js>(js: &Js<'js>, item: &JsValue<'js>, context: &JsValue<'js>) -> rquickjs::Result<bool> {
    let control = js.get(context, "process")?;
    if !js.truthy(&control)? || js.call::<String, _>("typeOf", (control.clone(),))? != "object" {
        return Ok(false);
    }
    let names = js.get(&control, "customNames")?;
    if !names.is_array() {
        return Ok(false);
    }
    let name = js.get(item, "customName")?;
    let matched = !name.is_undefined()
        && !name.is_null()
        && js.truthy(&js.method(names, "includes", vec![name])?)?;
    match js.text(js.get(&control, "type")?)?.as_str() {
        "disable" => Ok(matched),
        "enable" => Ok(!matched),
        _ => Ok(false),
    }
}

async fn process<'js>(
    js: &Js<'js>,
    mut input: JsValue<'js>,
    items: JsValue<'js>,
    scope: Scope<'js>,
) -> rquickjs::Result<JsValue<'js>> {
    let items: Vec<JsValue> = js.call("iterable", (items,))?;
    for item in items {
        let name = js.get(&item, "type")?;
        if js.equal(&name, &js.value("Response Transformer")?)?
            || skip(js, &item, &scope.context)?
            || js.truthy(&js.get(&item, "disabled")?)?
        {
            continue;
        }
        let script_index = js.method(name.clone(), "indexOf", vec![js.value("Script")?])?;
        let is_script = !js.equal(&script_index, &js.value(-1)?)?;
        let name = js.text(name)?;
        if is_script {
            if !["Script Operator", "Script Filter"].contains(&name.as_str()) {
                crate::script_loading::load(js, &item, &scope.execution).await?;
                eprintln!("[sub-store:error] Unknown operator: {name}");
                continue;
            }
            input = apply_script(js, &name, item, input, &scope).await?;
        } else if processors::known(&name) {
            let args = js.get(&item, "args")?;
            let args = if js.truthy(&args)? {
                args
            } else {
                Object::new(js.ctx.clone())?.into_value()
            };
            let args = processors::prepare(js, &name, args)?;
            let output = if name.contains("Operator") {
                js.json_clone(input)?
            } else {
                input
            };
            input = match processors::apply(js, &name, output.clone(), args) {
                Ok(result) => {
                    if js.truthy(&result)? {
                        result
                    } else {
                        output
                    }
                }
                Err(error) => {
                    let reason = message(&js.ctx, error);
                    let (label, entry, fallback) = if name.contains("Filter") {
                        ("脚本过滤", "filter", "filter.nodeFunc is not a function")
                    } else {
                        (
                            "脚本操作",
                            "operator",
                            "operator.nodeFunc is not a function",
                        )
                    };
                    return Err(js.error(&format!("{label} 执行 function {entry} 失败 {reason}; 执行快捷{}脚本 失败 {fallback}", if name.contains("Filter") { "过滤" } else { "" })));
                }
            };
        } else if name == "Add Proxies From Subscription Operator" {
            let output = js.json_clone(input)?;
            let args = crate::script_network::prefer(
                js,
                [
                    js.get(&item, "args")?,
                    Object::new(js.ctx.clone())?.into_value(),
                ],
            )?;
            match crate::script_pipeline::add(
                js,
                output.clone(),
                args.clone(),
                scope.execution.clone(),
            )
            .await
            {
                Ok(value) => input = value,
                Err(error) => {
                    let first = message(&js.ctx, error);
                    match crate::script_pipeline::add(js, output, args, scope.execution.clone())
                        .await
                    {
                        Ok(value) => input = value,
                        Err(error) => {
                            let second = message(&js.ctx, error);
                            let detail = if first == second {
                                format!("执行失败 {first}")
                            } else {
                                format!(
                                    "执行 function operator 失败 {first}; 执行快捷脚本 失败 {second}"
                                )
                            };
                            return Err(js.error(&format!("脚本操作 {detail}")));
                        }
                    }
                }
            }
        } else if name == "Resolve Domain Operator" {
            let args = crate::script_network::prefer(js, [js.get(&item, "args")?, Object::new(js.ctx.clone())?.into_value()])?;
            input = crate::script_dns::apply(js, input, args).await?;
        }
    }
    Ok(input)
}

async fn native_process<'js>(
    ctx: Ctx<'js>,
    Rest(arguments): Rest<JsValue<'js>>,
) -> rquickjs::Result<JsValue<'js>> {
    let js = Js::new(ctx);
    let argument = |index| {
        arguments
            .get(index)
            .cloned()
            .unwrap_or_else(|| js.undefined())
    };
    let input = argument(0);
    let operators = Opt(Some(argument(1)));
    let target = Opt(Some(argument(2)));
    let source = Opt(Some(argument(3)));
    let options = Opt(Some(argument(4)));
    let raw = Opt(Some(argument(5)));
    let execution = Opt(Some(argument(6)));
    let context = Object::new(js.ctx.clone())?.into_value();
    if let Some(raw) = raw.0.filter(|value| !value.is_undefined()) {
        let is_object =
            !raw.is_null() && js.call::<String, _>("typeOf", (raw.clone(),))? == "object";
        js.set(
            &context,
            "raw",
            if raw.is_array() || is_object {
                raw
            } else {
                js.array([raw])?
            },
        )?;
    }
    let scope = Scope {
        target: target.0.unwrap_or_else(|| js.undefined()),
        source: source.0.unwrap_or_else(|| js.undefined()),
        options: options.0.unwrap_or_else(|| js.undefined()),
        context,
        execution: execution
            .0
            .filter(|value| !value.is_undefined())
            .unwrap_or(Object::new(js.ctx.clone())?.into_value()),
    };
    process(
        &js,
        input,
        operators
            .0
            .filter(|value| !value.is_undefined())
            .unwrap_or(js.array([])?),
        scope,
    )
    .await
}

fn response_headers<'js>(js: &Js<'js>, value: JsValue<'js>) -> rquickjs::Result<JsValue<'js>> {
    if !value.is_null() && js.call::<String, _>("typeOf", (value.clone(),))? == "object" {
        Ok(value)
    } else {
        Ok(Object::new(js.ctx.clone())?.into_value())
    }
}

fn normalize_response<'js>(js: &Js<'js>, input: JsValue<'js>) -> rquickjs::Result<JsValue<'js>> {
    let input = if input.is_undefined() {
        Object::new(js.ctx.clone())?.into_value()
    } else {
        input
    };
    let headers = crate::script_network::prefer(
        js,
        [
            js.get(&input, "header")?,
            js.get(&input, "headers")?,
            Object::new(js.ctx.clone())?.into_value(),
        ],
    )?;
    let headers = Rc::new(RefCell::new(response_headers(js, headers)?));
    let output = Object::new(js.ctx.clone())?;
    output.set(
        "status",
        crate::script_network::prefer(
            js,
            [
                js.get(&input, "status")?,
                js.get(&input, "statusCode")?,
                js.value(200)?,
            ],
        )?,
    )?;
    let own: bool = js.call("own", (input.clone(), "body"))?;
    output.set(
        "body",
        if own {
            js.get(&input, "body")?
        } else {
            js.value("")?
        },
    )?;
    for name in ["headers", "header"] {
        let get = headers.clone();
        let set = headers.clone();
        let scope = js.clone();
        output.prop(
            name,
            Accessor::new(
                move || get.borrow().clone(),
                move |value: JsValue<'js>| {
                    *set.borrow_mut() = response_headers(&scope, value)?;
                    Ok::<_, rquickjs::Error>(())
                },
            )
            .enumerable(),
        )?;
    }
    Ok(output.into_value())
}

async fn apply_transformer<'js>(
    js: &Js<'js>,
    item: JsValue<'js>,
    input: JsValue<'js>,
    scope: &Scope<'js>,
) -> rquickjs::Result<JsValue<'js>> {
    let (script, arguments) = crate::script_loading::load(js, &item, &scope.execution).await?;
    let script = js.text(script)?;
    js.set(&scope.context, "source", scope.source.clone())?;
    js.set(
        &scope.context,
        "env",
        js.ctx.globals().get("__scriptEnvironment")?,
    )?;
    match invoke(
        js,
        "transformFunction",
        &script,
        arguments.clone(),
        scope,
        input.clone(),
    )
    .await
    {
        Ok(value) => {
            if js.truthy(&value)? {
                Ok(value)
            } else {
                Ok(input)
            }
        }
        Err(error) => {
            let first = message(&js.ctx, error);
            let suppressed = first.contains("$res is not defined");
            let shortcut = format!(
                "async function transformFunction(res = {{}}, context) {{ let $res = res; {script}\n return $res; }}"
            );
            match invoke(
                js,
                "transformFunction",
                &shortcut,
                arguments,
                scope,
                input.clone(),
            )
            .await
            {
                Ok(value) => {
                    if js.truthy(&value)? {
                        Ok(value)
                    } else {
                        Ok(input)
                    }
                }
                Err(error) => {
                    let second = message(&js.ctx, error);
                    let detail = if !suppressed && first == second {
                        format!("执行失败 {first}")
                    } else {
                        format!(
                            "{}执行快捷脚本失败 {second}",
                            if suppressed {
                                String::new()
                            } else {
                                format!("执行 function transformFunction 失败 {first}; ")
                            }
                        )
                    };
                    Err(js.error(&format!("响应修改 {detail}")))
                }
            }
        }
    }
}

async fn process_response<'js>(
    ctx: Ctx<'js>,
    input: JsValue<'js>,
    operators: Opt<JsValue<'js>>,
    target: Opt<JsValue<'js>>,
    source: Opt<JsValue<'js>>,
    options: Opt<JsValue<'js>>,
    execution: Opt<JsValue<'js>>,
) -> rquickjs::Result<JsValue<'js>> {
    let js = Js::new(ctx);
    let scope = Scope {
        target: target.0.unwrap_or_else(|| js.undefined()),
        source: source.0.unwrap_or_else(|| js.undefined()),
        options: options.0.unwrap_or_else(|| js.undefined()),
        context: Object::new(js.ctx.clone())?.into_value(),
        execution: execution
            .0
            .filter(|value| !value.is_undefined())
            .unwrap_or(Object::new(js.ctx.clone())?.into_value()),
    };
    let items = operators
        .0
        .filter(|value| !value.is_undefined())
        .unwrap_or(js.array([])?);
    let items: Vec<JsValue> = js.call("iterable", (items,))?;
    let mut output = normalize_response(&js, input)?;
    for item in items {
        if !js.equal(&js.get(&item, "type")?, &js.value("Response Transformer")?)?
            || skip(&js, &item, &scope.context)?
            || js.truthy(&js.get(&item, "disabled")?)?
        {
            continue;
        }
        let result = apply_transformer(&js, item, output, &scope).await?;
        output = normalize_response(&js, result)?;
    }
    Ok(output)
}

fn initialize<'js>(ctx: &Ctx<'js>, job: Job) -> rquickjs::Result<()> {
    let (cache, root) = job.baseline.borrow().clone();
    ctx.store_userdata(job)
        .map_err(|_| Exception::throw_message(ctx, "Script job already borrowed"))?;
    let globals = ctx.globals();
    globals.set("__hostWait", Func::from(Async(wait)))?;
    globals.set("__hostTimer", Func::from(script_io::timer))?;
    globals.set("__hostCancelTimer", Func::from(script_io::cancel_timer))?;
    globals.set("__hostRequest", Func::from(Async(script_io::request)))?;
    globals.set(
        "__hostDownloadFile",
        Func::from(Async(script_io::download_file)),
    )?;
    globals.set("__hostReadFile", Func::from(script_io::read_file))?;
    globals.set("__hostRandomValues", Func::from(script_io::random_values))?;
    globals.set(
        "__hostLog",
        Func::from(|level: String, text: String| eprintln!("[sub-store:{level}] {text}")),
    )?;
    ctx.eval::<(), _>(include_bytes!("../vendor/utilities.js").as_slice())?;
    ctx.eval::<(), _>(include_bytes!("../vendor/script-utilities.js").as_slice())?;
    ctx.eval::<(), _>(include_bytes!("../vendor/hash-utilities.js").as_slice())?;
    ctx.eval::<(), _>(include_str!("script-bootstrap.js"))?;
    ctx.eval::<(), _>(include_str!("script-primitives.js"))?;
    globals.set("__geo", ctx.json_parse(include_str!("geo-flags.json"))?)?;
    globals.set(
        "__scriptEnvironment",
        ctx.json_parse(r#"{"backend":"Rust","version":"2.42.4","feature":{},"meta":{}}"#)?,
    )?;
    let host = Object::new(ctx.clone())?;
    host.set("cache", ctx.json_parse(codec::stringify(&cache))?)?;
    host.set("root", ctx.json_parse(codec::stringify(&root))?)?;
    host.set("env", ctx.json_parse(r#"{"isNode":false,"isQX":false,"isLoon":false,"isSurge":false,"isStash":false,"isShadowRocket":false,"isLanceX":false,"isEgern":false,"isGUIforCores":false}"#)?)?;
    host.set("read", Func::from(read))?;
    host.set("write", Func::from(write))?;
    host.set("delete", Func::from(delete))?;
    host.set("persistCache", Func::from(persist))?;
    let wait: Function = ctx.eval("delay => new Promise(resolve => setTimeout(resolve, delay))")?;
    host.set("wait", wait)?;
    let http: Object = ctx.eval("createHostHTTP()")?;
    host.set("http", http)?;
    for level in ["log", "info", "warn", "error"] {
        let name = level.to_owned();
        host.set(
            level,
            Func::from(move |text: String| eprintln!("[sub-store:{name}] {text}")),
        )?;
    }
    host.set(
        "notify",
        Func::from(|title: String, subtitle: Opt<String>, text: Opt<String>| {
            eprintln!(
                "[sub-store:notify] {title}\n{}\n{}",
                subtitle.0.unwrap_or_default(),
                text.0.unwrap_or_default()
            )
        }),
    )?;
    globals.set("$substore", host)?;
    let utilities: Object = globals.get("SubStoreUtilities")?;
    let scripts: Object = globals.get("SubStoreScriptUtilities")?;
    let utils = Object::new(ctx.clone())?;
    utils.set("parse", Func::from(native_parse))?;
    utils.set("produce", Func::from(crate::script_conversion::produce))?;
    utils.set("process", Func::from(Async(native_process)))?;
    utils.set("processResponse", Func::from(Async(process_response)))?;
    utils.set("Buffer", scripts.get::<_, JsValue>("Buffer")?)?;
    utils.set("Base64", scripts.get::<_, JsValue>("Base64")?)?;
    utils.set("ipAddress", scripts.get::<_, JsValue>("ipAddress")?)?;
    utils.set("JSON5", utilities.get::<_, JsValue>("JSON5")?)?;
    crate::script_helpers::initialize(ctx, &utils)?;
    crate::script_flow::initialize(ctx)?;
    crate::script_download::initialize(ctx, &utils)?;
    crate::script_age::initialize(ctx, &utils)?;
    crate::script_mmdb::initialize(ctx, &utils)?;
    crate::script_dns::initialize(ctx, &utils)?;
    globals.set("ProxyUtils", utils)?;
    crate::script_pipeline::initialize(ctx)
}

pub async fn run(
    store: Arc<Mutex<Store>>,
    operation: &str,
    input: Value,
) -> Result<Value, conversion::Error> {
    let operation = operation.to_owned();
    let (sender, receiver) = tokio::sync::oneshot::channel();
    let worker = tokio::task::spawn_blocking(move || {
        let mut sender = Some(sender);
        let result = (|| {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|error| conversion::Error::Legacy(error.to_string()))?
                .block_on(async {
                    let (cache, root) = {
                        let store = store.lock().map_err(|_| {
                            conversion::Error::Legacy("State lock poisoned".to_owned())
                        })?;
                        (
                            store.data.clone(),
                            store
                                .script_root()
                                .map_err(|error| conversion::Error::Legacy(error.to_string()))?,
                        )
                    };
                    let job = Job {
                        store,
                        baseline: Rc::new(RefCell::new((cache, root))),
                    };
                    let runtime = AsyncRuntime::new()
                        .map_err(|error| conversion::Error::Legacy(error.to_string()))?;
                    let context = AsyncContext::full(&runtime)
                        .await
                        .map_err(|error| conversion::Error::Legacy(error.to_string()))?;
                    let result = context
                        .async_with(async |ctx| {
                            let result = async {
                                initialize(&ctx, job)?;
                                let js = Js::new(ctx.clone());
                                let input = ctx.json_parse(codec::stringify(&input))?;
                                let result = match operation.as_str() {
                                    "parse" => crate::script_http::parse(&js, input).await?,
                                    "preview-sub" | "preview-collection" => {
                                        crate::script_pipeline::preview(
                                            &js,
                                            if operation == "preview-collection" {
                                                "collection"
                                            } else {
                                                "sub"
                                            },
                                            js.get(&input, "item")?,
                                            js.get(&input, "target")?,
                                        )
                                        .await?
                                    }
                                    "artifact" => {
                                        let function: Function =
                                            ctx.globals().get("produceArtifact")?;
                                        settled(&js, function.call((input,))?).await?
                                    }
                                    "download-sub" | "download-collection" => {
                                        crate::script_http::download(
                                            &js,
                                            operation == "download-collection",
                                            js.get(&input, "item")?,
                                            js.get(&input, "options")?,
                                            js.get(&input, "request")?,
                                        )
                                        .await?
                                    }
                                    _ => return Err(js.error("Unknown script job operation")),
                                };
                                let text = serialized(&ctx, result)?;
                                codec::parse(&text).map_err(|error| js.error(&error.to_string()))
                            }
                            .await;
                            result.map_err(|error| conversion::Error::Legacy(message(&ctx, error)))
                        })
                        .await;
                    if let Some(sender) = sender.take() {
                        let _ = sender.send(result);
                    }
                    // Optimistic refreshes keep their context after the HTTP reply.
                    runtime.idle().await;
                    Ok::<_, conversion::Error>(())
                })
        })();
        if let Some(sender) = sender {
            let error = result.err().unwrap_or_else(|| {
                conversion::Error::Legacy("Script job produced no result".to_owned())
            });
            let _ = sender.send(Err(error));
        }
    });
    match receiver.await {
        Ok(result) => result,
        Err(_) => {
            worker
                .await
                .map_err(|error| conversion::Error::Legacy(error.to_string()))?;
            Err(conversion::Error::Legacy(
                "Script job ended without a result".to_owned(),
            ))
        }
    }
}
