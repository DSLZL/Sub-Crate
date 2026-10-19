//! Owned application script jobs and thin native bindings.
use crate::{conversion, js_language::Js, json as codec, processors, script_io, state::Store};
use rquickjs::{
    AsyncContext, AsyncRuntime, Ctx, Exception, Function, JsLifetime, Object, Promise,
    Value as JsValue,
    function::Args,
    prelude::{Async, Func, Opt, This},
};
use serde_json::{Value, json};
use std::{
    cell::{Cell, RefCell},
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

fn native_produce<'js>(
    ctx: Ctx<'js>,
    input: JsValue<'js>,
    target: JsValue<'js>,
    kind: Opt<JsValue<'js>>,
    options: Opt<JsValue<'js>>,
) -> rquickjs::Result<JsValue<'js>> {
    let js = Js::new(ctx);
    let input =
        codec::parse(&serialized(&js.ctx, input)?).map_err(|error| js.error(&error.to_string()))?;
    let input = input
        .as_array()
        .ok_or_else(|| Exception::throw_type(&js.ctx, "proxies.filter is not a function"))?
        .clone();
    let target = js.text(target)?;
    let kind = kind
        .0
        .map(|value| js.text(value))
        .transpose()?
        .unwrap_or_default();
    let options = options
        .0
        .filter(|value| !value.is_undefined())
        .map(|value| serialized(&js.ctx, value))
        .transpose()?
        .map(|text| codec::parse(&text))
        .transpose()
        .map_err(|error| js.error(&error.to_string()))?
        .unwrap_or_else(|| json!({}));
    let output = conversion::produce(input, &target, &kind, &options)
        .map_err(|error| js.error(&conversion_message(error)))?;
    js.ctx.json_parse(codec::stringify(&output))
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
}

pub(crate) async fn settled<'js>(js: &Js<'js>, value: JsValue<'js>) -> rquickjs::Result<JsValue<'js>> {
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
    let output: JsValue = function.call((input, scope.target.clone(), scope.context.clone()))?;
    settled(js, output).await
}

async fn apply_script<'js>(
    js: &Js<'js>,
    name: &str,
    item: JsValue<'js>,
    input: JsValue<'js>,
    scope: &Scope<'js>,
) -> rquickjs::Result<JsValue<'js>> {
    let args = js.get(&item, "args")?;
    let args = if js.truthy(&args)? {
        args
    } else {
        Object::new(js.ctx.clone())?.into_value()
    };
    if js.equal(&js.get(&args, "mode")?, &js.value("link")?)? {
        return Err(js.error("Linked script sources are not implemented yet"));
    }
    let script = js.text(js.get(&args, "content")?)?;
    let arguments = js.get(&args, "arguments")?;
    let arguments = if js.truthy(&arguments)? {
        arguments
    } else {
        Object::new(js.ctx.clone())?.into_value()
    };
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
                format!(
                    "async function operator(input = [], targetPlatform, context) {{ if (input && (input.$files || input.$content)) {{ let {{$content,$files,$options,$file}} = input; {script}\n return {{$content,$files,$options,$file}}; }} let list = []; for await (let $server of input) {{ {script}\n list.push($server); }} return list; }}"
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
                let args = js.get(&item, "args")?;
                if !args.is_null()
                    && !args.is_undefined()
                    && js.equal(&js.get(&args, "mode")?, &js.value("link")?)?
                {
                    return Err(js.error("Linked script sources are not implemented yet"));
                }
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
        } else if [
            "Resolve Domain Operator",
            "Add Proxies From Subscription Operator",
        ]
        .contains(&name.as_str())
        {
            return Err(js.error(&format!("{name} is not implemented yet")));
        }
    }
    Ok(input)
}

#[allow(clippy::too_many_arguments)] // Match the existing seven-argument public JS helper.
async fn native_process<'js>(
    ctx: Ctx<'js>,
    input: JsValue<'js>,
    operators: Opt<JsValue<'js>>,
    target: Opt<JsValue<'js>>,
    source: Opt<JsValue<'js>>,
    options: Opt<JsValue<'js>>,
    raw: Opt<JsValue<'js>>,
) -> rquickjs::Result<JsValue<'js>> {
    let js = Js::new(ctx);
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
    utils.set("produce", Func::from(native_produce))?;
    utils.set("process", Func::from(Async(native_process)))?;
    utils.set("Buffer", scripts.get::<_, JsValue>("Buffer")?)?;
    utils.set("Base64", scripts.get::<_, JsValue>("Base64")?)?;
    utils.set("ipAddress", scripts.get::<_, JsValue>("ipAddress")?)?;
    utils.set("JSON5", utilities.get::<_, JsValue>("JSON5")?)?;
    crate::script_helpers::initialize(ctx, &utils)?;
    crate::script_flow::initialize(ctx)?;
    crate::script_download::initialize(ctx, &utils)?;
    globals.set("ProxyUtils", utils)?;
    Ok(())
}

pub async fn local(
    store: Arc<Mutex<Store>>,
    sub: Value,
    target: String,
    content: Option<Value>,
    preview: bool,
) -> Result<Value, conversion::Error> {
    let (sender, receiver) = tokio::sync::oneshot::channel();
    let worker = tokio::task::spawn_blocking(move || {
        let mut sender = Some(sender);
        let result = (|| {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|error| conversion::Error::Legacy(error.to_string()))?
                .block_on(async {
                    let mut plain = sub.clone();
                    if let Some(fields) = plain.as_object_mut() {
                        fields.shift_remove("process");
                    }
                    let proxies = conversion::local(&plain, content.as_ref())?;
                    let (cache, root) = {
                        let store = store
                            .lock()
                            .map_err(|_| conversion::Error::Legacy("State lock poisoned".to_owned()))?;
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
                                let original = ctx.json_parse(codec::stringify(&json!(proxies)))?;
                                let sub_value = ctx.json_parse(codec::stringify(&sub))?;
                                let sub_name = js.get(&sub_value, "name")?;
                                let display_name = js.get(&sub_value, "displayName")?;
                                let next_index = Rc::new(Cell::new(0usize));
                                let tag_scope = js.clone();
                                let name_tag = sub_name.clone();
                                js.each(original.clone(), move |proxy| {
                                    if preview {
                                        tag_scope.set(
                                            &proxy,
                                            "id",
                                            tag_scope.value(next_index.get())?,
                                        )?;
                                        next_index.set(next_index.get() + 1);
                                    }
                                    tag_scope.set(&proxy, "_subName", name_tag.clone())?;
                                    tag_scope.set(&proxy, "_subDisplayName", display_name.clone())?;
                                    Ok(())
                                })?;
                                let items = sub
                                    .get("process")
                                    .filter(|value| crate::state::truthy(value))
                                    .cloned()
                                    .unwrap_or_else(|| json!([]));
                                let items = ctx.json_parse(codec::stringify(&items))?;
                                let source = Object::new(ctx.clone())?;
                                js.call::<(), _>("set", (source.clone(), sub_name, sub_value))?;
                                let context = Object::new(ctx.clone())?.into_value();
                                if let Some(raw) = content.as_ref().or_else(|| sub.get("content")) {
                                    let raw = ctx.json_parse(codec::stringify(raw))?;
                                    let raw = if raw.is_array() || (!raw.is_null() && raw.is_object()) {
                                        raw
                                    } else {
                                        js.array([raw])?
                                    };
                                    js.set(&context, "raw", raw)?;
                                }
                                let scope = Scope {
                                    target: js.value(codec::text(&target))?,
                                    source: source.into_value(),
                                    options: js.undefined(),
                                    context,
                                };
                                let processed = process(&js, original.clone(), items, scope).await?;
                                let result = Object::new(ctx.clone())?;
                                if preview {
                                    result.set("original", original)?;
                                }
                                result.set("processed", processed)?;
                                let text = serialized(&ctx, result.into_value())?;
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
