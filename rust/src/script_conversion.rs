//! Common producer preparation retains caller objects and ECMAScript values.
use crate::{clash_output, conversion, js_language::Js, script_pipeline::optional};
use rquickjs::{Ctx, Value, prelude::Opt};

fn nonblank<'js>(js: &Js<'js>, value: Value<'js>) -> rquickjs::Result<bool> {
    Ok(value.is_string()
        && js.call::<usize, _>("length", (js.method(value, "trim", vec![])?,))? > 0)
}

fn is_ip<'js>(js: &Js<'js>, method: &str, value: Value<'js>) -> rquickjs::Result<bool> {
    let utils = js.ctx.globals().get("ProxyUtils")?;
    js.truthy(&js.method(utils, method, vec![value])?)
}

fn wireguard<'js>(js: &Js<'js>, proxy: &Value<'js>) -> rquickjs::Result<()> {
    for (key, cidr, method, maximum) in [
        ("ip", "ip-cidr", "isIPv4", 32.0),
        ("ipv6", "ipv6-cidr", "isIPv6", 128.0),
    ] {
        let value = js.get(proxy, key)?;
        if value.is_null() || value.is_undefined() || js.text(value.clone())?.trim().is_empty() {
            js.call::<(), _>("remove", (proxy.clone(), cidr))?;
            continue;
        }
        let raw = js.value(js.text(value)?.trim())?;
        let parts = js.method(
            raw.clone(),
            "match",
            vec![js.call("regex", (r"^(.*?)(?:/(\d+))?$", ""))?],
        )?;
        let host = if parts.is_null() {
            raw
        } else {
            js.get(&parts, "1")?
        };
        let host = js
            .text(host)?
            .trim()
            .trim_start_matches('[')
            .trim_end_matches(']')
            .to_owned();
        if !is_ip(js, method, js.value(host.clone())?)? {
            continue;
        }
        let mut prefix = None;
        for value in [js.get(proxy, cidr)?, optional(js, &parts, "2")?] {
            if value.is_null() || value.is_undefined() {
                continue;
            }
            let value = js.text(value)?.trim().to_owned();
            if !value.is_empty() && value.bytes().all(|c| c.is_ascii_digit()) {
                let number = value.parse::<f64>().unwrap_or(f64::INFINITY);
                if (0.0..=maximum).contains(&number) {
                    prefix = Some(number);
                    break;
                }
            }
        }
        js.set(proxy, key, js.value(host)?)?;
        js.set(proxy, cidr, js.value(prefix.unwrap_or(maximum))?)?;
    }
    Ok(())
}

pub(crate) fn produce<'js>(
    ctx: Ctx<'js>,
    input: Value<'js>,
    target: Value<'js>,
    kind: Opt<Value<'js>>,
    options: Opt<Value<'js>>,
) -> rquickjs::Result<Value<'js>> {
    let js = Js::new(ctx);
    let target = js.text(target)?;
    if !conversion::ALIASES.contains(&target.as_str()) {
        return Err(js.error(&format!("Target platform: {target} is not supported!")));
    }
    let normalized = target.to_lowercase();
    let options = options
        .0
        .filter(|value| !value.is_undefined())
        .unwrap_or(rquickjs::Object::new(js.ctx.clone())?.into_value());
    let scope = js.clone();
    let exact = target.clone();
    let output_target = normalized.clone();
    let opts = options.clone();
    let input = js.filter(input, move |proxy, _| {
        let include = scope.truthy(&scope.get(&opts, "include-unsupported-proxy")?)?;
        let kind = scope.get(&proxy, "type")?;
        let kind = scope.text(kind)?;
        if !include {
            let supported = scope.get(&proxy, "supported")?;
            if scope.truthy(&supported)?
                && scope.equal(&scope.get(&supported, &exact)?, &scope.value(false)?)?
            {
                return Ok(false);
            }
            let headers = optional(&scope, &proxy, "headers")?;
            let object = scope.ctx.globals().get("Object")?;
            let has_headers = scope.truthy(&headers)?
                && scope.call::<String, _>("typeOf", (headers.clone(),))? == "object"
                && scope
                    .truthy(&scope.get(&scope.method(object, "keys", vec![headers])?, "length")?)?;
            if has_headers
                && ["http", "h2-connect", "trusttunnel"].contains(&kind.as_str())
                && !(output_target.starts_with("surge")
                    || output_target == "json"
                    || (kind == "http"
                        && [
                            "egern",
                            "meta",
                            "clashmeta",
                            "clash.meta",
                            "mihomo",
                            "singbox",
                            "sing-box",
                        ]
                        .contains(&output_target.as_str())))
            {
                return Ok(false);
            }
            let network = scope.get(&proxy, "network")?;
            let network = if network.is_string() {
                scope.value(scope.text(network)?.trim().to_lowercase())?
            } else {
                network
            };
            let plugin = scope.get(&proxy, "plugin")?;
            let ss_tls = kind == "ss"
                && scope.equal(&scope.get(&proxy, "tls")?, &scope.value(true)?)?
                && (plugin.is_null() || plugin.is_undefined())
                && (network.is_null()
                    || network.is_undefined()
                    || scope.equal(&network, &scope.value("tcp")?)?);
            if ss_tls && !["qx", "quantumultx", "shadowrocket"].contains(&output_target.as_str()) {
                return Ok(false);
            }
        }
        if kind == "vless" {
            let main = scope.get(&proxy, "reality-opts")?;
            let xhttp = scope.get(&proxy, "xhttp-opts")?;
            let downstream = optional(&scope, &xhttp, "download-settings")?;
            for (download, reality) in [
                (false, main.clone()),
                (true, optional(&scope, &downstream, "reality-opts")?),
            ] {
                if scope.truthy(&reality)?
                    && !nonblank(&scope, scope.get(&reality, "public-key")?)?
                    && !(download
                        && nonblank(&scope, optional(&scope, &main, "public-key")?)?
                        && scope.equal(&scope.get(&reality, "public-key")?, &scope.value("")?)?)
                {
                    return Ok(false);
                }
            }
            if scope.equal(&scope.get(&proxy, "network")?, &scope.value("xhttp")?)?
                && scope.equal(
                    &optional(&scope, &xhttp, "mode")?,
                    &scope.value("stream-one")?,
                )?
                && scope.truthy(&downstream)?
            {
                return Ok(false);
            }
        }
        Ok(true)
    })?;
    let scope = js.clone();
    let input = js.map(input, move |proxy| {
        scope.set(&proxy, "_resolved", scope.get(&proxy, "resolved")?)?;
        if !nonblank(&scope, scope.get(&proxy, "name")?)? {
            scope.set(
                &proxy,
                "name",
                scope.value(format!(
                    "{} {}:{}",
                    scope.text(scope.get(&proxy, "type")?)?,
                    scope.text(scope.get(&proxy, "server")?)?,
                    scope.text(scope.get(&proxy, "port")?)?
                ))?,
            )?;
        }
        if scope.truthy(&scope.get(&proxy, "disable-sni")?)? {
            if ["surge", "surgemac", "shadowrocket"].contains(&normalized.as_str()) {
                scope.set(&proxy, "sni", scope.value("off")?)?;
            } else if !scope.equal(&scope.get(&proxy, "type")?, &scope.value("tuic")?)?
                && !["singbox", "sing-box"].contains(&normalized.as_str())
            {
                let server = scope.get(&proxy, "server")?;
                scope.set(
                    &proxy,
                    "sni",
                    if is_ip(&scope, "isIP", server.clone())? {
                        server
                    } else {
                        scope.value("127.0.0.1")?
                    },
                )?;
            }
        }
        let ports = scope.get(&proxy, "ports")?;
        if scope.truthy(&ports)? {
            let mut ports = scope.text(ports)?;
            if !["meta", "clashmeta", "clash.meta", "mihomo"].contains(&normalized.as_str()) {
                ports = ports.replace('/', ",");
            }
            scope.set(&proxy, "ports", scope.value(ports.clone())?)?;
            if !scope.truthy(&scope.get(&proxy, "port")?)? {
                let utils = scope.ctx.globals().get("ProxyUtils")?;
                scope.set(
                    &proxy,
                    "port",
                    scope.method(utils, "getRandomPort", vec![scope.value(ports)?])?,
                )?;
            }
        }
        if scope.equal(&scope.get(&proxy, "type")?, &scope.value("wireguard")?)? {
            wireguard(&scope, &proxy)?;
        }
        Ok(proxy)
    })?;
    let kind = kind.0.unwrap_or_else(|| js.undefined());
    if ["json", "JSON"].contains(&target.as_str()) {
        if js.equal(&kind, &js.value("internal")?)? {
            return Ok(input);
        }
        return crate::js_values::stringify(&js.ctx, input, 2);
    }
    let live = input
        .as_array()
        .ok_or_else(|| js.error("Invalid producer input"))?
        .iter::<Value>()
        .collect::<rquickjs::Result<Vec<_>>>()?;
    let mut values = live
        .iter()
        .cloned()
        .map(|value| crate::js_values::project(&js.ctx, value))
        .collect::<rquickjs::Result<Vec<_>>>()?;
    let options = crate::js_values::project(&js.ctx, options)?;
    let kind = js.text(kind)?;
    if let Some(family) = clash_output::Family::for_target(&target) {
        for (proxy, caller) in values.iter_mut().zip(live) {
            if proxy["network"] != "ws"
                || (!crate::state::truthy(&options["include-unsupported-proxy"])
                    && !clash_output::supported(proxy, family))
            {
                continue;
            }
            let before = proxy
                .get("ws-opts")
                .cloned()
                .unwrap_or(serde_json::Value::Null);
            let mut normalized = if crate::state::truthy(&before) {
                before.clone()
            } else {
                serde_json::json!({})
            };
            clash_output::websocket(&mut normalized);
            if clash_output::strip_metadata(family, &kind, &options)
                && let Some(fields) = normalized.as_object_mut()
            {
                fields.shift_remove("_v2ray-http-upgrade-ed");
            }
            let mut shared = js.get(&caller, "ws-opts")?;
            if !js.truthy(&shared)? {
                shared = rquickjs::Object::new(js.ctx.clone())?.into_value();
                js.set(&caller, "ws-opts", shared.clone())?;
            }
            // A shallow user copy still shares transport options with its source.
            if let Some(fields) = normalized.as_object() {
                for (key, value) in fields {
                    if before.get(key) != Some(value) {
                        js.set(&shared, key, crate::js_values::decode(&js.ctx, value)?)?;
                    }
                }
            }
            if let Some(fields) = before.as_object() {
                for key in fields.keys() {
                    if normalized.get(key).is_none() {
                        js.call::<(), _>("remove", (shared.clone(), key.as_str()))?;
                    }
                }
            }
            proxy["ws-opts"] = normalized;
        }
    }
    let output = conversion::produce(values, &target, &kind, &options).map_err(|error| {
        js.error(&match error {
            conversion::Error::Legacy(message) => message,
        })
    })?;
    crate::js_values::decode(&js.ctx, &output)
}
