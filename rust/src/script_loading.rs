//! Existing inline, linked, module and local script source selection.
use crate::{js_language::Js, script, script_download, script_network as network};
use rquickjs::{Object, Value};

pub(crate) async fn load<'js>(
    js: &Js<'js>,
    item: &Value<'js>,
    execution: &Value<'js>,
) -> rquickjs::Result<(Value<'js>, Value<'js>)> {
    let args = network::prefer(js, [js.get(item, "args")?, Object::new(js.ctx.clone())?.into_value()])?;
    let content = js.get(&args, "content")?;
    if !js.equal(&js.get(&args, "mode")?, &js.value("link")?)? {
        let arguments = network::prefer(js, [js.get(&args, "arguments")?, Object::new(js.ctx.clone())?.into_value()])?;
        return Ok((content, arguments));
    }
    let raw = network::prefer(js, [content, js.value("")?])?;
    let parts = js.method(raw.clone(), "split", vec![js.value("#")?])?;
    let (clean, arguments) = network::fragment(js, raw)?;
    let suffix = js.get(&parts, "2")?;
    let suffix = if js.truthy(&suffix)? {
        Some(suffix)
    } else {
        let no_cache: Value = js.call("optional", (arguments.clone(), "noCache"))?;
        let insecure: Value = js.call("optional", (arguments.clone(), "insecure"))?;
        if (!no_cache.is_null() && !no_cache.is_undefined()) || (!insecure.is_null() && !insecure.is_undefined()) {
            Some(js.get(&parts, "1")?)
        } else {
            None
        }
    };
    let url = if let Some(suffix) = suffix {
        js.value(format!("{}#{}", js.text(clean.clone())?, js.text(suffix)?))?
    } else {
        clean.clone()
    };
    let path = js.text(clean.clone())?;
    let internal = if path.strip_prefix("/api/module/").is_some_and(|name| !name.is_empty()) { Some("module") }
        else if path.strip_prefix("/api/file/").is_some_and(|name| !name.is_empty()) { Some("file") } else { None };
    let options = Object::new(js.ctx.clone())?;
    options.set("noFlow", js.call::<Value, _>("optional", (execution.clone(), "noFlow"))?)?;
    let loaded = if let Some(kind) = internal {
        match script_download::source(js, clean, options.into_value()).await {
            Ok(value) => value,
            Err(error) => {
                eprintln!("[sub-store:error] Error when loading {kind}: {}", script::message(&js.ctx, error));
                return Err(js.error(&format!("无法加载 {kind}: {}", js.text(url)?)));
            }
        }
    } else if path.starts_with('/') || std::path::Path::new(&path).is_absolute() {
        match std::fs::read(&path) {
            Ok(bytes) => js.value(String::from_utf8_lossy(&bytes).into_owned())?,
            Err(error) => {
                eprintln!("[sub-store:error] Error when reading local script: {error}");
                return Err(js.error(&format!("无法从该路径读取脚本文件: {}", js.text(url)?)));
            }
        }
    } else {
        match script_download::source(js, url.clone(), options.into_value()).await {
            Ok(value) => value,
            Err(error) => {
                eprintln!("[sub-store:error] Error when downloading remote script: {}", script::message(&js.ctx, error));
                return Err(js.error(&format!("无法下载脚本: {}", js.text(url)?)));
            }
        }
    };
    Ok((loaded, arguments))
}
