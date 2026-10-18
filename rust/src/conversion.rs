use crate::json as codec;
use crate::state::truthy;
use base64::{Engine, engine::general_purpose::STANDARD};
use percent_encoding::percent_decode_str;
use serde_json::{Map, Value, json};
use std::net::Ipv6Addr;

pub enum Error {
    Pending(&'static str),
    Legacy(String),
}

fn component(raw: &str) -> Option<String> {
    if raw.as_bytes().iter().enumerate().any(|(index, byte)| {
        *byte == b'%'
            && !raw
                .as_bytes()
                .get(index + 1..index + 3)
                .is_some_and(|pair| pair.iter().all(u8::is_ascii_hexdigit))
    }) {
        return None;
    }
    percent_decode_str(raw)
        .decode_utf8()
        .ok()
        .map(|text| codec::encode(&text))
}

fn decoded(raw: &str) -> Option<String> {
    codec::decode_base64(raw)
        .ok()
        .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
}

fn ss(line: &str) -> Result<Option<Value>, Error> {
    let raw = &line[5..];
    let (content, fragment) = raw
        .split_once('#')
        .map_or((raw, None), |(raw, name)| (raw, Some(name)));
    let name = match fragment {
        Some(fragment) => match component(fragment) {
            Some(name) => Some(name),
            None => return Ok(None),
        },
        None => None,
    };
    let (content, query) = content.split_once('?').unwrap_or((content, ""));
    let legacy;
    let content = if content.contains('@') {
        content
    } else {
        let Some(value) = decoded(content) else {
            return Ok(None);
        };
        legacy = value;
        legacy.as_str()
    };
    let Some((auth, authority)) = content.split_once('@') else {
        return Ok(None);
    };
    let authority = authority.split('/').next().unwrap_or("");
    let Some((server, port)) = authority.rsplit_once(':') else {
        return Ok(None);
    };
    let mut params = Map::new();
    for part in query.split('&').filter(|part| !part.is_empty()) {
        let mut pair = part.split('=');
        let key = pair.next().unwrap();
        let Some(value) = component(pair.next().unwrap_or("undefined")) else {
            return Ok(None);
        };
        // shortcut: transport/TLS and remaining SIP002 extensions land with the M3 parser batch.
        if !["udp", "uot", "tfo", "plugin"].contains(&key) {
            return Err(Error::Pending(
                "SS query extensions are not implemented yet",
            ));
        }
        params.insert(key.to_owned(), json!(value));
    }
    let user = if let Some((cipher, password)) = auth.split_once(':') {
        match (component(cipher), component(password)) {
            (Some(cipher), Some(password)) => Some(format!("{cipher}:{password}")),
            _ => None,
        }
    } else {
        component(auth).and_then(|auth| {
            if auth.contains(':') {
                Some(auth)
            } else {
                decoded(&auth).map(|text| codec::encode(&text))
            }
        })
    };
    let Some(user) = user else {
        return Ok(None);
    };
    let udp = !matches!(
        params
            .get("udp")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_lowercase()
            .as_str(),
        "0" | "false" | "off"
    );
    let normalized_server = codec::trim(server);
    let normalized_server = normalized_server
        .strip_prefix('[')
        .unwrap_or(normalized_server);
    let normalized_server = normalized_server
        .strip_suffix(']')
        .unwrap_or(normalized_server);
    let mut proxy = json!({"type":"ss", "skip-cert-verify":false, "udp":udp, "server":codec::encode(normalized_server)});
    let digits: String = port
        .chars()
        .skip_while(|c| !c.is_ascii_digit())
        .take_while(char::is_ascii_digit)
        .collect();
    if !digits.is_empty() {
        proxy["port"] = match digits.parse::<u16>() {
            Ok(number)
                if digits.len() <= 4
                    || (digits.len() == 5
                        && (!digits.starts_with('0')
                            || digits.bytes().all(|byte| byte <= b'5'))) =>
            {
                json!(number)
            }
            _ => json!(digits),
        };
    }
    if let Some((cipher, password)) = user.split_once(':') {
        proxy["cipher"] = json!(cipher.to_lowercase());
        proxy["password"] = json!(password);
    }
    if let Some(plugin) = params
        .get("plugin")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
    {
        let mut items = plugin.split(';');
        if !matches!(items.next(), Some("obfs-local" | "simple-obfs")) {
            return Err(Error::Pending("This SS plugin is not implemented yet"));
        }
        let mut options = Map::new();
        for item in items {
            let (key, value) = item.split_once('=').unwrap_or((item, ""));
            if !key.is_empty() {
                options.insert(
                    key.to_owned(),
                    if value.is_empty() {
                        json!(true)
                    } else {
                        json!(value.replace("\\=", "="))
                    },
                );
            }
        }
        proxy["plugin"] = json!("obfs");
        let mut plugin_opts = Map::new();
        if let Some(mode) = options.get("obfs") {
            plugin_opts.insert("mode".to_owned(), mode.clone());
        }
        if let Some(host) = options.get("obfs-host").filter(|host| {
            host.as_str()
                .is_none_or(|text| !codec::trim(text).is_empty())
        }) {
            plugin_opts.insert("host".to_owned(), host.clone());
        }
        proxy["plugin-opts"] = Value::Object(plugin_opts);
    }
    for (parameter, field) in [("uot", "udp-over-tcp"), ("tfo", "tfo")] {
        if query.split('&').any(|part| {
            let part = part.to_ascii_lowercase();
            part.starts_with(&format!("{parameter}=1"))
                || part.starts_with(&format!("{parameter}=true"))
        }) {
            proxy[field] = json!(true);
        }
    }
    proxy["name"] = json!(name.unwrap_or_else(|| codec::encode(&format!(
        "SS {server}:{}",
        if digits.is_empty() {
            "undefined"
        } else {
            &digits
        }
    ))));
    Ok(Some(proxy))
}

pub fn parse(raw: &Value) -> Result<Vec<Value>, Error> {
    let Some(raw) = raw.as_str() else {
        return Err(Error::Pending(
            "Non-text subscription parsing is not implemented yet",
        ));
    };
    if raw.starts_with("<!DOCTYPE html>") {
        return Ok(Vec::new());
    }
    // shortcut: SSD imports must fail explicitly until their native M3 preprocessor is ported.
    if raw.starts_with("ssd://") {
        return Err(Error::Pending("SSD preprocessing is not implemented yet"));
    }
    let raw = String::from_utf16(&codec::units(raw)).map_err(|_| {
        Error::Pending("URI parsing of isolated UTF-16 surrogates is not implemented yet")
    })?;
    let mut proxies = Vec::new();
    for line in raw
        .split('\n')
        .map(codec::trim)
        .filter(|line| !line.is_empty())
    {
        if line.starts_with("ss://") {
            if let Some(proxy) = ss(line)? {
                proxies.push(proxy);
            }
        } else if line.split_once("://").is_some_and(|(scheme, _)| {
            [
                "socks5+tls",
                "socks5",
                "socks",
                "http",
                "https",
                "ssr",
                "vmess",
                "vless",
                "trojan",
                "anytls",
                "hy",
                "hysteria",
                "hy2",
                "hysteria2",
                "tuic",
                "wireguard",
                "wg",
                "juicity",
                "trusttunnel",
                "h2-connect",
                "naive",
                "masque",
                "shadowquic",
                "snell",
            ]
            .contains(&scheme)
        }) || line.contains("proxies:")
            || line.starts_with('[')
            || line.starts_with('{')
            || line.contains(" = ")
            || decoded(line).is_some_and(|text| text.contains("://"))
        {
            return Err(Error::Pending(
                "This subscription format is not implemented yet",
            ));
        }
    }
    Ok(proxies)
}

pub fn local(sub: &Value, content: Option<&Value>) -> Result<Vec<Value>, Error> {
    if content.is_none()
        && (sub["source"] != "local"
            || matches!(
                sub["mergeSources"].as_str(),
                Some("localFirst" | "remoteFirst")
            ))
    {
        return Err(Error::Pending(
            "Remote and merged subscription sources are not implemented yet",
        ));
    }
    if sub.get("process").is_some_and(|process| {
        truthy(process) && process.as_array().is_none_or(|items| !items.is_empty())
    }) {
        return Err(Error::Pending(
            "Subscription processors are not implemented yet",
        ));
    }
    let content = content.unwrap_or(&sub["content"]);
    let sources = content
        .as_array()
        .map_or_else(|| vec![content], |items| items.iter().collect());
    let mut result = Vec::new();
    for source in sources {
        result.extend(parse(source)?);
    }
    Ok(result)
}

pub fn tag(proxies: &mut [Value], sub: &Value, preview: bool) {
    for (index, proxy) in proxies.iter_mut().enumerate() {
        if preview {
            proxy["id"] = json!(index);
        }
        for (source, field) in [("name", "_subName"), ("displayName", "_subDisplayName")] {
            if let Some(value) = sub.get(source) {
                proxy[field] = value.clone();
            }
        }
    }
}

fn port_text(proxy: &Value) -> String {
    proxy.get("port").map_or_else(
        || "undefined".to_owned(),
        |value| {
            value
                .as_str()
                .map_or_else(|| value.to_string(), str::to_owned)
        },
    )
}

fn uri(proxy: &Value) -> Option<String> {
    let cipher = proxy["cipher"].as_str().unwrap_or("undefined");
    let password = proxy["password"].as_str().unwrap_or("undefined");
    let auth = if cipher.starts_with("2022-blake3-") {
        format!(
            "{}:{}",
            codec::uri_component(cipher).ok()?,
            codec::uri_component(password).ok()?
        )
    } else {
        STANDARD.encode(codec::text(&format!("{cipher}:{password}")))
    };
    let server = codec::text(proxy["server"].as_str().unwrap_or("undefined"));
    let server = if server.parse::<Ipv6Addr>().is_ok() {
        format!("[{server}]")
    } else {
        server
    };
    let port = port_text(proxy);
    let mut output = format!("ss://{auth}@{server}:{port}");
    let mut query = Vec::new();
    if proxy["plugin"] == "obfs" {
        let options = &proxy["plugin-opts"];
        let mut plugin = format!(
            "simple-obfs;obfs={}",
            options["mode"].as_str().unwrap_or("undefined")
        );
        if truthy(&options["host"]) {
            plugin.push_str(";obfs-host=");
            plugin.push_str(options["host"].as_str().unwrap_or("true"));
        }
        output.push('/');
        query.push(format!("plugin={}", codec::uri_component(&plugin).ok()?));
    }
    for (field, parameter) in [("udp-over-tcp", "uot"), ("tfo", "tfo")] {
        if truthy(&proxy[field]) {
            query.push(format!("{parameter}=1"));
        }
    }
    query.push(format!("udp={}", u8::from(truthy(&proxy["udp"]))));
    output.push('?');
    output.push_str(&query.join("&"));
    output.push('#');
    output.push_str(&codec::uri_component(proxy["name"].as_str()?).ok()?);
    Some(output)
}

pub fn produce(mut proxies: Vec<Value>, target: &str, kind: &str) -> Result<Value, Error> {
    match target {
        "json" | "JSON" | "uri" | "URI" | "v2" | "v2ray" | "V2Ray" => {}
        "qx" | "QX" | "QuantumultX" | "surge" | "Surge" | "SurgeMac" | "Loon" | "Clash"
        | "meta" | "clashmeta" | "clash.meta" | "Clash.Meta" | "ClashMeta" | "mihomo"
        | "Mihomo" | "stash" | "Stash" | "shadowrocket" | "Shadowrocket" | "ShadowRocket"
        | "surfboard" | "Surfboard" | "singbox" | "sing-box" | "egern" | "Egern" => {
            return Err(Error::Pending("This output format is not implemented yet"));
        }
        _ => {
            return Err(Error::Legacy(format!(
                "Target platform: {target} is not supported!"
            )));
        }
    }
    for proxy in &mut proxies {
        if proxy["name"]
            .as_str()
            .is_none_or(|name| codec::trim(name).is_empty())
        {
            proxy["name"] = json!(format!(
                "ss {}:{}",
                proxy["server"].as_str().unwrap_or("undefined"),
                port_text(proxy)
            ));
        }
    }
    if matches!(target, "json" | "JSON") {
        let value = Value::Array(proxies);
        return Ok(if kind == "internal" {
            value
        } else {
            json!(String::from_utf8(codec::pretty(&value)).unwrap())
        });
    }
    let lines: Vec<_> = proxies.iter().filter_map(uri).collect();
    if matches!(target, "v2" | "v2ray" | "V2Ray") {
        return Ok(json!(STANDARD.encode(lines.join("\n"))));
    }
    Ok(if kind == "internal" {
        json!(lines)
    } else {
        json!(lines.join("\n"))
    })
}
