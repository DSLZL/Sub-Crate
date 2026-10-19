use crate::{js_language::Js, script, script_backup, script_download, script_pipeline::object};
use rquickjs::{Function, Value};

pub(crate) async fn startup<'js>(js: &Js<'js>, input: Value<'js>) -> rquickjs::Result<Value<'js>> {
    let body = script_download::source(js, js.get(&input, "url")?, object(js)?).await?;
    let decoded = object(js)?;
    js.set(&decoded, "body", body)?;
    js.set(&decoded, "keep", js.array([])?)?;
    js.set(&decoded, "current", object(js)?)?;
    let content = script_backup::decode(js, decoded)?;
    let source = js.get(&input, "script")?;
    if !js.truthy(&source)? { return Ok(content); }
    script::log(js, "info", "[BACKEND] executing post-processing script");
    let evaluate: Function = js.ctx.eval("(function(content, source) { eval(source); return content; })")?;
    evaluate.call((content, source))
}
