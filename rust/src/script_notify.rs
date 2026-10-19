//! Configured notification delivery through the existing native HTTP host.
use crate::{
    js_language::Js,
    script, script_network as network,
    script_pipeline::{object, optional},
};
use rquickjs::{
    Ctx, Function, Value,
    prelude::{Async, Func, Opt},
};

async fn transport<'js>(
    ctx: Ctx<'js>,
    url: Value<'js>,
    init: Opt<Value<'js>>,
) -> rquickjs::Result<Value<'js>> {
    let js = Js::new(ctx);
    let init = init.0.unwrap_or(object(&js)?);
    let request = object(&js)?;
    js.set(&request, "url", url)?;
    js.set(
        &request,
        "method",
        network::prefer(&js, [optional(&js, &init, "method")?, js.value("GET")?])?,
    )?;
    let constructor: Value = js.ctx.globals().get("Headers")?;
    let headers: Value = js.call(
        "construct",
        (constructor, js.array([optional(&js, &init, "headers")?])?),
    )?;
    let entries = js.method(headers, "entries", vec![])?;
    let constructor: Value = js.ctx.globals().get("Object")?;
    js.set(
        &request,
        "headers",
        js.method(constructor, "fromEntries", vec![entries])?,
    )?;
    js.set(&request, "body", optional(&js, &init, "body")?)?;
    js.set(&request, "encoding", Value::new_null(js.ctx.clone()))?;
    js.set(&request, "timeout", js.value(8000)?)?;
    let response = network::request(&js, "request", request).await?;
    let status = js.get(&response, "statusCode")?;
    let null_body = matches!(status.as_int(), Some(204 | 205 | 304));
    let body = if null_body {
        Value::new_null(js.ctx.clone())
    } else {
        js.get(&response, "body")?
    };
    let options = object(&js)?;
    js.set(&options, "status", status)?;
    let constructor: Value = js.ctx.globals().get("Response")?;
    js.call("construct", (constructor, js.array([body, options])?))
}

pub(crate) async fn send<'js>(js: &Js<'js>, input: Value<'js>) -> rquickjs::Result<Value<'js>> {
    let service = js.get(&input, "service")?;
    let service_text = js.text(service.clone())?;
    let subtitle = js.get(&input, "subtitle")?;
    let content = js.get(&input, "content")?;
    if service_text.starts_with("http://") || service_text.starts_with("https://") {
        let encode: Function = js.ctx.globals().get("encodeURIComponent")?;
        let title: Value = encode.call((js.get(&input, "fallbackTitle")?,))?;
        let body = js.method(
            js.array([subtitle, content])?,
            "join",
            vec![js.value("\n")?],
        )?;
        let body: Value = encode.call((body,))?;
        let url = js.method(service, "replace", vec![js.value("[推送标题]")?, title])?;
        let url = js.method(url, "replace", vec![js.value("[推送内容]")?, body])?;
        let result = network::request(js, "get", url.clone()).await;
        let text = match result {
            Ok(response) => format!(
                "[Push Service] URL: {}\nRES: {} {}",
                js.text(url)?,
                js.text(js.get(&response, "statusCode")?)?,
                js.text(js.get(&response, "body")?)?
            ),
            Err(error) => {
                let error = crate::script_sync::reason(js, error);
                format!(
                    "[Push Service] URL: {}\nERROR: {}",
                    js.text(url)?,
                    script::message(&js.ctx, error)
                )
            }
        };
        script::console_text(js, "log", &text);
    } else {
        js.ctx
            .eval::<(), _>(include_bytes!("../vendor/notification-utilities.js").as_slice())?;
        let utilities: Value = js.ctx.globals().get("SubStoreNotificationUtilities")?;
        let options = rquickjs::Object::new(js.ctx.clone())?;
        options.set("transport", Func::from(Async(transport)))?;
        let title = js.get(&input, "title")?;
        let message = js.method(
            js.array([title, subtitle, content])?,
            "join",
            vec![js.value("\n")?],
        )?;
        let pending = js.method(
            utilities,
            "send",
            vec![service, message, options.into_value()],
        );
        let result = match pending {
            Ok(pending) => script::settled(js, pending).await,
            Err(error) => Err(error),
        };
        let text = match result {
            Ok(_) => "[Push Service] RES: sent".to_owned(),
            Err(error) => {
                let reason = script::message(&js.ctx, error);
                format!(
                    "[Push Service] ERROR: {}",
                    if reason.is_empty() {
                        "notification delivery failed"
                    } else {
                        &reason
                    }
                )
            }
        };
        script::console_text(js, "log", &text);
    }
    js.value(true)
}
