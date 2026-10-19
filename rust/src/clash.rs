use crate::conversion::Error;
use crate::json as codec;
use crate::state::truthy;
use rquickjs::{Array, Context, Ctx, Exception, Function, Object, Runtime, Value as JsValue};
use serde_json::Value;

enum Parsed {
    Proxy(Value),
    Deferred,
}

pub struct Parser(Context);

fn yaml<'js>(ctx: &Ctx<'js>, raw: &str) -> rquickjs::Result<JsValue<'js>> {
    let utilities: Object = ctx.globals().get("SubStoreUtilities")?;
    let parse: Function = utilities.get("parseYaml")?;
    let options = ctx.json_parse(r#"{"logLevel":"error","merge":true}"#)?;
    match parse.call((raw, options.clone())) {
        Ok(value) => Ok(value),
        Err(_) => {
            ctx.catch();
            let mut parts = raw.split("!<str>");
            let mut replaced = parts.next().unwrap_or_default().to_owned();
            for part in parts {
                replaced.push_str("__SubStoreJSYAMLString__");
                replaced.push_str(part.trim_start_matches(codec::is_space));
            }
            let value: JsValue = parse.call((replaced, options.clone()))?;
            let text = ctx
                .json_stringify(value)?
                .ok_or_else(|| Exception::throw_type(ctx, "Value is not JSON serializable"))?
                .to_string()?;
            let value = ctx.json_parse(text)?;
            let dump: Function = utilities.get("stringifyYaml")?;
            let text: String = dump.call((value,))?;
            parse.call((text.replace("__SubStoreJSYAMLString__", ""), options))
        }
    }
}

fn short_ids(raw: &str) -> String {
    let mut output = String::new();
    let mut remaining = raw;
    while let Some((before, tail)) = remaining.split_once("short-id:") {
        output.push_str(before);
        let end = tail.find(['#', '\n', ',', '}']).unwrap_or(tail.len());
        let value = codec::trim(&tail[..end]);
        output.push_str("short-id: ");
        if value == "null"
            || (value.len() >= 2
                && !value.contains(['\r', '\n', '\u{2028}', '\u{2029}'])
                && ((value.starts_with('"') && value.ends_with('"'))
                    || (value.starts_with('\'') && value.ends_with('\''))))
        {
            output.push_str(value);
        } else {
            output.push('"');
            output.push_str(value);
            output.push('"');
        }
        remaining = &tail[end..];
    }
    output.push_str(remaining);
    output
}

impl Parser {
    pub fn new() -> Result<Self, Error> {
        let runtime = Runtime::new().map_err(|error| Error::Legacy(error.to_string()))?;
        let context = Context::full(&runtime).map_err(|error| Error::Legacy(error.to_string()))?;
        context.with(|ctx| {
            ctx.eval::<(), _>(include_bytes!("../vendor/utilities.js").as_slice())
                .map_err(|error| Error::Legacy(error.to_string()))
        })?;
        Ok(Self(context))
    }

    pub fn dump(&self, value: &Value) -> Result<String, Error> {
        self.0.with(|ctx| {
            let result = (|| -> rquickjs::Result<String> {
                let utilities: Object = ctx.globals().get("SubStoreUtilities")?;
                let dump: Function = utilities.get("stringifyYaml")?;
                let content = ctx.json_parse(codec::stringify(value))?;
                let options = ctx.json_parse(r#"{"lineWidth":0}"#)?;
                let text: String = dump.call((content, options))?;
                Ok(
                    if text.contains("proxies:")
                        && text.contains("short-id:")
                        && value["proxies"]
                            .as_array()
                            .is_some_and(|proxies| !proxies.is_empty())
                    {
                        short_ids(&text)
                    } else {
                        text
                    },
                )
            })();
            result.map_err(|error| {
                ctx.catch();
                Error::Legacy(error.to_string())
            })
        })
    }

    pub fn preprocess(&self, raw: &str) -> Option<String> {
        if !raw.contains("proxies") {
            return None;
        }
        self.0.with(|ctx| {
            let result = (|| -> rquickjs::Result<Option<String>> {
                let value = yaml(&ctx, raw)?;
                let Some(root) = value.as_object() else {
                    return Ok(None);
                };
                let proxies: JsValue = root.get("proxies")?;
                let groups: JsValue = root.get("proxy-groups")?;
                if !proxies.is_array() && !groups.is_array() {
                    return Ok(None);
                }
                let fixed = if raw.contains("proxies:")
                    && raw.contains("short-id:")
                    && proxies.as_array().is_some_and(|array| !array.is_empty())
                {
                    short_ids(raw)
                } else {
                    raw.to_owned()
                };
                let value = yaml(&ctx, &fixed)?;
                let Some(root) = value.as_object() else {
                    return Ok(None);
                };
                let proxies: JsValue = root.get("proxies")?;
                let mut output = String::new();
                if let Some(array) = proxies.as_array() {
                    for value in array.iter::<JsValue>() {
                        let text = ctx
                            .json_stringify(value?)?
                            .map(|text| text.to_string())
                            .transpose()?
                            .unwrap_or_else(|| "undefined".to_owned());
                        output.push_str(&text);
                        output.push('\n');
                    }
                }
                Ok(Some(output))
            })();
            match result {
                Ok(value) => value,
                Err(_) => {
                    ctx.catch();
                    None
                }
            }
        })
    }

    pub fn line(&self, raw: &str) -> Result<Option<Value>, Error> {
        self.0.with(|ctx| {
            let result = (|| -> rquickjs::Result<Option<Parsed>> {
                let utilities: Object = ctx.globals().get("SubStoreUtilities")?;
                let json5: Object = utilities.get("JSON5")?;
                let parse: Function = json5.get("parse")?;
                let value: JsValue = match parse.call((raw,)) {
                    Ok(value) => value,
                    Err(_) => {
                        ctx.catch();
                        yaml(&ctx, raw)?
                    }
                };
                let Some(object) = value.as_object() else {
                    return Ok(None);
                };
                let kind: JsValue = object.get("type")?;
                let kind = kind.as_string().and_then(|kind| kind.to_string().ok());
                if !kind.as_deref().is_some_and(|kind| {
                    [
                        "zerotier",
                        "shadowquic",
                        "gost-relay",
                        "openvpn",
                        "tailscale",
                        "easytier",
                        "trusttunnel",
                        "h2-connect",
                        "naive",
                        "anytls",
                        "mieru",
                        "masque",
                        "masque-surge",
                        "sudoku",
                        "juicity",
                        "ss",
                        "ssr",
                        "vmess",
                        "socks5",
                        "http",
                        "snell",
                        "trojan",
                        "tuic",
                        "vless",
                        "hysteria",
                        "hysteria2",
                        "wireguard",
                        "ssh",
                        "direct",
                    ]
                    .contains(&kind)
                }) {
                    return Ok(None);
                }
                let stringify: Function = ctx.eval(
                    "(value) => { let nonfinite = false; const text = JSON.stringify(value, \
                    (_, field) => { if (typeof field === 'number' && !Number.isFinite(field)) \
                    nonfinite = true; return field; }); return [text, nonfinite]; }",
                )?;
                let serialized: Array = match stringify.call((value,)) {
                    Ok(value) => value,
                    Err(_) => {
                        ctx.catch();
                        return Ok(Some(Parsed::Deferred));
                    }
                };
                if serialized.get::<bool>(1)? {
                    return Ok(Some(Parsed::Deferred));
                }
                let text: String = serialized.get(0)?;
                Ok(codec::parse(&text).ok().map(Parsed::Proxy))
            })();
            let Some(parsed) = (match result {
                Ok(value) => value,
                Err(_) => {
                    ctx.catch();
                    return Ok(None);
                }
            }) else {
                return Ok(None);
            };
            // shortcut: non-finite and cyclic typed values need the M3 script/value bridge.
            let Parsed::Proxy(mut proxy) = parsed else {
                return Err(Error::Pending(
                    "Non-finite or cyclic Clash values are not implemented yet",
                ));
            };
            if matches!(proxy["type"].as_str(), Some("vmess" | "vless"))
                && truthy(&proxy["servername"])
            {
                proxy["sni"] = proxy["servername"].clone();
                proxy.as_object_mut().unwrap().shift_remove("servername");
            }
            for (from, to) in [
                ("server-cert-fingerprint", "tls-fingerprint"),
                ("fingerprint", "tls-fingerprint"),
                ("dialer-proxy", "underlying-proxy"),
                ("benchmark-url", "test-url"),
                ("benchmark-timeout", "test-timeout"),
            ] {
                if truthy(&proxy[from]) {
                    proxy[to] = proxy[from].clone();
                }
            }
            if proxy["type"] == "vmess" {
                proxy["cipher"] = crate::conversion::vmess_security(&proxy["cipher"]);
            }
            Ok(Some(proxy))
        })
    }
}
