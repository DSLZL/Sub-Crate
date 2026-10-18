use crate::json as codec;
use crate::state::truthy;
use base64::{Engine, engine::general_purpose::STANDARD};
use percent_encoding::percent_decode_str;
use serde_json::{Map, Value, json};
use std::net::{IpAddr, Ipv6Addr};

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

fn field_text(object: &Value, key: &str) -> String {
    object.get(key).map_or_else(|| "undefined".to_owned(), codec::js_string)
}

fn nonblank(value: &Value) -> Option<Value> {
    value.as_str().filter(|text| !codec::trim(text).is_empty()).map(|_| value.clone())
}

fn parse_int(raw: &str) -> Value {
    let raw = codec::trim(raw);
    let end = raw.char_indices().skip(usize::from(raw.starts_with(['+', '-'])))
        .find(|(_, c)| !c.is_ascii_digit()).map_or(raw.len(), |(index, _)| index);
    raw[..end].parse::<f64>().ok().map_or(Value::Null, |number| json!(number))
}

fn safe_integer(raw: &str) -> Option<u64> {
    if raw.is_empty() || !raw.bytes().all(|c| c.is_ascii_digit()) {
        return None;
    }
    raw.parse().ok().filter(|number| *number <= 9_007_199_254_740_991)
}

fn path_ed(path: &str) -> (String, Option<String>) {
    let Some((base, query)) = path.split_once('?') else { return (path.to_owned(), None); };
    let mut value = None;
    let mut kept = Vec::new();
    for part in query.split('&').filter(|part| !part.is_empty()) {
        let (key, raw) = part.split_once('=').unwrap_or((part, ""));
        let decode = |text: &str| component(&text.replace('+', "%20")).unwrap_or_else(|| text.to_owned());
        if decode(key) == "ed" {
            let raw = decode(raw);
            if value.is_none() && !raw.is_empty() { value = Some(raw); }
        } else { kept.push(part); }
    }
    if value.as_deref().and_then(safe_integer).is_none() { return (path.to_owned(), None); }
    let path = if kept.is_empty() { base.to_owned() } else { format!("{base}?{}", kept.join("&")) };
    (path, value)
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
    let mut proxy = json!({"type":"ss"});
    let is_legacy = !content.contains('@');
    let legacy;
    let content = if content.contains('@') {
        content
    } else {
        let Some(value) = decoded(content) else {
            return Ok(None);
        };
        if let Some(value) = query.split('&').find_map(|part| part.strip_prefix("v2ray-plugin=")) {
            let Some(options) = decoded(value).and_then(|text| codec::parse(&text).ok()) else { return Ok(None); };
            proxy["plugin"] = json!("v2ray-plugin");
            proxy["plugin-opts"] = options;
        }
        legacy = value;
        legacy.as_str()
    };
    let Some((auth, authority)) = (if is_legacy { content.rsplit_once('@') } else { content.split_once('@') }) else {
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
        params.insert(key.to_owned(), json!(value));
    }
    let user = if is_legacy {
        Some(codec::encode(auth))
    } else if let Some((cipher, password)) = auth.split_once(':') {
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
    let params = Value::Object(params);
    proxy["tls"] = params.get("security").map_or(Value::Null, |security| {
        if truthy(security) { json!(security != "none") } else { security.clone() }
    });
    proxy["skip-cert-verify"] = json!(truthy(&params["allowInsecure"]));
    proxy["sni"] = if truthy(&params["sni"]) { params["sni"].clone() } else { params["peer"].clone() };
    proxy["client-fingerprint"] = params["fp"].clone();
    proxy["alpn"] = if truthy(&params["alpn"]) {
        let Some(value) = component(&codec::text(params["alpn"].as_str().unwrap())) else { return Ok(None); };
        json!(value.split(',').collect::<Vec<_>>())
    } else { Value::Null };
    if truthy(&params["ws"]) {
        proxy["network"] = json!("ws");
        proxy["ws-opts"] = json!({"path":params["wspath"]});
    }
    if let Some(network) = params["type"].as_str().filter(|text| !text.is_empty()) {
        let upgrade = network == "httpupgrade";
        let network = if upgrade { "ws" } else { network };
        proxy["network"] = json!(network);
        let key = format!("{network}-opts");
        let mut opts = proxy.get(&key).cloned().unwrap_or_else(|| json!({}));
        if network == "grpc" {
            opts = json!({});
            for (field, param) in [("grpc-service-name", "serviceName"), ("_grpc-type", "mode"), ("_grpc-authority", "authority")] {
                if let Some(value) = params.get(param) { opts[field] = value.clone(); }
            }
            proxy[&key] = opts;
        } else {
            let mut early = None;
            if let Some(path) = params["path"].as_str().filter(|text| !text.is_empty()) {
                let path = if network == "ws" { let (path, ed) = path_ed(path); early = ed; path } else { path.to_owned() };
                opts["path"] = json!(path);
            }
            if let Some(host) = params["host"].as_str().filter(|text| !text.is_empty()) {
                let Some(host) = component(&codec::text(host)) else { return Ok(None); };
                opts["headers"] = json!({"Host":host});
            }
            if upgrade {
                if early.is_none() && params["ed"].as_str().and_then(safe_integer).is_some() { early = params["ed"].as_str().map(str::to_owned); }
                opts["v2ray-http-upgrade"] = json!(true);
                if let Some(ed) = early {
                    opts["v2ray-http-upgrade-fast-open"] = json!(true);
                    opts["_v2ray-http-upgrade-ed"] = json!(ed);
                }
            } else if network == "ws" && let Some(ed) = early {
                opts["max-early-data"] = json!(safe_integer(&ed).unwrap());
                opts["early-data-header-name"] = json!("Sec-WebSocket-Protocol");
            }
            if opts.as_object().is_some_and(|map| !map.is_empty()) { proxy[&key] = opts; }
        }
        if params["security"] == "reality" {
            let mut opts = Map::new();
            for (field, param) in [("public-key", "pbk"), ("short-id", "sid"), ("_spider-x", "spx")] {
                if truthy(&params[param]) { opts.insert(field.to_owned(), params[param].clone()); }
            }
            for param in ["mode", "extra"] { if truthy(&params[param]) { proxy[format!("_{param}")] = params[param].clone(); } }
            if !opts.is_empty() { proxy["reality-opts"] = Value::Object(opts); }
        }
    }
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
    proxy["udp"] = json!(udp);
    proxy["server"] = json!(codec::encode(normalized_server));
    let digits: String = port
        .chars()
        .skip_while(|c| !c.is_ascii_digit())
        .take_while(char::is_ascii_digit)
        .collect();
    let mut name_server = codec::encode(server);
    let mut name_port = if digits.is_empty() { "undefined".to_owned() } else { digits.clone() };
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
        let plugin_name = items.next().unwrap();
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
        let options = Value::Object(options);
        let mut plugin_opts = json!({});
        match plugin_name {
            "obfs-local" | "simple-obfs" => {
                proxy["plugin"] = json!("obfs");
                if let Some(mode) = options.get("obfs") { plugin_opts["mode"] = mode.clone(); }
                if let Some(host) = nonblank(&options["obfs-host"]) { plugin_opts["host"] = host; }
            }
            "v2ray-plugin" => {
                proxy["plugin"] = json!("v2ray-plugin");
                plugin_opts["mode"] = nonblank(&options["obfs"]).or_else(|| nonblank(&options["mode"])).unwrap_or_else(|| json!("websocket"));
                if let Some(host) = nonblank(&options["obfs-host"]).or_else(|| nonblank(&options["host"])) { plugin_opts["host"] = host; }
                if let Some(path) = nonblank(&options["path"]) { plugin_opts["path"] = path; }
                for field in ["tls", "sni"] { if let Some(value) = options.get(field) { plugin_opts[field] = value.clone(); } }
                plugin_opts["skip-cert-verify"] = json!(matches!(options["skip-cert-verify"].as_str(), Some("1" | "true")) || options["skip-cert-verify"] == true || options["skip-cert-verify"] == 1);
                if let Some(mux) = options["mux"].as_str().filter(|mux| !mux.is_empty() && mux.bytes().all(|c| c.is_ascii_digit())) { plugin_opts["mux"] = parse_int(mux); }
            }
            "shadow-tls" => {
                proxy["plugin"] = json!("shadow-tls");
                for field in ["host", "password"] { if let Some(value) = nonblank(&options[field]) { plugin_opts[field] = value; } }
                if let Some(version) = nonblank(&options["version"]) { plugin_opts["version"] = parse_int(version.as_str().unwrap()); }
            }
            _ => return Ok(None),
        }
        proxy["plugin-opts"] = plugin_opts;
    }
    for key in ["shadow-tls", "gost"] {
        if let Some(value) = query.split('&').find_map(|part| part.strip_prefix(&format!("{key}="))) {
            let value = if key == "gost" { let Some(value) = component(value) else { return Ok(None); }; value } else { value.to_owned() };
            let Some(options) = decoded(&value).and_then(|text| codec::parse(&text).ok()) else { return Ok(None); };
            proxy["plugin"] = json!(if key == "gost" { "gost-plugin" } else { key });
            let mut opts = json!({});
            if key == "gost" && let Some(route) = nonblank(&options["route"]) {
                let normalized = codec::trim(route.as_str().unwrap()).to_lowercase();
                opts["mode"] = if ["ws", "wss", "websocket"].contains(&normalized.as_str()) { json!("websocket") } else { route };
            }
            for field in ["host", if key == "gost" { "path" } else { "password" }] { if let Some(value) = nonblank(&options[field]) { opts[field] = value; } }
            if key == "shadow-tls" { if let Some(version) = nonblank(&options["version"]) { opts["version"] = parse_int(version.as_str().unwrap()); } }
            else if options["route"].as_str().is_some_and(|route| codec::trim(route).eq_ignore_ascii_case("wss")) { opts["tls"] = json!(true); }
            proxy["plugin-opts"] = opts;
            if let Some(address) = nonblank(&options["address"]) { name_server = codec::js_string(&address); proxy["server"] = address; }
            if let Some(port) = nonblank(&options["port"]) {
                proxy["port"] = parse_int(port.as_str().unwrap());
                name_port = if proxy["port"].is_null() { "NaN".to_owned() } else { field_text(&proxy, "port") };
            }
        }
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
    proxy["name"] = json!(name.unwrap_or_else(|| format!("SS {name_server}:{name_port}")));
    if let Some(server) = proxy["server"].as_str() {
        let server = codec::trim(server);
        let server = server.strip_prefix('[').unwrap_or(server);
        proxy["server"] = json!(server.strip_suffix(']').unwrap_or(server));
    }
    normalize_ss(&mut proxy);
    for field in ["tls", "sni", "client-fingerprint", "alpn"] {
        if proxy[field].is_null() { proxy.as_object_mut().unwrap().shift_remove(field); }
    }
    Ok(Some(proxy))
}

fn normalize_ss(proxy: &mut Value) {
    let keys: Vec<_> = proxy.as_object().unwrap().keys().cloned().collect();
    for key in keys {
        let lower = key.to_lowercase();
        if !lower.ends_with("-opts") { continue; }
        if key != lower {
            let value = proxy.as_object_mut().unwrap().shift_remove(&key).unwrap();
            proxy.as_object_mut().unwrap().entry(lower.clone()).or_insert(value);
        }
        if let Some(options) = proxy[&lower].as_object_mut() {
            for key in options.keys().cloned().collect::<Vec<_>>() {
                let lower = key.to_lowercase();
                if key != lower {
                    let value = options.shift_remove(&key).unwrap();
                    options.entry(lower).or_insert(value);
                }
            }
        }
    }
    if proxy["plugin"] == "shadow-tls" && !proxy["alpn"].is_null() {
        if proxy["plugin-opts"]["alpn"].is_null() { proxy["plugin-opts"]["alpn"] = proxy["alpn"].clone(); }
        proxy.as_object_mut().unwrap().shift_remove("alpn");
    }
    if let Some(network) = proxy["network"].as_str().map(str::to_owned) {
        let key = format!("{network}-opts");
        let path = proxy[&key]["path"].as_str().map(|path| {
            let path = codec::trim(path);
            if path.starts_with('/') { path.to_owned() } else { format!("/{path}") }
        });
        if let Some(path) = path { proxy[&key]["path"] = json!(path); }
        if !truthy(&proxy["tls"]) && ["ws", "http"].contains(&network.as_str()) && !truthy(&proxy[&key]["headers"]["Host"]) && proxy["server"].as_str().is_some_and(|server| server.parse::<IpAddr>().is_err()) {
            proxy[&key]["headers"]["Host"] = proxy["server"].clone();
        }
        if network == "h2" {
            if truthy(&proxy[&key]["headers"]["Host"]) { proxy[&key]["host"] = json!([proxy[&key]["headers"]["Host"]]); }
            if let Some(headers) = proxy[&key]["headers"].as_object_mut() { headers.shift_remove("Host"); }
            if proxy[&key]["headers"].as_object().is_some_and(Map::is_empty) { proxy[&key].as_object_mut().unwrap().shift_remove("headers"); }
        }
        if truthy(&proxy["tls"]) && !truthy(&proxy["sni"]) && proxy["sni"] != "" {
            let host = if network == "h2" { proxy[&key]["host"].as_array().and_then(|hosts| hosts.first()).cloned().unwrap_or(Value::Null) } else { proxy[&key]["headers"]["Host"].clone() };
            if truthy(&host) { proxy["sni"] = host; }
        }
        if ["ws", "h2"].contains(&network.as_str()) && !truthy(&proxy[&key]["path"]) { proxy[&key]["path"] = json!("/"); }
        if network == "http" { proxy[&key]["path"] = json!(["/"]); }
    }
    if truthy(&proxy["tls"]) && !truthy(&proxy["sni"]) && proxy["sni"] != "" && proxy["server"].as_str().is_some_and(|server| server.parse::<IpAddr>().is_err()) {
        proxy["sni"] = proxy["server"].clone();
    }
    if matches!(proxy["sni"].as_str(), Some("" | "off")) { proxy["disable-sni"] = json!(true); }
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
    if truthy(&proxy["plugin"]) {
        let options = &proxy["plugin-opts"];
        let plugin = match proxy["plugin"].as_str()? {
            "obfs" => format!("simple-obfs;obfs={}{}", field_text(options, "mode"), if truthy(&options["host"]) { format!(";obfs-host={}", field_text(options, "host")) } else { String::new() }),
            "v2ray-plugin" => {
                let mut plugin = format!("v2ray-plugin;obfs={};mode={}", field_text(options, "mode"), field_text(options, "mode"));
                if truthy(&options["host"]) { plugin.push_str(&format!(";obfs-host={};host={}", field_text(options, "host"), field_text(options, "host"))); }
                if truthy(&options["path"]) { plugin.push_str(&format!(";path={}", field_text(options, "path"))); }
                if truthy(&options["tls"]) { plugin.push_str(";tls"); }
                if truthy(&options["sni"]) { plugin.push_str(&format!(";sni={}", field_text(options, "sni"))); }
                if truthy(&options["skip-cert-verify"]) { plugin.push_str(&format!(";skip-cert-verify={}", field_text(options, "skip-cert-verify"))); }
                if !options["mux"].is_null() {
                    let mux = match &options["mux"] {
                        Value::Bool(value) => json!(u8::from(*value)),
                        Value::String(value) if codec::trim(value).eq_ignore_ascii_case("true") => json!(1),
                        Value::String(value) if codec::trim(value).eq_ignore_ascii_case("false") => json!(0),
                        Value::String(value) if safe_integer(codec::trim(value)).is_some() => parse_int(codec::trim(value)),
                        value => value.clone(),
                    };
                    plugin.push_str(&format!(";mux={}", codec::js_string(&mux)));
                }
                plugin
            }
            "shadow-tls" => format!("shadow-tls;host={};password={};version={}", field_text(options, "host"), field_text(options, "password"), field_text(options, "version")),
            _ => return None,
        };
        output.push('/');
        query.push(format!("plugin={}", codec::uri_component(&plugin).ok()?));
    }
    for (field, parameter) in [("udp-over-tcp", "uot"), ("tfo", "tfo")] {
        if truthy(&proxy[field]) {
            query.push(format!("{parameter}=1"));
        }
    }
    query.push(format!("udp={}", u8::from(truthy(&proxy["udp"]))));
    if truthy(&proxy["tls"]) {
        let sni = if truthy(&proxy["sni"]) { field_text(proxy, "sni") } else { server.clone() };
        query.push(format!("sni={}", codec::uri_component(&sni).ok()?));
        if truthy(&proxy["skip-cert-verify"]) { query.push("allowInsecure=1".to_owned()); }
    }
    if let Some(network) = proxy["network"].as_str().filter(|text| !text.is_empty()) {
        let options = &proxy[format!("{network}-opts")];
        let upgrade = network == "ws" && truthy(&options["v2ray-http-upgrade"]);
        query.push(format!("type={}", codec::uri_component(if upgrade { "httpupgrade" } else { network }).ok()?));
        if network == "grpc" {
            for (field, param) in [("grpc-service-name", "serviceName"), ("_grpc-authority", "authority")] {
                if truthy(&options[field]) { query.push(format!("{param}={}", codec::uri_component(&field_text(options, field)).ok()?)); }
            }
            query.push(format!("mode={}", codec::uri_component(&if truthy(&options["_grpc-type"]) { field_text(options, "_grpc-type") } else { "gun".to_owned() }).ok()?));
        }
        let mut path = options["path"].as_array().and_then(|items| items.first()).unwrap_or(&options["path"]).as_str().unwrap_or("").to_owned();
        let early = if upgrade && truthy(&options["v2ray-http-upgrade-fast-open"]) { options["_v2ray-http-upgrade-ed"].as_str().filter(|ed| safe_integer(ed).is_some()).map(str::to_owned).or_else(|| Some("2560".to_owned())) }
            else if network == "ws" && matches!(options["early-data-header-name"].as_str(), None | Some("" | "Sec-WebSocket-Protocol")) { options.get("max-early-data").map(codec::js_string).filter(|ed| safe_integer(ed).is_some()) } else { None };
        if let Some(ed) = early {
            let (base, _) = path_ed(&path);
            path = if base.is_empty() { "/".to_owned() } else { base };
            let separator = if path.ends_with(['?', '&']) { "" } else if path.contains('?') { "&" } else { "?" };
            path = format!("{path}{separator}ed={ed}");
        }
        if !path.is_empty() { query.push(format!("path={}", codec::uri_component(&path).ok()?)); }
        let host = options["headers"]["Host"].as_array().and_then(|items| items.first()).unwrap_or(&options["headers"]["Host"]);
        if truthy(host) { query.push(format!("host={}", codec::uri_component(&codec::js_string(host)).ok()?)); }
    }
    if truthy(&proxy["alpn"]) { query.push(format!("alpn={}", codec::uri_component(&codec::js_string(&proxy["alpn"])).ok()?)); }
    if truthy(&proxy["client-fingerprint"]) { query.push(format!("fp={}", codec::uri_component(&field_text(proxy, "client-fingerprint")).ok()?)); }
    if truthy(&proxy["reality-opts"]) {
        query.push("security=reality".to_owned());
        for (field, param) in [("short-id", "sid"), ("public-key", "pbk"), ("_spider-x", "spx")] {
            if truthy(&proxy["reality-opts"][field]) { query.push(format!("{param}={}", codec::uri_component(&field_text(&proxy["reality-opts"], field)).ok()?)); }
        }
        for param in ["mode", "extra"] { let field = format!("_{param}"); if truthy(&proxy[&field]) { query.push(format!("{param}={}", codec::uri_component(&field_text(proxy, &field)).ok()?)); } }
    } else if truthy(&proxy["tls"]) { query.push("security=tls".to_owned()); }
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
    proxies.retain(|proxy| !(proxy["tls"] == true && proxy.get("plugin").is_none_or(Value::is_null) && proxy["network"].as_str().is_none_or(|network| codec::trim(network).eq_ignore_ascii_case("tcp"))));
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
        if truthy(&proxy["disable-sni"]) {
            proxy["sni"] = json!(if proxy["server"].as_str().is_some_and(|server| server.parse::<IpAddr>().is_ok()) { field_text(proxy, "server") } else { "127.0.0.1".to_owned() });
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
