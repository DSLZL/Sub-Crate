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

fn field_text(object: &Value, key: &str) -> String {
    object
        .get(key)
        .map_or_else(|| "undefined".to_owned(), codec::js_string)
}

fn nonblank(value: &Value) -> Option<Value> {
    value
        .as_str()
        .filter(|text| !codec::trim(text).is_empty())
        .map(|_| value.clone())
}

fn finite_int(raw: &str) -> Result<Value, Error> {
    let raw = codec::trim(raw);
    let end = raw
        .char_indices()
        .skip(usize::from(raw.starts_with(['+', '-'])))
        .find(|(_, c)| !c.is_ascii_digit())
        .map_or(raw.len(), |(index, _)| index);
    // shortcut: non-finite numbers need the M3 script/value bridge to retain their identity.
    let number = raw[..end]
        .parse::<f64>()
        .ok()
        .filter(|number| number.is_finite())
        .ok_or(Error::Pending(
            "Non-finite proxy numeric fields are not implemented yet",
        ))?;
    Ok(json!(number))
}

fn safe_integer(raw: &str) -> Option<u64> {
    if raw.is_empty() || !raw.bytes().all(|c| c.is_ascii_digit()) {
        return None;
    }
    raw.parse()
        .ok()
        .filter(|number| *number <= 9_007_199_254_740_991)
}

fn path_ed(path: &str) -> (String, Option<String>) {
    let Some((base, query)) = path.split_once('?') else {
        return (path.to_owned(), None);
    };
    let mut value = None;
    let mut kept = Vec::new();
    for part in query.split('&').filter(|part| !part.is_empty()) {
        let (key, raw) = part.split_once('=').unwrap_or((part, ""));
        let decode =
            |text: &str| component(&text.replace('+', "%20")).unwrap_or_else(|| text.to_owned());
        if decode(key) == "ed" {
            let raw = decode(raw);
            if value.is_none() && !raw.is_empty() {
                value = Some(raw);
            }
        } else {
            kept.push(part);
        }
    }
    if value.as_deref().and_then(safe_integer).is_none() {
        return (path.to_owned(), None);
    }
    let path = if kept.is_empty() {
        base.to_owned()
    } else {
        format!("{base}?{}", kept.join("&"))
    };
    (path, value)
}

fn normalized_port(raw: &str) -> Value {
    match raw.parse::<u16>() {
        Ok(number)
            if raw.bytes().all(|byte| byte.is_ascii_digit())
                && (raw.len() <= 4
                    || (raw.len() == 5
                        && (!raw.starts_with('0') || raw.bytes().all(|byte| byte <= b'5')))) =>
        {
            json!(number)
        }
        _ => json!(raw),
    }
}

fn normalize_endpoint(proxy: &mut Value) -> Result<(), Error> {
    if let Some(server) = proxy["server"].as_str() {
        let server = codec::trim(server);
        let server = server.strip_prefix('[').unwrap_or(server);
        proxy["server"] = json!(server.strip_suffix(']').unwrap_or(server));
    }
    normalize(proxy)
}

fn ipv6(address: &str) -> bool {
    if let Some((host, zone)) = address.split_once('%') {
        let Some(rest) = host.strip_prefix("fe80:") else {
            return false;
        };
        return !zone.is_empty()
            && zone.bytes().all(|byte| byte.is_ascii_alphanumeric())
            && (rest.is_empty()
                || (rest.starts_with(':')
                    && rest[1..].split(':').count() <= 4
                    && rest[1..].split(':').all(|part| {
                        part.len() <= 4 && part.bytes().all(|byte| byte.is_ascii_hexdigit())
                    })));
    }
    if address.contains('.') {
        let Some((prefix, tail)) = address.rsplit_once(':') else {
            return false;
        };
        let prefix = format!("{prefix}:");
        let compressed = prefix.strip_suffix("::").is_some_and(|parts| {
            let parts: Vec<_> = parts.split(':').collect();
            (1..=4).contains(&parts.len())
                && parts.iter().all(|part| {
                    !part.is_empty()
                        && part.len() <= 4
                        && part.bytes().all(|byte| byte.is_ascii_hexdigit())
                })
        });
        let mapped = prefix == "::"
            || prefix == "::ffff:"
            || prefix.strip_prefix("::ffff:").is_some_and(|rest| {
                rest.strip_suffix(':').is_some_and(|zeros| {
                    !zeros.is_empty() && zeros.len() <= 4 && zeros.bytes().all(|byte| byte == b'0')
                })
            });
        let parts: Vec<_> = tail.split('.').collect();
        return (compressed || mapped)
            && parts.len() == 4
            && parts.iter().all(|part| {
                !part.is_empty()
                    && part.len() <= 3
                    && part.bytes().all(|byte| byte.is_ascii_digit())
                    && (part.len() < 3 || !part.starts_with('0'))
                    && part.parse::<u8>().is_ok()
            });
    }
    address.parse::<Ipv6Addr>().is_ok()
}

pub(crate) fn is_ip(address: &str) -> bool {
    address.parse::<std::net::Ipv4Addr>().is_ok() || ipv6(address)
}

fn url_parts(raw: &str) -> (&str, &str, Option<&str>) {
    let (raw, name) = raw
        .split_once('#')
        .map_or((raw, None), |(raw, name)| (raw, Some(name)));
    let (address, query) = raw.split_once('?').unwrap_or((raw, ""));
    (address.strip_suffix('/').unwrap_or(address), query, name)
}

fn endpoint(address: &str, default_port: u16) -> Result<(&str, Value), Error> {
    match address.rsplit_once(':') {
        Some((server, port))
            if !port.is_empty() && port.bytes().all(|byte| byte.is_ascii_digit()) =>
        {
            Ok((server, finite_int(port)?))
        }
        _ => Ok((address, json!(default_port))),
    }
}

fn flag(value: &Value) -> bool {
    let value = codec::js_string(value).to_lowercase();
    value.contains("true") || value.contains('1')
}

fn random_port(raw: &str) -> Result<Value, Error> {
    let random = || {
        let mut bytes = [0; 8];
        getrandom::fill(&mut bytes).map_err(|error| Error::Legacy(error.to_string()))?;
        Ok::<f64, Error>((u64::from_le_bytes(bytes) >> 11) as f64 / 9_007_199_254_740_992.0)
    };
    let parts: Vec<_> = raw.split([',', '/']).collect();
    let part = parts[(random()? * parts.len() as f64).floor() as usize];
    let number = |raw: &str| codec::to_number(&json!(raw));
    let port = if let Some((min, remaining)) = part.split_once('-') {
        let min = number(min).ceil();
        let max = number(remaining.split('-').next().unwrap()).floor();
        (random()? * (max - min + 1.0)).floor() + min
    } else {
        number(part)
    };
    if !port.is_finite() {
        return Err(Error::Pending(
            "Non-finite proxy numeric fields are not implemented yet",
        ));
    }
    Ok(json!(port))
}

fn interface_address(raw: &str) -> Option<(&str, Option<u32>, bool)> {
    let raw = codec::trim(raw);
    let (host, cidr) = raw
        .rsplit_once('/')
        .filter(|(_, cidr)| !cidr.is_empty() && cidr.bytes().all(|byte| byte.is_ascii_digit()))
        .map_or((raw, None), |(host, cidr)| (host, Some(cidr)));
    let host = codec::trim(host);
    let host = host.strip_prefix('[').unwrap_or(host);
    let host = host.strip_suffix(']').unwrap_or(host);
    let v6 = ipv6(host);
    if !v6 && host.parse::<std::net::Ipv4Addr>().is_err() {
        return None;
    }
    let cidr = cidr
        .and_then(|cidr| cidr.parse::<u32>().ok())
        .filter(|cidr| *cidr <= if v6 { 128 } else { 32 });
    Some((host, cidr, v6))
}

fn normalize_wireguard(proxy: &mut Value) {
    for (address, cidr_key, max) in [("ip", "ip-cidr", 32), ("ipv6", "ipv6-cidr", 128)] {
        let raw = field_text(proxy, address);
        let Some((host, parsed_cidr, v6)) = interface_address(&raw) else {
            if proxy[address].is_null() || codec::trim(&raw).is_empty() {
                proxy.as_object_mut().unwrap().shift_remove(cidr_key);
            }
            continue;
        };
        if v6 != (max == 128) {
            continue;
        }
        let cidr = safe_integer(codec::trim(&field_text(proxy, cidr_key)))
            .filter(|cidr| *cidr <= max)
            .map(|cidr| cidr as u32)
            .or(parsed_cidr)
            .unwrap_or(max as u32);
        proxy[address] = json!(host);
        proxy[cidr_key] = json!(cidr);
    }
}

fn ech_options(raw: &str) -> Option<Value> {
    if codec::trim(raw).is_empty() {
        return None;
    }
    if !raw.contains("://") {
        return Some(json!({"enable":true, "config":raw}));
    }
    let parts: Vec<_> = raw.split('+').collect();
    match parts.as_slice() {
        [dns] if !codec::trim(dns).is_empty() => Some(json!({"enable":true, "_dns":dns})),
        [name, dns] if !codec::trim(name).is_empty() && !codec::trim(dns).is_empty() => {
            Some(json!({"enable":true, "_dns":dns, "query-server-name":name}))
        }
        _ => None,
    }
}

fn vmess_security(value: &Value) -> Value {
    let cipher = value.as_str().map(codec::trim).unwrap_or("").to_lowercase();
    json!(match cipher.as_str() {
        "auto" | "none" | "zero" | "aes-128-gcm" | "chacha20-poly1305" => cipher,
        "chacha20-ietf-poly1305" => "chacha20-poly1305".to_owned(),
        _ => "auto".to_owned(),
    })
}

fn uri_params(raw: &str, rest: bool) -> Option<Value> {
    let mut result = json!({});
    for part in raw.split('&').filter(|part| !part.is_empty()) {
        let (key, value) = part
            .split_once('=')
            .unwrap_or((part, if rest { "" } else { "undefined" }));
        if key == "__proto__" {
            continue;
        }
        let value = if rest {
            value
        } else {
            value.split('=').next().unwrap_or("")
        };
        result[codec::encode(key)] = json!(component(value)?);
    }
    Some(result)
}

fn uri_hosts(value: &Value) -> Option<Value> {
    if let Some(values) = value.as_array() {
        let values: Vec<_> = values
            .iter()
            .filter_map(uri_hosts)
            .flat_map(|value| value.as_array().cloned().unwrap_or_default())
            .collect();
        return (!values.is_empty()).then(|| json!(values));
    }
    let Some(raw) = value.as_str() else {
        return (!value.is_null()).then(|| json!([value]));
    };
    let values: Vec<_> = raw
        .split(',')
        .map(codec::trim)
        .filter(|raw| !raw.is_empty())
        .collect();
    (!values.is_empty()).then(|| json!(values))
}

fn vless(line: &str, protocol: &str) -> Result<Option<Value>, Error> {
    let raw = line.split_once("://").map_or(line, |(_, raw)| raw);
    let valid_endpoint = |raw: &str| {
        let (address, _, _) = url_parts(raw);
        let (_, endpoint) = address.split_once('@')?;
        let (_, port) = endpoint.rsplit_once(':')?;
        (!port.is_empty() && port.bytes().all(|byte| byte.is_ascii_digit())).then_some(())
    };
    let mut shadowrocket = false;
    let decoded_line;
    let raw = if valid_endpoint(raw).is_some() {
        raw
    } else {
        let Some((base, other)) = raw.split_once('?') else {
            return Ok(None);
        };
        let Some(base) = decoded(base) else {
            return Ok(None);
        };
        decoded_line = format!("{base}?{other}");
        if valid_endpoint(&decoded_line).is_none() {
            return Ok(None);
        }
        shadowrocket = true;
        &decoded_line
    };
    let (address, query, fragment) = url_parts(raw);
    let Some((auth, endpoint)) = address.split_once('@') else {
        return Ok(None);
    };
    let Some((server, port)) = endpoint.rsplit_once(':') else {
        return Ok(None);
    };
    let Some(uuid) = component(if shadowrocket {
        auth.rsplit(':').next().unwrap_or(auth)
    } else {
        auth
    }) else {
        return Ok(None);
    };
    let name = match fragment.map(component) {
        Some(Some(name)) => Some(name),
        Some(None) => return Ok(None),
        None => None,
    };
    let Some(mut params) = uri_params(query, true) else {
        return Ok(None);
    };
    let port = finite_int(port)?;
    let name = name
        .map(Value::String)
        .or_else(|| params.get("remarks").cloned())
        .or_else(|| params.get("remark").cloned())
        .unwrap_or_else(|| {
            json!(format!(
                "{} {server}:{}",
                if protocol == "vmess" {
                    "VMess"
                } else {
                    "VLESS"
                },
                codec::js_string(&port)
            ))
        });
    let mut proxy = json!({
        "type":protocol, "name":name, "server":codec::encode(server),
        "port":port, "uuid":uuid, "udp":true,
        "tls":null, "sni":null, "flow":null, "client-fingerprint":null, "alpn":null,
        "skip-cert-verify":false, "_echConfigList":null
    });
    if let Some(security) = params.get("security") {
        proxy["tls"] = if truthy(security) {
            json!(security != "none")
        } else {
            security.clone()
        };
    }
    if truthy(&params["pbk"]) {
        params["security"] = json!("reality");
    }
    if shadowrocket && flag(&params["tls"]) {
        proxy["tls"] = json!(true);
        if params["security"].is_null() {
            params["security"] = json!("reality");
        }
    }
    let sni = if truthy(&params["sni"]) {
        params.get("sni")
    } else {
        params.get("peer")
    };
    if let Some(value) = sni {
        proxy["sni"] = value.clone();
    }
    if let Some(value) = params.get("flow") {
        proxy["flow"] = value.clone();
    }
    if !truthy(&proxy["flow"]) && shadowrocket {
        match params["xtls"].as_str() {
            Some("1") => proxy["flow"] = json!("xtls-rprx-direct"),
            Some("2") => proxy["flow"] = json!("xtls-rprx-vision"),
            _ => {}
        }
    }
    if let Some(value) = params.get("fp") {
        proxy["client-fingerprint"] = value.clone();
    }
    if truthy(&params["alpn"]) {
        proxy["alpn"] = json!(
            params["alpn"]
                .as_str()
                .unwrap()
                .split(',')
                .collect::<Vec<_>>()
        );
    }
    proxy["skip-cert-verify"] = json!(flag(&params["allowInsecure"]));
    if let Some(value) = params.get("ech") {
        proxy["_echConfigList"] = value.clone();
    }
    if let Some(value) = crate::xhttp::ech_options(&json!({"echConfigList":params["ech"]})) {
        proxy["ech-opts"] = value;
    }
    for key in ["tls-fingerprint", "_vcn", "name-cert-verify"] {
        proxy[key] = Value::Null;
    }
    if let Some(value) = params.get("pcs") {
        proxy["tls-fingerprint"] = value.clone();
    }
    if let Some(raw) = params["vcn"].as_str() {
        let values: Vec<_> = raw
            .split(',')
            .map(codec::trim)
            .filter(|raw| !raw.is_empty())
            .collect();
        proxy["_vcn"] = json!(values);
        if let Some(name) = values.first() {
            proxy["name-cert-verify"] = json!(name);
        }
    }
    proxy["_h2"] = json!(flag(&params["h2"]));
    proxy["packet-encoding"] = json!(match params["packetEncoding"]
        .as_str()
        .map(codec::trim)
        .map(str::to_lowercase)
        .as_deref()
    {
        Some("none") => "",
        Some("packet") => "packetaddr",
        _ => "xudp",
    });
    if params["security"] == "reality" {
        let mut options = json!({});
        if truthy(&params["pbk"]) {
            options["public-key"] = params["pbk"].clone();
            if matches!(
                params["support-x25519mlkem768"].as_str(),
                Some("1" | "t" | "T" | "true" | "TRUE" | "True")
            ) {
                options["support-x25519mlkem768"] = json!(true);
            }
        }
        for (param, field) in [("sid", "short-id"), ("spx", "_spider-x")] {
            if truthy(&params[param]) {
                options[field] = params[param].clone();
            }
        }
        if !options.as_object().unwrap().is_empty() {
            proxy["reality-opts"] = options;
        }
    }
    let raw_network = params["type"]
        .as_str()
        .filter(|raw| !raw.is_empty())
        .unwrap_or("tcp");
    let upgrade = raw_network == "httpupgrade";
    let mut network = match raw_network {
        "tcp" if params["headerType"] == "http" => "http",
        "http" => "h2",
        "httpupgrade" => "ws",
        other => other,
    }
    .to_owned();
    if !truthy(&params["type"]) && shadowrocket && truthy(&params["obfs"]) {
        network = field_text(&params, "obfs");
        if network == "none" {
            network = "tcp".to_owned();
        }
    }
    if network == "websocket" {
        network = "ws".to_owned();
    }
    proxy["network"] = json!(network);
    if !["tcp", "none", ""].contains(&network.as_str()) {
        let mut options = json!({});
        let host = params.get("host").unwrap_or(&params["obfsParam"]);
        if truthy(host) {
            options["headers"] = if truthy(&params["obfsParam"]) {
                host.as_str()
                    .and_then(|raw| codec::parse(&codec::text(raw)).ok())
                    .unwrap_or_else(|| json!({"Host":host}))
            } else {
                json!({"Host":host})
            };
            if !options["headers"].is_null() && !options["headers"].is_object() {
                // shortcut: non-object headers need the production script/value bridge.
                return Err(Error::Pending(
                    "Non-object transport headers are not implemented yet",
                ));
            }
            if network == "xhttp" && truthy(&options["headers"]["Host"]) {
                options["host"] = options["headers"]["Host"].clone();
                options["headers"]
                    .as_object_mut()
                    .unwrap()
                    .shift_remove("Host");
                if options["headers"].as_object().is_some_and(Map::is_empty) {
                    options.as_object_mut().unwrap().shift_remove("headers");
                }
            }
            let h2_host = options["headers"]
                .get("Host")
                .unwrap_or(&options["headers"]["host"])
                .clone();
            if network == "h2" && truthy(&h2_host) {
                if let Some(hosts) = uri_hosts(&h2_host) {
                    options["host"] = hosts;
                }
                if let Some(headers) = options["headers"].as_object_mut() {
                    headers.shift_remove("Host");
                    headers.shift_remove("host");
                    if headers.is_empty() {
                        options.as_object_mut().unwrap().shift_remove("headers");
                    }
                }
            }
        }
        if truthy(&params["serviceName"]) {
            options[format!("{network}-service-name")] = params["serviceName"].clone();
            if network == "grpc" && truthy(&params["authority"]) {
                options["_grpc-authority"] = params["authority"].clone();
            }
        } else if shadowrocket
            && truthy(&params["path"])
            && !["ws", "http", "h2"].contains(&network.as_str())
        {
            options[format!("{network}-service-name")] = params["path"].clone();
            params.as_object_mut().unwrap().shift_remove("path");
        }
        let mut path_early = None;
        if truthy(&params["path"]) {
            let mut path = field_text(&params, "path");
            if network == "ws" {
                let (clean, early) = path_ed(&path);
                path = clean;
                path_early = early;
            }
            options["path"] = json!(path);
        } else if network == "h2" {
            options["path"] = json!("/");
        }
        if network == "http" && truthy(&params["method"]) {
            options["method"] = params["method"].clone();
        }
        if network == "grpc" {
            options["_grpc-type"] = if truthy(&params["mode"]) {
                params["mode"].clone()
            } else {
                json!("gun")
            };
        }
        if upgrade {
            options["v2ray-http-upgrade"] = json!(true);
        }
        let early = path_early.or_else(|| {
            params["ed"]
                .as_str()
                .filter(|raw| !raw.is_empty())
                .map(str::to_owned)
        });
        if let Some(raw) = early {
            if upgrade {
                options["v2ray-http-upgrade-fast-open"] = json!(true);
                options["_v2ray-http-upgrade-ed"] = json!(raw);
            } else if network == "ws" {
                if safe_integer(&raw).is_none() {
                    return Ok(None);
                }
                let max = finite_int(&raw)?;
                options["max-early-data"] = max;
                options["early-data-header-name"] = if truthy(&params["eh"]) {
                    params["eh"].clone()
                } else {
                    json!("Sec-WebSocket-Protocol")
                };
            }
        }
        if network == "ws" && truthy(&params["eh"]) {
            options["early-data-header-name"] = params["eh"].clone();
        }
        if !options.as_object().unwrap().is_empty() {
            proxy[format!("{network}-opts")] = options;
        }
        if network == "kcp" {
            if truthy(&params["seed"]) {
                proxy["seed"] = params["seed"].clone();
            }
            proxy["headerType"] = if truthy(&params["headerType"]) {
                params["headerType"].clone()
            } else {
                json!("none")
            };
        }
        if network == "xhttp" {
            let extra = params["extra"]
                .as_str()
                .map(codec::text)
                .unwrap_or_default();
            crate::xhttp::parse_extra(&mut proxy, &extra, &params["mode"]);
        } else {
            for (param, field) in [("extra", "_extra"), ("mode", "_mode")] {
                if truthy(&params[param]) {
                    proxy[field] = params[param].clone();
                }
            }
        }
    }
    if protocol == "vmess" {
        proxy["cipher"] = vmess_security(&params["encryption"]);
        proxy["alterId"] = json!(0);
        proxy.as_object_mut().unwrap().shift_remove("flow");
    } else if truthy(&params["encryption"]) {
        proxy["encryption"] = params["encryption"].clone();
    }
    if truthy(&params["pqv"]) {
        proxy["_pqv"] = params["pqv"].clone();
    }
    if let Some(raw) = params["fm"].as_str().filter(|raw| !raw.is_empty()) {
        proxy["_finalmask"] = codec::parse(&codec::text(raw))
            .ok()
            .filter(Value::is_object)
            .unwrap_or_else(|| json!(raw));
    }
    Ok(Some(proxy))
}

fn anytls(line: &str) -> Result<Option<Value>, Error> {
    let Some(mut proxy) = vless(line, "vless")? else {
        return Ok(None);
    };
    let raw = line.strip_prefix("anytls://").unwrap();
    let (address, query, fragment) = url_parts(raw);
    let Some((auth, endpoint)) = address.split_once('@') else {
        return Ok(None);
    };
    let Some((server, port)) = endpoint.rsplit_once(':') else {
        return Ok(None);
    };
    let Some(password) = component(auth).and_then(|raw| component(&codec::text(&raw))) else {
        return Ok(None);
    };
    let name = if let Some(raw) = fragment {
        let Some(value) = component(raw) else {
            return Ok(None);
        };
        value
    } else {
        codec::encode(&format!(
            "AnyTLS {server}:{}",
            codec::js_string(&proxy["port"])
        ))
    };
    proxy["uuid"] = Value::Null;
    proxy["type"] = json!("anytls");
    proxy["name"] = json!(name);
    proxy["server"] = json!(codec::encode(server));
    proxy["port"] = finite_int(port)?;
    proxy["password"] = json!(password);
    for part in query.split('&').filter(|raw| !raw.is_empty()) {
        let (key, value) = part.split_once('=').unwrap_or((part, "undefined"));
        let key = codec::encode(&key.replace('_', "-"));
        if key == "__proto__" {
            continue;
        }
        let Some(value) = component(value.split('=').next().unwrap_or("")) else {
            return Ok(None);
        };
        match key.as_str() {
            "alpn" => {
                if value.is_empty() {
                    proxy.as_object_mut().unwrap().shift_remove("alpn");
                } else {
                    proxy["alpn"] = json!(value.split(',').collect::<Vec<_>>());
                }
            }
            "insecure" => proxy["skip-cert-verify"] = json!(flag(&json!(value))),
            "udp" => proxy["udp"] = json!(flag(&json!(value))),
            _ => {
                proxy
                    .as_object_mut()
                    .unwrap()
                    .entry(key)
                    .or_insert(json!(value));
            }
        }
    }
    if proxy["network"] == "tcp" && !truthy(&proxy["reality-opts"]) {
        proxy.as_object_mut().unwrap().shift_remove("network");
        proxy.as_object_mut().unwrap().shift_remove("security");
    }
    Ok(Some(proxy))
}

fn vmess(line: &str) -> Result<Option<Value>, Error> {
    let raw = line.strip_prefix("vmess://").unwrap();
    if raw
        .find('@')
        .is_some_and(|at| !raw[..at].contains(['/', '?', '#']))
    {
        let Some(mut proxy) = vless(line, "vmess")? else {
            return Ok(None);
        };
        normalize_endpoint(&mut proxy)?;
        proxy
            .as_object_mut()
            .unwrap()
            .retain(|_, value| !value.is_null());
        return Ok(Some(proxy));
    }
    let (without_fragment, fragment) = raw
        .split_once('#')
        .map_or((raw, None), |(raw, name)| (raw, Some(name)));
    let fragment = match fragment.map(component) {
        Some(Some(name)) => Some(name),
        Some(None) => return Ok(None),
        None => None,
    };
    let Some(content) = decoded(without_fragment.split('?').next().unwrap_or("")) else {
        return Ok(None);
    };
    if content
        .split_once('=')
        .is_some_and(|(_, rest)| rest.trim_start().starts_with("vmess"))
    {
        let parts: Vec<_> = content.split(',').map(codec::trim).collect();
        if parts.len() < 5 {
            return Ok(None);
        }
        let Some(uuid) = parts[4]
            .strip_prefix('"')
            .and_then(|raw| raw.strip_suffix('"'))
        else {
            return Ok(None);
        };
        let mut params = json!({});
        for part in &parts {
            if let Some((key, value)) = part.split_once('=') {
                params[codec::encode(codec::trim(key))] = json!(codec::encode(codec::trim(
                    value.split('=').next().unwrap_or("")
                )));
            }
        }
        // The frozen Quantumult branch rejects obfs before it can build ws-opts.
        if params.get("obfs").is_some() {
            return Ok(None);
        }
        let mut proxy = json!({
            "name":codec::encode(codec::trim(parts[0].split('=').next().unwrap_or(""))),
            "type":"vmess", "server":codec::encode(parts[1]), "port":codec::encode(parts[2]),
            "cipher":vmess_security(&json!(parts[3])), "uuid":codec::encode(uuid), "tls":false,
            "udp":null, "tfo":null, "skip-cert-verify":null
        });
        for (param, field) in [("udp-relay", "udp"), ("fast-open", "tfo")] {
            if let Some(value) = params.get(param) {
                proxy[field] = value.clone();
            }
        }
        if let Some(value) = params.get("tls-verification") {
            proxy["skip-cert-verify"] = json!(!truthy(value));
        }
        if let Some(name) = fragment.filter(|raw| !codec::trim(raw).is_empty()) {
            proxy["name"] = json!(name);
        }
        normalize_endpoint(&mut proxy)?;
        proxy
            .as_object_mut()
            .unwrap()
            .retain(|_, value| !value.is_null());
        return Ok(Some(proxy));
    }
    let params = codec::parse(&content);
    let params = if let Ok(value) = params {
        if !value.is_object() {
            return Ok(None);
        }
        value
    } else {
        let Some((base, query)) = without_fragment.split_once('?') else {
            return Ok(None);
        };
        let Some(content) = decoded(base.strip_suffix('/').unwrap_or(base)) else {
            return Ok(None);
        };
        let Some((auth, endpoint)) = content.split_once('@') else {
            return Ok(None);
        };
        let Some((cipher, uuid)) = auth.split_once(':') else {
            return Ok(None);
        };
        let Some((server, port)) = endpoint.rsplit_once(':') else {
            return Ok(None);
        };
        if port.is_empty() || !port.bytes().all(|byte| byte.is_ascii_digit()) {
            return Ok(None);
        }
        let Some(mut params) = uri_params(query, false) else {
            return Ok(None);
        };
        for value in params.as_object_mut().unwrap().values_mut() {
            if let Some(raw) = value.as_str().filter(|raw| raw.contains(',')) {
                *value = json!(raw.split(',').collect::<Vec<_>>());
            }
        }
        params["scy"] = json!(codec::encode(cipher));
        params["id"] = json!(codec::encode(uuid));
        params["port"] = json!(port);
        params["add"] = json!(codec::encode(server));
        params
    };
    let port = finite_int(&field_text(&params, "port"))?;
    let name = ["ps", "remarks", "remark"]
        .iter()
        .find_map(|key| params.get(*key).filter(|value| !value.is_null()).cloned())
        .unwrap_or_else(|| {
            json!(format!(
                "VMess {}:{}",
                field_text(&params, "add"),
                codec::js_string(&port)
            ))
        });
    let alter_id = finite_int(
        &params
            .get("aid")
            .filter(|value| !value.is_null())
            .or_else(|| params.get("alterId").filter(|value| !value.is_null()))
            .map(codec::js_string)
            .unwrap_or_else(|| "0".to_owned()),
    )?;
    let tls = params["tls"] == "tls"
        || params["tls"] == true
        || params["tls"] == "1"
        || params["tls"].as_f64() == Some(1.0);
    let mut proxy = json!({"name":name,"type":"vmess","server":null});
    if let Some(value) = params.get("add") {
        proxy["server"] = value.clone();
    }
    proxy["port"] = port;
    proxy["cipher"] = vmess_security(&params["scy"]);
    proxy["uuid"] = Value::Null;
    if let Some(value) = params.get("id") {
        proxy["uuid"] = value.clone();
    }
    proxy["alterId"] = alter_id;
    proxy["tls"] = json!(tls);
    proxy["skip-cert-verify"] = Value::Null;
    if let Some(value) = params.get("verify_cert").filter(|value| !value.is_null()) {
        proxy["skip-cert-verify"] = json!(!truthy(value));
    }
    if !truthy(&proxy["skip-cert-verify"])
        && let Some(value) = params.get("allowInsecure").filter(|value| !value.is_null())
    {
        proxy["skip-cert-verify"] = json!(flag(value));
    }
    if truthy(&proxy["tls"]) {
        if truthy(&params["sni"]) {
            proxy["sni"] = params["sni"].clone();
        } else if truthy(&params["peer"]) {
            proxy["sni"] = params["peer"].clone();
        }
    }
    let (network, upgrade) = if params["net"] == "ws" || params["obfs"] == "websocket" {
        (Some("ws"), false)
    } else if params["obfs"] == "http" || params["type"] == "http" {
        (Some("http"), false)
    } else if params["net"] == "http" || params["net"] == "h2" {
        (Some("h2"), false)
    } else if matches!(params["net"].as_str(), Some("grpc" | "kcp" | "quic")) {
        (params["net"].as_str(), false)
    } else if params["net"] == "httpupgrade" {
        (Some("ws"), true)
    } else {
        (None, false)
    };
    if let Some(network) = network {
        proxy["network"] = json!(network);
        let mut host = params.get("host").unwrap_or(&params["obfsParam"]).clone();
        if let Some(parsed) = host
            .as_str()
            .and_then(|raw| codec::parse(&codec::text(raw)).ok())
            && truthy(&parsed["Host"])
        {
            host = parsed["Host"].clone();
        }
        let mut path = params["path"].clone();
        let mut early = None;
        if network == "ws" && truthy(&path) {
            let Some(raw) = path.as_str() else {
                return Ok(None);
            };
            let (raw, ed) = path_ed(raw);
            path = json!(raw);
            early = ed;
        }
        if ["ws", "http", "h2"].contains(&network) && !truthy(&path) {
            path = json!("/");
        }
        if network == "http" {
            if truthy(&host) {
                let Some(raw) = host.as_str() else {
                    return Ok(None);
                };
                host = json!(codec::trim(raw.split(',').next().unwrap_or("")));
            }
            if let Some(values) = path.as_array() {
                path = values.first().cloned().unwrap_or(Value::Null);
            }
        }
        if truthy(&path) || truthy(&host) || ["kcp", "quic"].contains(&network) {
            let mut options = json!({});
            if network == "grpc" {
                for (field, value) in [
                    ("grpc-service-name", &path),
                    ("_grpc-type", &params["type"]),
                    ("_grpc-authority", &params["authority"]),
                ] {
                    if let Some(value) = nonblank(value) {
                        options[field] = value;
                    }
                }
            } else if ["kcp", "quic"].contains(&network) {
                for (field, value) in [("type", &params["type"]), ("host", &host), ("path", &path)]
                {
                    if let Some(value) = nonblank(value) {
                        options[format!("_{network}-{field}")] = value;
                    }
                }
            } else {
                if let Some(value) = nonblank(&path) {
                    options["path"] = value;
                }
                if network == "h2" {
                    if let Some(value) = uri_hosts(&host) {
                        options["host"] = value;
                    }
                } else {
                    options["headers"] = json!({});
                    if let Some(value) = nonblank(&host) {
                        options["headers"]["Host"] = value;
                    }
                }
                if upgrade {
                    options["v2ray-http-upgrade"] = json!(true);
                    let early = early.or_else(|| {
                        params["ed"]
                            .as_str()
                            .filter(|raw| safe_integer(raw).is_some())
                            .map(str::to_owned)
                    });
                    if let Some(raw) = early {
                        options["v2ray-http-upgrade-fast-open"] = json!(true);
                        options["_v2ray-http-upgrade-ed"] = json!(raw);
                    }
                } else if network == "ws"
                    && let Some(raw) = early
                {
                    options["max-early-data"] = finite_int(&raw)?;
                    options["early-data-header-name"] = json!("Sec-WebSocket-Protocol");
                }
            }
            proxy[format!("{network}-opts")] = options;
        } else {
            proxy.as_object_mut().unwrap().shift_remove("network");
        }
    }
    proxy["client-fingerprint"] = params.get("fp").cloned().unwrap_or(Value::Null);
    proxy["alpn"] = Value::Null;
    if truthy(&params["alpn"]) {
        let Some(value) = params["alpn"].as_str() else {
            return Ok(None);
        };
        proxy["alpn"] = json!(value.split(',').collect::<Vec<_>>());
    }
    if let Some(name) = fragment.filter(|raw| !codec::trim(raw).is_empty()) {
        proxy["name"] = json!(name);
    }
    normalize_endpoint(&mut proxy)?;
    for (field, source) in [
        ("server", "add"),
        ("uuid", "id"),
        ("client-fingerprint", "fp"),
        ("alpn", "alpn"),
        ("skip-cert-verify", ""),
    ] {
        if proxy[field].is_null()
            && (source.is_empty() || !params.as_object().unwrap().contains_key(source))
        {
            proxy.as_object_mut().unwrap().shift_remove(field);
        }
    }
    if proxy["alpn"].is_null() && !truthy(&params["alpn"]) {
        proxy.as_object_mut().unwrap().shift_remove("alpn");
    }
    Ok(Some(proxy))
}

fn modern_uri(line: &str) -> Result<Option<Value>, Error> {
    let (scheme, raw) = line.split_once("://").unwrap();
    let (kind, label, default_port) = match scheme {
        "hy" | "hysteria" => ("hysteria", "Hysteria", 443),
        "hy2" | "hysteria2" => ("hysteria2", "Hysteria2", 443),
        "tuic" => ("tuic", "TUIC", 443),
        _ => ("wireguard", "WireGuard", 51820),
    };
    let (address, query, name) = url_parts(raw);
    let (auth, address) = if kind == "hysteria" {
        (None, address)
    } else if let Some((auth, address)) = address.split_once('@') {
        (Some(auth), address)
    } else if kind == "wireguard" {
        (None, address)
    } else {
        return Ok(None);
    };
    let mut ports = Value::Null;
    let (server, port) = if kind == "hysteria2"
        && let Some((server, range)) = address.rsplit_once(':')
        && range.split([',', ';']).all(|part| {
            let parts: Vec<_> = part.split('-').collect();
            (1..=2).contains(&parts.len())
                && parts
                    .iter()
                    .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
        })
        && !range.bytes().all(|byte| byte.is_ascii_digit())
    {
        ports = json!(range);
        (server, random_port(range)?)
    } else {
        endpoint(address, default_port)?
    };
    let name = match name {
        Some(raw) => match component(raw) {
            Some(name) => name,
            None => return Ok(None),
        },
        None => codec::encode(&format!("{label} {server}:{}", codec::js_string(&port))),
    };
    let mut proxy = json!({"type":kind, "name":name, "server":codec::encode(server), "port":port});
    match kind {
        "hysteria2" => {
            let Some(password) = component(auth.unwrap()) else {
                return Ok(None);
            };
            proxy["ports"] = ports;
            proxy["password"] = json!(password);
        }
        "tuic" => {
            let Some(auth) = component(auth.unwrap()) else {
                return Ok(None);
            };
            let (uuid, password) = auth.split_once(':').unwrap_or((&auth, ""));
            let Some(password) = component(&codec::text(password)) else {
                return Ok(None);
            };
            proxy["password"] = json!(password);
            proxy["uuid"] = json!(uuid);
        }
        "wireguard" => {
            let Some(key) = component(auth.unwrap_or("undefined")) else {
                return Ok(None);
            };
            proxy["private-key"] = json!(key);
            proxy["udp"] = json!(true);
        }
        _ => {}
    }
    let mut params = json!({});
    for part in query.split('&').filter(|part| !part.is_empty()) {
        let (key, raw) = if kind == "wireguard" {
            part.split_once('=').unwrap_or((part, ""))
        } else {
            let mut parts = part.split('=');
            (parts.next().unwrap(), parts.next().unwrap_or("undefined"))
        };
        let key = if kind == "tuic" {
            key.replace('_', "-")
        } else if kind == "hysteria" || kind == "wireguard" {
            key.replacen('_', "-", 1)
        } else {
            key.to_owned()
        };
        let key = codec::encode(&key);
        let Some(value) = component(raw) else {
            return Ok(None);
        };
        let value = json!(value);
        if key == "__proto__" {
            continue;
        }
        match kind {
            "hysteria2" => params[&key] = value,
            "hysteria" => match key.as_str() {
                "alpn" => {
                    proxy["alpn"] = if truthy(&value) {
                        json!(value.as_str().unwrap().split(',').collect::<Vec<_>>())
                    } else {
                        Value::Null
                    }
                }
                "insecure" => proxy["skip-cert-verify"] = json!(flag(&value)),
                "auth" => proxy["auth-str"] = value,
                "mport" => proxy["ports"] = value,
                "obfsParam" => proxy["obfs"] = value,
                "upmbps" => proxy["up"] = value,
                "downmbps" => proxy["down"] = value,
                "obfs" => proxy["_obfs"] = value,
                "fast-open" | "peer" => params[&key] = value,
                _ => {
                    if proxy.get(&key).is_none() {
                        proxy[&key] = value;
                    }
                }
            },
            "tuic" => match key.as_str() {
                "alpn" => {
                    proxy["alpn"] = if truthy(&value) {
                        json!(value.as_str().unwrap().split(',').collect::<Vec<_>>())
                    } else {
                        Value::Null
                    }
                }
                "allow-insecure" | "insecure" => proxy["skip-cert-verify"] = json!(flag(&value)),
                "fast-open" => proxy["tfo"] = json!(true),
                "disable-sni" | "reduce-rtt" => proxy[&key] = json!(flag(&value)),
                "congestion-control" => proxy["congestion-controller"] = value,
                _ => {
                    if proxy.get(&key).is_none() {
                        proxy[&key] = value;
                    }
                }
            },
            _ => match key.as_str() {
                "reserved" => {
                    let items: Vec<_> = value
                        .as_str()
                        .unwrap()
                        .split(',')
                        .filter_map(|raw| finite_int(raw).ok())
                        .collect();
                    if items.len() == 3 {
                        proxy["reserved"] = json!(items);
                    }
                }
                "address" | "ip" => {
                    for raw in value.as_str().unwrap().split(',') {
                        if let Some((host, cidr, v6)) = interface_address(raw) {
                            proxy[if v6 { "ipv6" } else { "ip" }] = json!(host);
                            if let Some(cidr) = cidr {
                                proxy[if v6 { "ipv6-cidr" } else { "ip-cidr" }] = json!(cidr);
                            }
                        }
                    }
                }
                "mtu" => {
                    if let Ok(mtu) = finite_int(value.as_str().unwrap()) {
                        proxy["mtu"] = mtu;
                    }
                }
                "udp" => proxy["udp"] = json!(flag(&value)),
                _ if key.to_lowercase().contains("publickey") => proxy["public-key"] = value,
                _ if key.to_lowercase().contains("privatekey") => proxy["private-key"] = value,
                _ => {
                    if key != "flag" && proxy.get(&key).is_none() {
                        proxy[&key] = value;
                    }
                }
            },
        }
    }
    if kind == "hysteria2" {
        proxy["sni"] = params["sni"].clone();
        if !truthy(&proxy["sni"]) && truthy(&params["peer"]) {
            proxy["sni"] = params["peer"].clone();
        }
        if truthy(&params["obfs"]) && params["obfs"] != "none" {
            proxy["obfs"] = params["obfs"].clone();
        }
        if truthy(&params["mport"]) {
            proxy["ports"] = params["mport"].clone();
        }
        proxy["obfs-password"] = params["obfs-password"].clone();
        proxy["skip-cert-verify"] = json!(flag(&params["insecure"]));
        proxy["tfo"] = json!(flag(&params["fastopen"]));
        proxy["tls-fingerprint"] = params["pinSHA256"].clone();
        let hop = if truthy(&params["hop-interval"]) {
            &params["hop-interval"]
        } else {
            &params["hop_interval"]
        };
        if !hop.is_null() {
            proxy["hop-interval"] = hop.clone();
        }
        if let Some(raw) = params["keepalive"].as_str()
            && !raw.is_empty()
            && raw.bytes().all(|byte| byte.is_ascii_digit())
        {
            proxy["keepalive"] = finite_int(raw)?;
        }
        for (source, key) in [("upmbps", "up"), ("downmbps", "down")] {
            if truthy(&params[source]) {
                proxy[key] = params[source].clone();
            }
        }
        if let Some(ech) = params["ech"].as_str().and_then(ech_options) {
            proxy["ech-opts"] = ech;
        }
    } else if kind == "hysteria" {
        if !truthy(&proxy["sni"]) && truthy(&params["peer"]) {
            proxy["sni"] = params["peer"].clone();
        }
        if !truthy(&proxy["protocol"]) {
            proxy["protocol"] = json!("udp");
        }
    }
    normalize_endpoint(&mut proxy)?;
    proxy
        .as_object_mut()
        .unwrap()
        .retain(|_, value| !value.is_null());
    if kind == "hysteria2" && truthy(&proxy["obfs"]) && !truthy(&proxy["obfs-password"]) {
        return Ok(None);
    }
    Ok(Some(proxy))
}

fn trojan(line: &str) -> Result<Option<Value>, Error> {
    let (before_query, query) = line
        .split_once('?')
        .map_or((line, None), |(raw, query)| (raw, Some(query)));
    if !before_query.contains('@') {
        return Ok(None);
    }
    let trailing_slash = before_query.ends_with('/');
    let base = before_query.strip_suffix('/').unwrap_or(before_query);
    let has_port = base.rsplit_once(':').is_some_and(|(_, port)| {
        !port.is_empty() && port.bytes().all(|byte| byte.is_ascii_digit())
    });
    let rewritten;
    let line = if has_port {
        line
    } else {
        rewritten = format!(
            "{base}:443{}{}",
            if trailing_slash { "/" } else { "" },
            query.map_or_else(String::new, |query| format!("?{query}"))
        );
        &rewritten
    };
    let (raw, name) = line
        .split_once('#')
        .map_or((line, None), |(raw, name)| (raw, Some(name)));
    let (address, query, _) = url_parts(raw.strip_prefix("trojan://").unwrap());
    let Some((password, address)) = address.split_once('@') else {
        return Ok(None);
    };
    if password.is_empty() || address.contains(['/', '#']) {
        return Ok(None);
    }
    let Some((server, port_raw)) = address.rsplit_once(':') else {
        return Ok(None);
    };
    let Ok(port) = port_raw.parse::<u16>() else {
        return Ok(None);
    };
    if server.is_empty()
        || port == 0
        || port_raw.is_empty()
        || !port_raw.bytes().all(|byte| byte.is_ascii_digit())
    {
        return Ok(None);
    }
    let host = server.strip_prefix('[').unwrap_or(server);
    let host = host.strip_suffix(']').unwrap_or(host);
    if (server.starts_with('[') || server.contains(':')) && !ipv6(host) {
        return Ok(None);
    }
    let decode = |raw: &str| component(raw).unwrap_or_else(|| codec::encode(raw));
    let mut proxy = json!({"type":"trojan", "password":decode(password), "server":codec::encode(server),
        "port":port, "name":codec::encode(&format!("{server}:{port_raw}"))});
    if let Some(name) = name.filter(|name| !codec::trim(name).is_empty())
        && let Some(name) = component(name)
    {
        proxy["name"] = json!(name);
    }
    let mut params = json!({});
    if !query.is_empty() {
        for part in query.split('&') {
            let (key, value) = part
                .split_once('=')
                .map_or((part, json!(true)), |(key, raw)| (key, json!(decode(raw))));
            if key != "__proto__" {
                let key = codec::encode(key);
                params[&key] = value;
            }
        }
    }
    let boolean = |key: &str| {
        if params[key].is_null() {
            Value::Null
        } else {
            json!(flag(&params[key]))
        }
    };
    proxy["skip-cert-verify"] = boolean("allowInsecure");
    proxy["sni"] = if truthy(&params["sni"]) {
        params["sni"].clone()
    } else {
        params["peer"].clone()
    };
    proxy["client-fingerprint"] = params["fp"].clone();
    proxy["tls-fingerprint"] = params["pcs"].clone();
    proxy["_vcn"] = match &params["vcn"] {
        Value::Null => Value::Null,
        Value::String(names) => json!(
            names
                .split(',')
                .map(codec::trim)
                .filter(|name| !name.is_empty())
                .collect::<Vec<_>>()
        ),
        _ => return Ok(None),
    };
    proxy["name-cert-verify"] = proxy["_vcn"]
        .as_array()
        .and_then(|names| names.first())
        .cloned()
        .unwrap_or(Value::Null);
    if truthy(&params["alpn"]) {
        let Some(alpn) = params["alpn"].as_str() else {
            return Ok(None);
        };
        proxy["alpn"] = json!(alpn.split(',').collect::<Vec<_>>());
    }
    if truthy(&boolean("ws")) {
        proxy["network"] = json!("ws");
        proxy["ws-opts"]["path"] = params["wspath"].clone();
    }
    if truthy(&params["type"]) {
        proxy["network"] = params["type"].clone();
        let upgrade = proxy["network"] == "httpupgrade";
        if upgrade {
            proxy["network"] = json!("ws");
        }
        let network = field_text(&proxy, "network");
        let key = format!("{network}-opts");
        if network == "grpc" {
            proxy[&key] = json!({"grpc-service-name":params["serviceName"], "_grpc-type":params["mode"],
                "_grpc-authority":params["authority"]});
        } else {
            if truthy(&params["path"]) || truthy(&params["host"]) {
                if format!("{network}-opts")
                    .split(['.', '[', ']'])
                    .any(|part| ["__proto__", "constructor", "prototype"].contains(&part))
                {
                    return Ok(None);
                }
                // shortcut: exotic nested transport property paths need the M3 value bridge.
                if network.contains(['.', '[', ']']) {
                    return Err(Error::Pending(
                        "Nested Trojan transport property paths are not implemented yet",
                    ));
                }
            }
            let mut early = None;
            if truthy(&params["path"]) {
                let path = if network == "ws" {
                    if let Some(path) = params["path"].as_str() {
                        let (path, ed) = path_ed(path);
                        early = ed;
                        json!(path)
                    } else {
                        params["path"].clone()
                    }
                } else {
                    params["path"].clone()
                };
                proxy[&key]["path"] = path;
            }
            if truthy(&params["host"]) {
                proxy[&key]["headers"]["Host"] = params["host"].clone();
            }
            if upgrade {
                let ed = early.or_else(|| {
                    let ed = field_text(&params, "ed");
                    safe_integer(&ed).map(|_| ed)
                });
                proxy["ws-opts"]["v2ray-http-upgrade"] = json!(true);
                if let Some(ed) = ed {
                    proxy["ws-opts"]["v2ray-http-upgrade-fast-open"] = json!(true);
                    proxy["ws-opts"]["_v2ray-http-upgrade-ed"] = json!(ed);
                }
            } else if network == "ws"
                && let Some(ed) = early
            {
                proxy["ws-opts"]["max-early-data"] = json!(safe_integer(&ed).unwrap());
                proxy["ws-opts"]["early-data-header-name"] = json!("Sec-WebSocket-Protocol");
            }
        }
        if params["security"] == "reality" {
            let mut options = json!({});
            for (source, field) in [
                ("pbk", "public-key"),
                ("sid", "short-id"),
                ("spx", "_spider-x"),
            ] {
                if truthy(&params[source]) {
                    options[field] = params[source].clone();
                }
            }
            for (source, field) in [("mode", "_mode"), ("extra", "_extra")] {
                if truthy(&params[source]) {
                    proxy[field] = params[source].clone();
                }
            }
            if !options.as_object().unwrap().is_empty() {
                proxy["reality-opts"] = options;
            }
        }
    }
    proxy["udp"] = boolean("udp");
    proxy["tfo"] = boolean("tfo");
    normalize_endpoint(&mut proxy)?;
    for value in proxy.as_object_mut().unwrap().values_mut() {
        if let Some(options) = value.as_object_mut() {
            options.retain(|_, value| !value.is_null());
        }
    }
    proxy
        .as_object_mut()
        .unwrap()
        .retain(|_, value| !value.is_null());
    Ok(Some(proxy))
}

fn proxy_uri(line: &str) -> Result<Option<Value>, Error> {
    let (scheme, raw) = line.split_once("://").unwrap();
    let (raw, fragment) = raw
        .split_once('#')
        .map_or((raw, None), |(raw, name)| (raw, Some(name)));
    let name = match fragment {
        Some(fragment) => match component(fragment) {
            Some(name) => Some(name),
            None => return Ok(None),
        },
        None => None,
    };
    let raw = raw.split('?').next().unwrap();
    let legacy = scheme == "socks";
    let raw = if legacy {
        raw
    } else {
        raw.strip_suffix('/').unwrap_or(raw)
    };
    let (auth, address) = if legacy {
        raw.rsplit_once('@')
            .map_or((None, raw), |(auth, address)| (Some(auth), address))
    } else {
        raw.split_once('@')
            .filter(|(auth, _)| auth.contains(':'))
            .map_or((None, raw), |(auth, address)| (Some(auth), address))
    };
    let (server, port) = address
        .rsplit_once(':')
        .filter(|(_, port)| !port.is_empty() && port.bytes().all(|byte| byte.is_ascii_digit()))
        .map_or((address, None), |(server, port)| (server, Some(port)));
    let tls = matches!(scheme, "https" | "socks5+tls");
    let port = if let Some(port) = port {
        finite_int(port)?
    } else if tls {
        json!(443)
    } else if scheme == "http" {
        json!(80)
    } else {
        return Ok(None);
    };
    let proxy_type = if matches!(scheme, "http" | "https") {
        "http"
    } else {
        "socks5"
    };
    let mut proxy = json!({"name":name.unwrap_or_else(|| codec::encode(&format!("{} {server}:{}", if legacy { "socks" } else { proxy_type }, codec::js_string(&port)))), "type":proxy_type});
    if !legacy {
        proxy["tls"] = json!(tls);
    }
    proxy["server"] = json!(codec::encode(server));
    proxy["port"] = port;
    if let Some(auth) = auth {
        if legacy {
            if !auth.is_empty() {
                let Some(auth) = component(auth).and_then(|auth| decoded(&auth)) else {
                    return Ok(None);
                };
                let mut credentials = auth.split(':');
                proxy["username"] = json!(codec::encode(credentials.next().unwrap()));
                if let Some(password) = credentials.next() {
                    proxy["password"] = json!(codec::encode(password));
                }
            }
        } else {
            let (username, password) = auth.split_once(':').unwrap();
            let (Some(username), Some(password)) = (component(username), component(password))
            else {
                return Ok(None);
            };
            proxy["username"] = json!(username);
            proxy["password"] = json!(password);
        }
    }
    proxy["udp"] = json!(true);
    normalize_endpoint(&mut proxy)?;
    Ok(Some(proxy))
}

fn ssr(line: &str) -> Result<Option<Value>, Error> {
    let Some(raw) = decoded(&line[6..]) else {
        return Ok(None);
    };
    let Some(index) = raw.find(":origin").or_else(|| raw.find(":auth_")) else {
        return Ok(None);
    };
    let Some((server, port)) = raw[..index].rsplit_once(':') else {
        return Ok(None);
    };
    let Some((protocol, parameters)) = raw[index + 1..].split_once("/?") else {
        return Ok(None);
    };
    let fields: Vec<_> = protocol.split(':').collect();
    if fields.len() < 4 {
        return Ok(None);
    }
    let Some(password) = decoded(fields[3]) else {
        return Ok(None);
    };
    let mut proxy = json!({"type":"ssr", "server":codec::encode(server), "port":port, "protocol":codec::encode(fields[0]), "cipher":codec::encode(&fields[1].to_lowercase()), "obfs":codec::encode(fields[2]), "password":codec::encode(&password)});
    let mut options = Map::new();
    let parameters: Vec<_> = parameters.split('&').collect();
    if parameters.len() > 1 {
        for parameter in parameters {
            let mut pair = parameter.split('=');
            let key = pair.next().unwrap();
            let Some(value) = pair.next() else {
                return Ok(None);
            };
            let value = codec::trim(value);
            if !value.is_empty() && value != "(null)" {
                options.insert(key.to_owned(), json!(value));
            }
        }
    }
    let options = Value::Object(options);
    proxy["name"] = if truthy(&options["remarks"]) {
        json!(codec::encode(
            &decoded(options["remarks"].as_str().unwrap()).unwrap_or_default()
        ))
    } else {
        proxy["server"].clone()
    };
    for (field, raw) in [
        (
            "protocol-param",
            options
                .get("protoparam")
                .filter(|value| truthy(value))
                .or_else(|| options.get("protocolparam")),
        ),
        ("obfs-param", options.get("obfsparam")),
    ] {
        let text = decoded(raw.and_then(Value::as_str).unwrap_or("")).unwrap_or_default();
        let text: String = text.chars().filter(|c| !codec::is_space(*c)).collect();
        if !text.is_empty() {
            proxy[field] = json!(codec::encode(&text));
        }
    }
    proxy["udp"] = json!(true);
    if port.is_empty() {
        return Err(Error::Pending(
            "Non-finite proxy numeric fields are not implemented yet",
        ));
    }
    proxy["port"] = normalized_port(port);
    normalize_endpoint(&mut proxy)?;
    Ok(Some(proxy))
}

fn ssd(raw: &str) -> Option<String> {
    let document = codec::parse(&decoded(raw.split("ssd://").nth(1)?)?).ok()?;
    let mut method = document.get("encryption");
    let mut password = document.get("password");
    let mut port = document.get("port");
    let mut lines = Vec::new();
    for (index, server) in document["servers"].as_array()?.iter().enumerate() {
        for (field, value) in [
            ("encryption", &mut method),
            ("password", &mut password),
            ("port", &mut port),
        ] {
            if let Some(next) = server.get(field).filter(|value| truthy(value)) {
                *value = Some(next);
            }
        }
        let text = |value: Option<&Value>| {
            codec::text(&value.map_or_else(|| "undefined".to_owned(), codec::js_string))
        };
        let auth = STANDARD.encode(format!("{}:{}", text(method), text(password)));
        let plugin = if truthy(&server["plugin_options"]) {
            format!(
                "/?plugin={}",
                codec::uri_component(&format!(
                    "{};{}",
                    field_text(server, "plugin"),
                    field_text(server, "plugin_options")
                ))
                .ok()?
            )
        } else {
            String::new()
        };
        let name = if truthy(&server["remarks"]) {
            text(server.get("remarks"))
        } else {
            index.to_string()
        };
        lines.push(format!(
            "ss://{auth}@{}:{}{plugin}#{name}",
            text(server.get("server")),
            text(port)
        ));
    }
    Some(lines.join("\n"))
}

fn preprocess(raw: &str) -> String {
    if raw.starts_with("ssd://") {
        return ssd(raw).unwrap_or_else(|| raw.to_owned());
    }
    let compact: String = raw.chars().filter(|c| !codec::is_space(*c)).collect();
    let unpadded = compact
        .strip_suffix("==")
        .or_else(|| compact.strip_suffix('='))
        .unwrap_or(&compact);
    let alphabet = |extra: &[u8]| {
        unpadded
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || extra.contains(&byte))
    };
    if (alphabet(b"+/") || alphabet(b"-_"))
        && let Some(text) = decoded(raw)
        && text
            .split(['\r', '\n', '\u{2028}', '\u{2029}'])
            .any(|line| {
                let word = |c: u8| c.is_ascii_alphanumeric() || c == b'_';
                let prefix = line.bytes().take_while(|byte| word(*byte)).count();
                if prefix == 0 {
                    return false;
                }
                let tail = &line[prefix..];
                tail.strip_prefix("://")
                    .is_some_and(|tail| tail.bytes().next().is_some_and(word))
                    || codec::trim(tail)
                        .strip_prefix('=')
                        .is_some_and(|tail| codec::trim(tail).bytes().next().is_some_and(word))
            })
    {
        return text;
    }
    crate::surge::full_config(raw).unwrap_or(raw).to_owned()
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
        if let Some(value) = query
            .split('&')
            .find_map(|part| part.strip_prefix("v2ray-plugin="))
        {
            let Some(options) = decoded(value).and_then(|text| codec::parse(&text).ok()) else {
                return Ok(None);
            };
            proxy["plugin"] = json!("v2ray-plugin");
            proxy["plugin-opts"] = options;
        }
        legacy = value;
        legacy.as_str()
    };
    let Some((auth, authority)) = (if is_legacy {
        content.rsplit_once('@')
    } else {
        content.split_once('@')
    }) else {
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
        if truthy(security) {
            json!(security != "none")
        } else {
            security.clone()
        }
    });
    proxy["skip-cert-verify"] = json!(truthy(&params["allowInsecure"]));
    proxy["sni"] = if truthy(&params["sni"]) {
        params["sni"].clone()
    } else {
        params["peer"].clone()
    };
    proxy["client-fingerprint"] = params["fp"].clone();
    proxy["alpn"] = if truthy(&params["alpn"]) {
        let Some(value) = component(&codec::text(params["alpn"].as_str().unwrap())) else {
            return Ok(None);
        };
        json!(value.split(',').collect::<Vec<_>>())
    } else {
        Value::Null
    };
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
            for (field, param) in [
                ("grpc-service-name", "serviceName"),
                ("_grpc-type", "mode"),
                ("_grpc-authority", "authority"),
            ] {
                if let Some(value) = params.get(param) {
                    opts[field] = value.clone();
                }
            }
            proxy[&key] = opts;
        } else {
            let mut early = None;
            if let Some(path) = params["path"].as_str().filter(|text| !text.is_empty()) {
                let path = if network == "ws" {
                    let (path, ed) = path_ed(path);
                    early = ed;
                    path
                } else {
                    path.to_owned()
                };
                opts["path"] = json!(path);
            }
            if let Some(host) = params["host"].as_str().filter(|text| !text.is_empty()) {
                let Some(host) = component(&codec::text(host)) else {
                    return Ok(None);
                };
                opts["headers"] = json!({"Host":host});
            }
            if upgrade {
                if early.is_none() && params["ed"].as_str().and_then(safe_integer).is_some() {
                    early = params["ed"].as_str().map(str::to_owned);
                }
                opts["v2ray-http-upgrade"] = json!(true);
                if let Some(ed) = early {
                    opts["v2ray-http-upgrade-fast-open"] = json!(true);
                    opts["_v2ray-http-upgrade-ed"] = json!(ed);
                }
            } else if network == "ws"
                && let Some(ed) = early
            {
                opts["max-early-data"] = json!(safe_integer(&ed).unwrap());
                opts["early-data-header-name"] = json!("Sec-WebSocket-Protocol");
            }
            if opts.as_object().is_some_and(|map| !map.is_empty()) {
                proxy[&key] = opts;
            }
        }
        if params["security"] == "reality" {
            let mut opts = Map::new();
            for (field, param) in [
                ("public-key", "pbk"),
                ("short-id", "sid"),
                ("_spider-x", "spx"),
            ] {
                if truthy(&params[param]) {
                    opts.insert(field.to_owned(), params[param].clone());
                }
            }
            for param in ["mode", "extra"] {
                if truthy(&params[param]) {
                    proxy[format!("_{param}")] = params[param].clone();
                }
            }
            if !opts.is_empty() {
                proxy["reality-opts"] = Value::Object(opts);
            }
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
    let mut name_port = if digits.is_empty() {
        "undefined".to_owned()
    } else {
        digits.clone()
    };
    if !digits.is_empty() {
        proxy["port"] = normalized_port(&digits);
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
                if let Some(mode) = options.get("obfs") {
                    plugin_opts["mode"] = mode.clone();
                }
                if let Some(host) = nonblank(&options["obfs-host"]) {
                    plugin_opts["host"] = host;
                }
            }
            "v2ray-plugin" => {
                proxy["plugin"] = json!("v2ray-plugin");
                plugin_opts["mode"] = nonblank(&options["obfs"])
                    .or_else(|| nonblank(&options["mode"]))
                    .unwrap_or_else(|| json!("websocket"));
                if let Some(host) =
                    nonblank(&options["obfs-host"]).or_else(|| nonblank(&options["host"]))
                {
                    plugin_opts["host"] = host;
                }
                if let Some(path) = nonblank(&options["path"]) {
                    plugin_opts["path"] = path;
                }
                for field in ["tls", "sni"] {
                    if let Some(value) = options.get(field) {
                        plugin_opts[field] = value.clone();
                    }
                }
                plugin_opts["skip-cert-verify"] = json!(
                    matches!(options["skip-cert-verify"].as_str(), Some("1" | "true"))
                        || options["skip-cert-verify"] == true
                        || options["skip-cert-verify"] == 1
                );
                if let Some(mux) = options["mux"]
                    .as_str()
                    .filter(|mux| !mux.is_empty() && mux.bytes().all(|c| c.is_ascii_digit()))
                {
                    plugin_opts["mux"] = finite_int(mux)?;
                }
            }
            "shadow-tls" => {
                proxy["plugin"] = json!("shadow-tls");
                for field in ["host", "password"] {
                    if let Some(value) = nonblank(&options[field]) {
                        plugin_opts[field] = value;
                    }
                }
                if let Some(version) = nonblank(&options["version"]) {
                    plugin_opts["version"] = finite_int(version.as_str().unwrap())?;
                }
            }
            _ => return Ok(None),
        }
        proxy["plugin-opts"] = plugin_opts;
    }
    for key in ["shadow-tls", "gost"] {
        if let Some(value) = query
            .split('&')
            .find_map(|part| part.strip_prefix(&format!("{key}=")))
        {
            let value = if key == "gost" {
                let Some(value) = component(value) else {
                    return Ok(None);
                };
                value
            } else {
                value.to_owned()
            };
            let Some(options) = decoded(&value).and_then(|text| codec::parse(&text).ok()) else {
                return Ok(None);
            };
            proxy["plugin"] = json!(if key == "gost" { "gost-plugin" } else { key });
            let mut opts = json!({});
            if key == "gost"
                && let Some(route) = nonblank(&options["route"])
            {
                let normalized = codec::trim(route.as_str().unwrap()).to_lowercase();
                opts["mode"] = if ["ws", "wss", "websocket"].contains(&normalized.as_str()) {
                    json!("websocket")
                } else {
                    route
                };
            }
            for field in ["host", if key == "gost" { "path" } else { "password" }] {
                if let Some(value) = nonblank(&options[field]) {
                    opts[field] = value;
                }
            }
            if key == "shadow-tls" {
                if let Some(version) = nonblank(&options["version"]) {
                    opts["version"] = finite_int(version.as_str().unwrap())?;
                }
            } else if options["route"]
                .as_str()
                .is_some_and(|route| codec::trim(route).eq_ignore_ascii_case("wss"))
            {
                opts["tls"] = json!(true);
            }
            proxy["plugin-opts"] = opts;
            if let Some(address) = nonblank(&options["address"]) {
                name_server = codec::js_string(&address);
                proxy["server"] = address;
            }
            if let Some(port) = nonblank(&options["port"]) {
                proxy["port"] = finite_int(port.as_str().unwrap())?;
                name_port = field_text(&proxy, "port");
            }
        }
    }
    if proxy["plugin"] == "v2ray-plugin"
        && let Some(mux) = proxy["plugin-opts"]["mux"].as_str()
    {
        let mux = codec::trim(mux);
        if !mux.is_empty() && mux.bytes().all(|byte| byte.is_ascii_digit()) {
            finite_int(mux)?;
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
    normalize_endpoint(&mut proxy)?;
    for field in ["tls", "sni", "client-fingerprint", "alpn"] {
        if proxy[field].is_null() {
            proxy.as_object_mut().unwrap().shift_remove(field);
        }
    }
    Ok(Some(proxy))
}

fn normalize(proxy: &mut Value) -> Result<(), Error> {
    let keys: Vec<_> = proxy.as_object().unwrap().keys().cloned().collect();
    for key in keys {
        let lower = key.to_lowercase();
        if !lower.ends_with("-opts") {
            continue;
        }
        if key != lower {
            let value = proxy.as_object_mut().unwrap().shift_remove(&key).unwrap();
            proxy
                .as_object_mut()
                .unwrap()
                .entry(lower.clone())
                .or_insert(value);
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
    if truthy(&proxy["reality-opts"]["public-key"])
        && proxy["reality-opts"]
            .get("support-x25519mlkem768")
            .is_none()
        && matches!(
            proxy["_loon_tls_profile"].as_str(),
            Some("chrome147" | "safari-ios-26")
        )
    {
        proxy["reality-opts"]["support-x25519mlkem768"] = json!(true);
    }
    proxy["udp"] = json!(match &proxy["udp"] {
        Value::Bool(false) => false,
        Value::Number(number) if number.as_f64() == Some(0.0) => false,
        Value::String(value) if ["0", "false", "off"].contains(&value.to_lowercase().as_str()) =>
            false,
        _ => true,
    });
    if !proxy["hop-interval"].is_null() {
        let raw = field_text(proxy, "hop-interval");
        let raw = codec::trim(&raw);
        let digits = |raw: &str| !raw.is_empty() && raw.bytes().all(|byte| byte.is_ascii_digit());
        let range = raw
            .split_once('-')
            .map(|(min, max)| (codec::trim(min), codec::trim(max)));
        let interval =
            if let Some((min, max)) = range.filter(|(min, max)| digits(min) && digits(max)) {
                let min = finite_int(min)?;
                let max = finite_int(max)?;
                if min.as_f64().unwrap() > 0.0 && min.as_f64().unwrap() <= max.as_f64().unwrap() {
                    Some((min, Some(max)))
                } else {
                    None
                }
            } else if digits(raw) {
                let value = finite_int(raw)?;
                if value.as_f64().unwrap() > 0.0 {
                    Some((value, None))
                } else {
                    None
                }
            } else {
                None
            };
        if let Some((min, max)) = interval {
            proxy["hop-interval"] = min;
            if let Some(max) = max {
                proxy["hop-interval-max"] = max;
            } else {
                proxy
                    .as_object_mut()
                    .unwrap()
                    .shift_remove("hop-interval-max");
            }
        } else {
            proxy.as_object_mut().unwrap().shift_remove("hop-interval");
            proxy
                .as_object_mut()
                .unwrap()
                .shift_remove("hop-interval-max");
        }
    }
    if truthy(&proxy["interface"]) {
        proxy["interface-name"] = proxy["interface"].clone();
        proxy.as_object_mut().unwrap().shift_remove("interface");
    }
    if let Some(port) = proxy["port"].as_str() {
        proxy["port"] = normalized_port(port);
    }
    if let Some(cipher) = proxy["cipher"].as_str() {
        proxy["cipher"] = json!(cipher.to_lowercase());
    }
    if proxy["password"].is_number() {
        proxy["password"] = json!(field_text(proxy, "password"));
    }
    if matches!(proxy["type"].as_str(), Some("trojan" | "vmess" | "vless"))
        && !truthy(&proxy["network"])
    {
        proxy["network"] = json!("tcp");
    }
    if proxy["type"] == "vmess" {
        if !truthy(&proxy["cipher"]) {
            proxy["cipher"] = json!("none");
        }
        if !truthy(&proxy["alterId"]) {
            proxy["alterId"] = json!(0);
        }
    }
    if matches!(proxy["type"].as_str(), Some("vmess" | "vless"))
        && proxy["packet-encoding"].is_null()
    {
        if truthy(&proxy["xudp"]) {
            proxy["packet-encoding"] = json!("xudp");
        } else if truthy(&proxy["packet-addr"]) {
            proxy["packet-encoding"] = json!("packetaddr");
        }
    }
    if matches!(
        proxy["type"].as_str(),
        Some("trojan" | "tuic" | "hysteria" | "hysteria2" | "anytls")
    ) {
        proxy["tls"] = json!(true);
    }
    if proxy["plugin"] == "shadow-tls" && !proxy["alpn"].is_null() {
        if proxy["plugin-opts"]["alpn"].is_null() {
            proxy["plugin-opts"]["alpn"] = proxy["alpn"].clone();
        }
        proxy.as_object_mut().unwrap().shift_remove("alpn");
    }
    if truthy(&proxy["network"]) {
        let network = field_text(proxy, "network");
        let key = format!("{network}-opts");
        let format_path = |value: &Value| {
            if let Some(path) = value.as_str() {
                let path = codec::trim(path);
                json!(if path.starts_with('/') {
                    path.to_owned()
                } else {
                    format!("/{path}")
                })
            } else {
                value.clone()
            }
        };
        if let Some(path) = proxy[&key].get("path") {
            proxy[&key]["path"] = if let Some(paths) = path.as_array() {
                json!(paths.iter().map(format_path).collect::<Vec<_>>())
            } else {
                format_path(path)
            };
        }
        if network != "h2"
            && truthy(&proxy[&key]["headers"]["host"])
            && !truthy(&proxy[&key]["headers"]["Host"])
        {
            proxy[&key]["headers"]["Host"] = proxy[&key]["headers"]["host"].clone();
            proxy[&key]["headers"]
                .as_object_mut()
                .unwrap()
                .shift_remove("host");
        }
        if network == "h2" {
            let host = crate::xhttp::transport_host("h2", &proxy[&key]).clone();
            if truthy(&host) {
                proxy[&key]["host"] = if host.is_array() { host } else { json!([host]) };
            }
            if let Some(headers) = proxy
                .get_mut(&key)
                .and_then(|options| options.get_mut("headers"))
                .and_then(Value::as_object_mut)
            {
                headers.shift_remove("host");
                headers.shift_remove("Host");
                if headers.is_empty() {
                    proxy[&key].as_object_mut().unwrap().shift_remove("headers");
                }
            }
            if let Some(paths) = proxy[&key]["path"].as_array() {
                let path = paths.first().cloned();
                if let Some(path) = path {
                    proxy[&key]["path"] = path;
                } else {
                    proxy[&key].as_object_mut().unwrap().shift_remove("path");
                }
            }
        }
        let http_array =
            matches!(proxy["type"].as_str(), Some("vmess" | "vless")) && network == "http";
        if !truthy(&proxy["tls"])
            && ["ws", "http"].contains(&network.as_str())
            && !truthy(&proxy[&key]["headers"]["Host"])
            && proxy["server"]
                .as_str()
                .is_some_and(|server| !is_ip(server))
        {
            proxy[&key]["headers"]["Host"] = if http_array {
                json!([proxy["server"]])
            } else {
                proxy["server"].clone()
            };
        }
        if http_array {
            let host = proxy[&key]["headers"]["Host"].clone();
            if truthy(&host) && !host.is_array() {
                proxy[&key]["headers"]["Host"] = json!([host]);
            }
            let path = proxy[&key]["path"].clone();
            if truthy(&path) && !path.is_array() {
                proxy[&key]["path"] = json!([path]);
            }
        }
        if truthy(&proxy["tls"]) && !truthy(&proxy["sni"]) && proxy["sni"] != "" {
            let host = if network == "h2" {
                &proxy[&key]["host"]
            } else {
                &proxy[&key]["headers"]["Host"]
            };
            let host = host
                .as_array()
                .and_then(|hosts| hosts.first())
                .unwrap_or(host);
            if truthy(host) {
                proxy["sni"] = host.clone();
            }
        }
        if ["ws", "h2"].contains(&network.as_str()) && !truthy(&proxy[&key]["path"]) {
            proxy[&key]["path"] = json!("/");
        }
        if network == "http"
            && proxy[&key]["path"]
                .as_array()
                .is_none_or(|paths| paths.iter().all(|path| !truthy(path)))
        {
            proxy[&key]["path"] = json!(["/"]);
        }
    }
    if truthy(&proxy["tls"])
        && !truthy(&proxy["sni"])
        && proxy["sni"] != ""
        && proxy["server"]
            .as_str()
            .is_some_and(|server| !is_ip(server))
    {
        proxy["sni"] = proxy["server"].clone();
    }
    if matches!(proxy["sni"].as_str(), Some("" | "off")) {
        proxy["disable-sni"] = json!(true);
    }
    if truthy(&proxy["ports"]) {
        proxy["ports"] = json!(field_text(proxy, "ports").replace('/', ","));
    } else {
        proxy.as_object_mut().unwrap().shift_remove("ports");
    }
    if proxy["type"] == "hysteria2" && truthy(&proxy["obfs"]) {
        if proxy["obfs"] != "salamander" && !truthy(&proxy["obfs-password"]) {
            proxy["obfs-password"] = proxy["obfs"].clone();
            proxy["obfs"] = json!("salamander");
        }
        if !truthy(&proxy["obfs-password"]) && truthy(&proxy["obfs_password"]) {
            proxy["obfs-password"] = proxy["obfs_password"].clone();
        }
    }
    if proxy["type"] == "tuic" {
        if !proxy["alpn"].is_array() {
            proxy["alpn"] = json!([if truthy(&proxy["alpn"]) {
                proxy["alpn"].clone()
            } else {
                json!("h3")
            }]);
        }
        for (key, value) in [
            ("congestion-controller", "cubic"),
            ("udp-relay-mode", "native"),
        ] {
            if !truthy(&proxy[key]) {
                proxy[key] = json!(value);
            }
        }
    }
    if proxy["type"] == "wireguard" {
        normalize_wireguard(proxy);
    }
    if proxy["type"] == "vless" {
        for key in ["reality-opts", "grpc-opts"] {
            if proxy[key].as_object().is_some_and(Map::is_empty) {
                proxy.as_object_mut().unwrap().shift_remove(key);
            }
        }
        if (!truthy(&proxy["reality-opts"]) && !truthy(&proxy["flow"]))
            || proxy["flow"].is_null()
            || proxy["flow"] == "null"
        {
            proxy.as_object_mut().unwrap().shift_remove("flow");
        }
    }
    if proxy["type"] == "anytls" && truthy(&proxy["disable-reuse"]) {
        proxy["reuse"] = json!(false);
    }
    if !proxy["name"].is_string() {
        let raw = field_text(proxy, "name");
        if !raw.is_empty() && raw.bytes().all(|byte| byte.is_ascii_digit()) {
            proxy["name"] = json!(raw);
        } else {
            let value = if truthy(&proxy["name"]["data"]) {
                &proxy["name"]["data"]
            } else {
                &proxy["name"]
            };
            let decoded = value
                .as_array()
                .map(|values| {
                    let bytes: Vec<_> = values
                        .iter()
                        .map(|value| {
                            let number = codec::to_number(value);
                            if number.is_finite() {
                                number.trunc().rem_euclid(256.0) as u8
                            } else {
                                0
                            }
                        })
                        .collect();
                    codec::encode(&String::from_utf8_lossy(&bytes))
                })
                .or_else(|| value.as_str().map(str::to_owned));
            proxy["name"] = json!(decoded.unwrap_or_else(|| format!(
                "{} {}:{}",
                field_text(proxy, "type"),
                field_text(proxy, "server"),
                field_text(proxy, "port")
            )));
        }
    }
    Ok(())
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
    let raw = String::from_utf16(&codec::units(raw)).map_err(|_| {
        Error::Pending("URI parsing of isolated UTF-16 surrogates is not implemented yet")
    })?;
    let raw = preprocess(&raw);
    let full_config = crate::surge::is_full_config(&raw);
    let mut proxies = Vec::new();
    let mut last_parser = None;
    for line in raw
        .split('\n')
        .map(codec::trim)
        .filter(|line| !line.is_empty())
    {
        let before = proxies.len();
        let mut parsed_classic = None;
        if line.starts_with("ss://") {
            if let Some(proxy) = ss(line)? {
                proxies.push(proxy);
            }
        } else if line.starts_with("vmess://") {
            if let Some(proxy) = vmess(line)? {
                proxies.push(proxy);
            }
        } else if line.starts_with("vless://") || line.starts_with("anytls://") {
            let parsed = if line.starts_with("anytls://") {
                anytls(line)?
            } else {
                vless(line, "vless")?
            };
            if let Some(mut proxy) = parsed {
                normalize_endpoint(&mut proxy)?;
                proxy
                    .as_object_mut()
                    .unwrap()
                    .retain(|_, value| !value.is_null());
                proxies.push(proxy);
            }
        } else if line.starts_with("ssr://") {
            if let Some(proxy) = ssr(line)? {
                proxies.push(proxy);
            }
        } else if line.starts_with("trojan://") {
            if let Some(proxy) = trojan(line)? {
                proxies.push(proxy);
            }
        } else if line.split_once("://").is_some_and(|(scheme, _)| {
            [
                "hy",
                "hysteria",
                "hy2",
                "hysteria2",
                "tuic",
                "wireguard",
                "wg",
            ]
            .contains(&scheme)
        }) {
            if let Some(proxy) = modern_uri(line)? {
                proxies.push(proxy);
            }
        } else if line.split_once("://").is_some_and(|(scheme, _)| {
            ["http", "https", "socks5", "socks5+tls", "socks"].contains(&scheme)
        }) {
            if let Some(proxy) = proxy_uri(line)? {
                proxies.push(proxy);
            }
        } else if let Some((mut proxy, parser)) = crate::classic::parse(line, last_parser)? {
            normalize_endpoint(&mut proxy)?;
            proxies.push(proxy);
            parsed_classic = Some(parser);
        } else if full_config && line.starts_with('[') && line.ends_with(']') {
            continue;
        } else if line.contains("proxies:") || line.starts_with('[') || line.starts_with('{') {
            return Err(Error::Pending(
                "This subscription format is not implemented yet",
            ));
        }
        if proxies.len() > before {
            last_parser = parsed_classic;
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

fn transport_path(proxy: &Value, embed_ws: bool) -> Option<String> {
    let network = proxy["network"].as_str()?;
    let options = &proxy[format!("{network}-opts")];
    let path_value = options["path"]
        .as_array()
        .and_then(|items| items.first())
        .unwrap_or(&options["path"]);
    let mut path = path_value.as_str().map(str::to_owned);
    if network != "ws" {
        return path;
    }
    let upgrade = truthy(&options["v2ray-http-upgrade"]);
    let early = if upgrade && truthy(&options["v2ray-http-upgrade-fast-open"]) {
        options
            .get("_v2ray-http-upgrade-ed")
            .map(codec::js_string)
            .filter(|raw| safe_integer(raw).is_some())
            .or_else(|| path.as_deref().and_then(|raw| path_ed(raw).1))
            .or_else(|| Some("2560".to_owned()))
    } else if !upgrade && embed_ws {
        let custom_header = options["early-data-header-name"]
            .as_str()
            .is_some_and(|raw| !raw.is_empty() && raw != "Sec-WebSocket-Protocol");
        if custom_header {
            if let Some(raw) = path.as_mut() {
                *raw = path_ed(raw).0;
            }
            None
        } else {
            options
                .get("max-early-data")
                .map(codec::js_string)
                .filter(|raw| safe_integer(raw).is_some())
        }
    } else {
        if !upgrade
            && options.get("max-early-data").is_some()
            && let Some(raw) = path.as_mut()
        {
            *raw = path_ed(raw).0;
        }
        None
    };
    if let Some(early) = early {
        let raw = path.as_deref().unwrap_or("/");
        let (raw, _) = path_ed(raw);
        let raw = if raw.is_empty() { "/".to_owned() } else { raw };
        let separator = if raw.ends_with(['?', '&']) {
            ""
        } else if raw.contains('?') {
            "&"
        } else {
            "?"
        };
        path = Some(format!("{raw}{separator}ed={early}"));
    }
    path
}

fn vless_query(proxy: &Value) -> Option<Vec<(String, String)>> {
    let mut query = Vec::new();
    let mut push = |key: &str, value: &Value| -> Option<()> {
        query.push((
            key.to_owned(),
            codec::uri_component(&codec::js_string(value)).ok()?,
        ));
        Some(())
    };
    let reality = truthy(&proxy["reality-opts"]);
    push(
        "security",
        &json!(if reality {
            "reality"
        } else if truthy(&proxy["tls"]) {
            "tls"
        } else {
            "none"
        }),
    )?;
    let network = proxy["network"].as_str().unwrap_or("tcp");
    let options = &proxy[format!("{network}-opts")];
    let upgrade = network == "ws" && truthy(&options["v2ray-http-upgrade"]);
    push(
        "type",
        &json!(match network {
            "ws" if upgrade => "httpupgrade",
            "http" => "tcp",
            "h2" => "http",
            other => other,
        }),
    )?;
    if network == "http" {
        push("headerType", &json!("http"))?;
    }
    if network == "grpc" {
        push(
            "mode",
            &if truthy(&options["_grpc-type"]) {
                options["_grpc-type"].clone()
            } else {
                json!("gun")
            },
        )?;
        if truthy(&options["_grpc-authority"]) {
            push("authority", &options["_grpc-authority"])?;
        }
    }
    if let Some(path) = transport_path(proxy, false).filter(|raw| !raw.is_empty()) {
        push("path", &json!(path))?;
    }
    let host = crate::xhttp::transport_host(network, options);
    let host = host
        .as_array()
        .and_then(|items| items.first())
        .unwrap_or(host);
    if truthy(host) {
        push("host", host)?;
    }
    let service = format!("{network}-service-name");
    if truthy(&options[&service]) {
        push("serviceName", &options[&service])?;
    }
    if network == "http" && truthy(&options["method"]) {
        push("method", &options["method"])?;
    }
    if network == "kcp" {
        for key in ["seed", "headerType"] {
            if truthy(&proxy[key]) {
                push(key, &proxy[key])?;
            }
        }
    }
    let early = proxy["ws-opts"]
        .get("max-early-data")
        .map(codec::js_string)
        .filter(|raw| safe_integer(raw).is_some());
    if network == "ws"
        && !upgrade
        && let Some(value) = early
    {
        push("ed", &json!(value))?;
    }
    let header = &proxy["ws-opts"]["early-data-header-name"];
    if truthy(header)
        && (upgrade
            || proxy["ws-opts"]["max-early-data"].is_null()
            || header != "Sec-WebSocket-Protocol")
    {
        push("eh", header)?;
    }
    let encoding = if !proxy["packet-encoding"].is_null() {
        Some(field_text(proxy, "packet-encoding").trim().to_lowercase())
    } else if truthy(&proxy["xudp"]) {
        Some("xudp".to_owned())
    } else if truthy(&proxy["packet-addr"]) {
        Some("packetaddr".to_owned())
    } else if proxy["udp"] == true {
        Some(String::new())
    } else {
        None
    };
    if let Some(value) = match encoding.as_deref() {
        Some("") => Some("none"),
        Some("packetaddr") => Some("packet"),
        Some("xudp") => Some("xudp"),
        _ => None,
    } {
        push("packetEncoding", &json!(value))?;
    }
    if truthy(&proxy["alpn"]) {
        if !proxy["alpn"].is_array() {
            return None;
        }
        push("alpn", &proxy["alpn"])?;
    }
    if truthy(&proxy["skip-cert-verify"]) {
        push("allowInsecure", &json!(1))?;
    }
    if truthy(&proxy["tls-fingerprint"]) {
        push("pcs", &proxy["tls-fingerprint"])?;
    }
    let cert_names = if proxy["_vcn"].is_array() {
        &proxy["_vcn"]
    } else {
        &proxy["name-cert-verify"]
    };
    if proxy["_vcn"].is_array() || truthy(cert_names) {
        push("vcn", cert_names)?;
    }
    let ech = crate::xhttp::ech_fields(&proxy["ech-opts"], &proxy["_echConfigList"]);
    if truthy(&ech["echConfigList"]) {
        push("ech", &ech["echConfigList"])?;
    }
    if truthy(&proxy["_h2"]) {
        push("h2", &json!(1))?;
    }
    for (field, param) in [
        ("sni", "sni"),
        ("client-fingerprint", "fp"),
        ("flow", "flow"),
    ] {
        if truthy(&proxy[field]) {
            push(param, &proxy[field])?;
        }
    }
    if reality {
        for (field, param) in [
            ("short-id", "sid"),
            ("_spider-x", "spx"),
            ("public-key", "pbk"),
        ] {
            if truthy(&proxy["reality-opts"][field]) {
                push(param, &proxy["reality-opts"][field])?;
                if field == "public-key" && truthy(&proxy["reality-opts"]["support-x25519mlkem768"])
                {
                    push(
                        "support-x25519mlkem768",
                        &proxy["reality-opts"]["support-x25519mlkem768"],
                    )?;
                }
            }
        }
    }
    if network == "xhttp" && truthy(&options["mode"]) {
        push("mode", &options["mode"])?;
    } else if truthy(&proxy["_mode"]) {
        push("mode", &proxy["_mode"])?;
    }
    if let Some(extra) = crate::xhttp::uri_extra(proxy).filter(|raw| !raw.is_empty()) {
        push("extra", &json!(extra))?;
    }
    if let Some(value) = proxy.get("_finalmask") {
        let raw = if let Some(raw) = value.as_str() {
            Some(raw.to_owned())
        } else {
            value.is_object().then(|| codec::stringify(value))
        };
        if let Some(raw) = raw.filter(|raw| !raw.is_empty()) {
            push("fm", &json!(raw))?;
        }
    }
    for (field, param) in [("_pqv", "pqv"), ("encryption", "encryption")] {
        if truthy(&proxy[field]) {
            push(param, &proxy[field])?;
        }
    }
    Some(query)
}

fn vless_output(proxy: &Value, server: &str, port: &str, anytls: bool) -> Option<String> {
    let mut query: Map<String, Value> = vless_query(proxy)?
        .into_iter()
        .map(|(key, value)| (key, json!(value)))
        .collect();
    if anytls {
        for (key, value) in proxy.as_object()? {
            if ["name", "type", "password", "server", "port", "tls"].contains(&key.as_str()) {
                continue;
            }
            let param = key.replace('-', "_");
            let encoded = match key.as_str() {
                "alpn" if truthy(value) => {
                    let value = value
                        .as_array()
                        .and_then(|items| items.first())
                        .unwrap_or(value);
                    Some((param, codec::uri_component(&codec::js_string(value)).ok()?))
                }
                "skip-cert-verify" if truthy(value) => {
                    Some(("insecure".to_owned(), "1".to_owned()))
                }
                "udp" if truthy(value) => Some(("udp".to_owned(), "1".to_owned())),
                _ if truthy(value)
                    && !key.starts_with('_')
                    && !key.to_lowercase().contains("client-fingerprint")
                    && (value.is_string() || value.is_number() || value.is_boolean()) =>
                {
                    Some((param, codec::uri_component(&codec::js_string(value)).ok()?))
                }
                _ => None,
            };
            if let Some((key, value)) = encoded {
                query.insert(key, json!(value));
            }
        }
    }
    let scheme = if anytls { "anytls" } else { "vless" };
    Some(format!(
        "{scheme}://{}@{server}:{port}?{}#{}",
        codec::text(&field_text(proxy, if anytls { "password" } else { "uuid" })),
        query
            .iter()
            .map(|(key, value)| format!("{key}={}", value.as_str().unwrap()))
            .collect::<Vec<_>>()
            .join("&"),
        codec::uri_component(&field_text(proxy, "name")).ok()?
    ))
}

fn vmess_output(proxy: &Value, server: &str, port: &str) -> Option<String> {
    if truthy(&proxy["reality-opts"]) || truthy(&proxy["_finalmask"]) {
        if proxy["aead"] == false
            || (proxy["aead"] != true
                && truthy(&proxy["alterId"])
                && codec::to_number(&proxy["alterId"]) != 0.0)
        {
            return None;
        }
        let mut query_proxy = proxy.clone();
        query_proxy["encryption"] = vmess_security(&proxy["cipher"]);
        query_proxy.as_object_mut().unwrap().shift_remove("flow");
        return vless_output(&query_proxy, server, port, false)
            .map(|raw| raw.replacen("vless://", "vmess://", 1));
    }
    let network = proxy["network"].as_str().unwrap_or("tcp");
    let options = &proxy[format!("{network}-opts")];
    let upgrade = network == "ws" && truthy(&options["v2ray-http-upgrade"]);
    let mut output = json!({"v":"2","ps":proxy["name"],"add":proxy["server"],"port":port});
    if let Some(value) = proxy.get("uuid") {
        output["id"] = value.clone();
    }
    output["aid"] = json!(if truthy(&proxy["alterId"]) {
        field_text(proxy, "alterId")
    } else {
        "0".to_owned()
    });
    output["scy"] = vmess_security(&proxy["cipher"]);
    output["net"] = json!(if network == "http" {
        "tcp"
    } else if upgrade {
        "httpupgrade"
    } else {
        network
    });
    output["type"] = json!(if network == "http" { "http" } else { "" });
    output["tls"] = json!(if truthy(&proxy["tls"]) { "tls" } else { "" });
    if let Some(value) = proxy.get("alpn") {
        output["alpn"] = if value.is_array() {
            json!(codec::js_string(value))
        } else {
            value.clone()
        };
    }
    if let Some(value) = proxy.get("client-fingerprint") {
        output["fp"] = value.clone();
    }
    if truthy(&proxy["tls"]) && truthy(&proxy["sni"]) {
        output["sni"] = proxy["sni"].clone();
    }
    if truthy(&proxy["network"]) {
        if network == "grpc" || ["kcp", "quic"].contains(&network) {
            let fields = if network == "grpc" {
                vec![
                    ("path".to_owned(), "grpc-service-name".to_owned()),
                    ("type".to_owned(), "_grpc-type".to_owned()),
                    ("host".to_owned(), "_grpc-authority".to_owned()),
                ]
            } else {
                vec![
                    ("type".to_owned(), format!("_{network}-type")),
                    ("host".to_owned(), format!("_{network}-host")),
                    ("path".to_owned(), format!("_{network}-path")),
                ]
            };
            for (key, field) in fields {
                if key == "type" {
                    output[&key] = if truthy(&options[&field]) {
                        options[&field].clone()
                    } else {
                        json!(if network == "grpc" { "gun" } else { "none" })
                    };
                } else if let Some(value) = options.get(&field) {
                    output[&key] = value.clone();
                }
            }
        } else {
            if let Some(path) = transport_path(proxy, true).filter(|raw| !raw.is_empty()) {
                output["path"] = json!(path);
            }
            let host = crate::xhttp::transport_host(network, options);
            let host = host
                .as_array()
                .and_then(|items| items.first())
                .unwrap_or(host);
            if truthy(host) {
                output["host"] = host.clone();
            }
        }
    }
    Some(format!(
        "vmess://{}",
        STANDARD.encode(codec::stringify(&output))
    ))
}

fn modern_output(proxy: &Value, server: &str, port: &str) -> Option<String> {
    let kind = proxy["type"].as_str()?;
    let encode = |value: &Value| codec::uri_component(&codec::js_string(value)).ok();
    let mut query = Vec::new();
    if kind == "hysteria2" {
        for (key, parameter) in [("hop-interval", "hop-interval"), ("keepalive", "keepalive")] {
            if truthy(&proxy[key]) {
                query.push(format!("{parameter}={}", field_text(proxy, key)));
            }
        }
        if truthy(&proxy["skip-cert-verify"]) {
            query.push("insecure=1".to_owned());
        }
        if truthy(&proxy["obfs"]) {
            query.push(format!("obfs={}", encode(&proxy["obfs"])?));
            if truthy(&proxy["obfs-password"]) {
                query.push(format!(
                    "obfs-password={}",
                    encode(&proxy["obfs-password"])?
                ));
            }
        }
        if truthy(&proxy["sni"]) {
            query.push(format!("sni={}", encode(&proxy["sni"])?));
        }
        if truthy(&proxy["ports"]) {
            query.push(format!("mport={}", field_text(proxy, "ports")));
        }
        if truthy(&proxy["tls-fingerprint"]) {
            query.push(format!("pinSHA256={}", encode(&proxy["tls-fingerprint"])?));
        }
        if truthy(&proxy["tfo"]) {
            query.push("fastopen=1".to_owned());
        }
        let ech = &proxy["ech-opts"];
        let enabled = ech["enable"] == true
            || ech["enable"]
                .as_f64()
                .is_some_and(|value| value != 0.0 && value.fract() == 0.0);
        if enabled {
            let name = nonblank(&ech["query-server-name"]).map(|value| codec::js_string(&value));
            let config = if let Some(config) = nonblank(&ech["config"]) {
                Some(codec::js_string(&config))
            } else if let Some(dns) = nonblank(&ech["_dns"]) {
                Some(name.map_or_else(
                    || codec::js_string(&dns),
                    |name| format!("{name}+{}", codec::js_string(&dns)),
                ))
            } else {
                name.map(|name| format!("{name}+https://dns.alidns.com/dns-query"))
            };
            if let Some(config) = config {
                query.push(format!("ech={}", codec::uri_component(&config).ok()?));
            }
        }
        return Some(format!(
            "hysteria2://{}@{server}:{port}?{}#{}",
            encode(&proxy["password"])?,
            query.join("&"),
            encode(&proxy["name"])?
        ));
    }
    if kind == "tuic"
        && truthy(&proxy["token"])
        && !proxy["token"].as_array().is_some_and(Vec::is_empty)
    {
        return Some(String::new());
    }
    for (key, value) in proxy.as_object()? {
        if [
            "name",
            "type",
            "server",
            "port",
            "id",
            "resolved",
            "no-resolve",
            "subName",
            "collectionName",
        ]
        .contains(&key.as_str())
            || value.is_null()
            || (kind != "wireguard" && key == "tls")
            || (kind == "tuic" && ["uuid", "password"].contains(&key.as_str()))
            || (kind == "wireguard"
                && ["ip", "ipv6", "ip-cidr", "ipv6-cidr", "private-key"].contains(&key.as_str()))
        {
            continue;
        }
        let parameter = if kind == "tuic" {
            key.replace('-', "_")
        } else if kind == "hysteria" {
            key.replacen('-', "_", 1)
        } else {
            key.clone()
        };
        match (kind, key.as_str()) {
            ("wireguard", "public-key") => query.push(format!("publickey={}", encode(value)?)),
            ("wireguard", "udp") => {
                if truthy(value) {
                    query.push("udp=1".to_owned());
                }
            }
            ("hysteria" | "tuic", "alpn") => {
                if truthy(value) {
                    let value = value.as_array().map_or(Some(value), |items| items.first());
                    let encoded = value.map_or_else(|| Some("undefined".to_owned()), encode)?;
                    query.push(format!("{parameter}={encoded}"));
                }
            }
            ("hysteria" | "tuic", "skip-cert-verify") => {
                if truthy(value) {
                    query.push(
                        if kind == "tuic" {
                            "allow_insecure=1"
                        } else {
                            "insecure=1"
                        }
                        .to_owned(),
                    );
                }
            }
            ("hysteria" | "tuic", "tfo" | "fast-open") => {
                let fast = if kind == "tuic" {
                    "fast_open=1"
                } else {
                    "fastopen=1"
                }
                .to_owned();
                if truthy(value) && !query.contains(&fast) {
                    query.push(fast);
                }
            }
            ("tuic", "disable-sni" | "reduce-rtt") => {
                if truthy(value) {
                    query.push(format!("{parameter}=1"));
                }
            }
            ("tuic", "congestion-controller") => {
                query.push(format!(
                    "congestion_control={}",
                    codec::text(&codec::js_string(value))
                ));
            }
            ("hysteria", "ports" | "auth-str" | "up" | "down" | "_obfs" | "obfs" | "sni") => {
                let parameter = match key.as_str() {
                    "ports" => "mport",
                    "auth-str" => "auth",
                    "up" => "upmbps",
                    "down" => "downmbps",
                    "_obfs" => "obfs",
                    "obfs" => "obfsParam",
                    _ => "peer",
                };
                query.push(format!(
                    "{parameter}={}",
                    codec::text(&codec::js_string(value))
                ));
            }
            _ if truthy(value) && !key.starts_with('_') => {
                query.push(format!("{parameter}={}", encode(value)?));
            }
            _ => {}
        }
    }
    let auth = if kind == "tuic" {
        format!(
            "{}:{}@",
            encode(&proxy["uuid"])?,
            encode(&proxy["password"])?
        )
    } else if kind == "wireguard" {
        let mut addresses = Vec::new();
        for (key, cidr_key, v6) in [("ip", "ip-cidr", false), ("ipv6", "ipv6-cidr", true)] {
            if let Some((host, parsed_cidr, family)) =
                proxy[key].as_str().and_then(interface_address)
                && family == v6
            {
                let max = if v6 { 128 } else { 32 };
                let cidr = safe_integer(codec::trim(&field_text(proxy, cidr_key)))
                    .filter(|cidr| *cidr <= max)
                    .map(|cidr| cidr as u32)
                    .or(parsed_cidr)
                    .unwrap_or(max as u32);
                addresses.push(format!("{host}/{cidr}"));
            }
        }
        if !addresses.is_empty() {
            query.push(format!(
                "address={}",
                codec::uri_component(&addresses.join(",")).ok()?
            ));
        }
        format!("{}@", encode(&proxy["private-key"])?)
    } else {
        String::new()
    };
    Some(format!(
        "{kind}://{auth}{server}:{port}{}?{}#{}",
        if kind == "wireguard" { "/" } else { "" },
        query.join("&"),
        encode(&proxy["name"])?
    ))
}

fn uri(proxy: &Value) -> Option<String> {
    let server = codec::text(&field_text(proxy, "server"));
    let server = if ipv6(&server) {
        format!("[{server}]")
    } else {
        server
    };
    let port = field_text(proxy, "port");
    match proxy["type"].as_str()? {
        "vmess" => return vmess_output(proxy, &server, &port),
        "vless" => return vless_output(proxy, &server, &port, false),
        "anytls" => return vless_output(proxy, &server, &port, true),
        "socks5" => {
            let credentials = |field: &str| {
                proxy
                    .get(field)
                    .filter(|value| !value.is_null())
                    .map(codec::js_string)
                    .unwrap_or_default()
            };
            let auth = STANDARD.encode(codec::text(&format!(
                "{}:{}",
                credentials("username"),
                credentials("password")
            )));
            return Some(format!(
                "socks://{}@{server}:{port}#{}",
                codec::uri_component(&auth).ok()?,
                codec::text(&field_text(proxy, "name"))
            ));
        }
        "ssr" => {
            let text = |field| codec::text(&field_text(proxy, field));
            let mut output = format!(
                "{server}:{port}:{}:{}:{}:{}/?remarks={}",
                text("protocol"),
                text("cipher"),
                text("obfs"),
                STANDARD.encode(text("password")),
                STANDARD.encode(text("name"))
            );
            for (field, param) in [
                ("obfs-param", "obfsparam"),
                ("protocol-param", "protoparam"),
            ] {
                if truthy(&proxy[field]) {
                    output.push_str(&format!("&{param}={}", STANDARD.encode(text(field))));
                }
            }
            return Some(format!("ssr://{}", STANDARD.encode(output)));
        }
        "hysteria" | "hysteria2" | "tuic" | "wireguard" => {
            return modern_output(proxy, &server, &port);
        }
        "ss" | "trojan" => {}
        _ => return Some(String::new()),
    }
    let trojan = proxy["type"] == "trojan";
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
    let mut output = if trojan {
        format!(
            "trojan://{}@{server}:{port}",
            codec::text(&field_text(proxy, "password"))
        )
    } else {
        format!("ss://{auth}@{server}:{port}")
    };
    let mut query = Vec::new();
    if !trojan && truthy(&proxy["plugin"]) {
        let options = &proxy["plugin-opts"];
        let plugin = match proxy["plugin"].as_str()? {
            "obfs" => format!(
                "simple-obfs;obfs={}{}",
                field_text(options, "mode"),
                if truthy(&options["host"]) {
                    format!(";obfs-host={}", field_text(options, "host"))
                } else {
                    String::new()
                }
            ),
            "v2ray-plugin" => {
                let mut plugin = format!(
                    "v2ray-plugin;obfs={};mode={}",
                    field_text(options, "mode"),
                    field_text(options, "mode")
                );
                if truthy(&options["host"]) {
                    plugin.push_str(&format!(
                        ";obfs-host={};host={}",
                        field_text(options, "host"),
                        field_text(options, "host")
                    ));
                }
                if truthy(&options["path"]) {
                    plugin.push_str(&format!(";path={}", field_text(options, "path")));
                }
                if truthy(&options["tls"]) {
                    plugin.push_str(";tls");
                }
                if truthy(&options["sni"]) {
                    plugin.push_str(&format!(";sni={}", field_text(options, "sni")));
                }
                if truthy(&options["skip-cert-verify"]) {
                    plugin.push_str(&format!(
                        ";skip-cert-verify={}",
                        field_text(options, "skip-cert-verify")
                    ));
                }
                if !options["mux"].is_null() {
                    let mux = match &options["mux"] {
                        Value::Bool(value) => json!(u8::from(*value)),
                        Value::String(value) if codec::trim(value).eq_ignore_ascii_case("true") => {
                            json!(1)
                        }
                        Value::String(value)
                            if codec::trim(value).eq_ignore_ascii_case("false") =>
                        {
                            json!(0)
                        }
                        Value::String(value)
                            if !codec::trim(value).is_empty()
                                && codec::trim(value).bytes().all(|byte| byte.is_ascii_digit()) =>
                        {
                            finite_int(codec::trim(value)).ok()?
                        }
                        value => value.clone(),
                    };
                    plugin.push_str(&format!(";mux={}", codec::js_string(&mux)));
                }
                plugin
            }
            "shadow-tls" => format!(
                "shadow-tls;host={};password={};version={}",
                field_text(options, "host"),
                field_text(options, "password"),
                field_text(options, "version")
            ),
            _ => return None,
        };
        output.push('/');
        query.push(format!("plugin={}", codec::uri_component(&plugin).ok()?));
    }
    if !trojan {
        for (field, parameter) in [("udp-over-tcp", "uot"), ("tfo", "tfo")] {
            if truthy(&proxy[field]) {
                query.push(format!("{parameter}=1"));
            }
        }
        query.push(format!("udp={}", u8::from(truthy(&proxy["udp"]))));
    }
    if truthy(&proxy["tls"]) {
        let sni = if truthy(&proxy["sni"]) {
            field_text(proxy, "sni")
        } else {
            server.clone()
        };
        query.push(format!("sni={}", codec::uri_component(&sni).ok()?));
        if truthy(&proxy["skip-cert-verify"]) {
            query.push("allowInsecure=1".to_owned());
        }
    }
    if truthy(&proxy["network"]) {
        let network = field_text(proxy, "network");
        let options = &proxy[format!("{network}-opts")];
        let upgrade = network == "ws" && truthy(&options["v2ray-http-upgrade"]);
        query.push(format!(
            "type={}",
            codec::uri_component(if upgrade { "httpupgrade" } else { &network }).ok()?
        ));
        if network == "grpc" {
            for (field, param) in [
                ("grpc-service-name", "serviceName"),
                ("_grpc-authority", "authority"),
            ] {
                if truthy(&options[field]) {
                    query.push(format!(
                        "{param}={}",
                        codec::uri_component(&field_text(options, field)).ok()?
                    ));
                }
            }
            query.push(format!(
                "mode={}",
                codec::uri_component(&if truthy(&options["_grpc-type"]) {
                    field_text(options, "_grpc-type")
                } else {
                    "gun".to_owned()
                })
                .ok()?
            ));
        }
        let mut path = options["path"]
            .as_array()
            .and_then(|items| items.first())
            .unwrap_or(&options["path"])
            .as_str()
            .unwrap_or("")
            .to_owned();
        let early = if upgrade && truthy(&options["v2ray-http-upgrade-fast-open"]) {
            options["_v2ray-http-upgrade-ed"]
                .as_str()
                .filter(|ed| safe_integer(ed).is_some())
                .map(str::to_owned)
                .or_else(|| Some("2560".to_owned()))
        } else if network == "ws"
            && matches!(
                options["early-data-header-name"].as_str(),
                None | Some("" | "Sec-WebSocket-Protocol")
            )
        {
            options
                .get("max-early-data")
                .map(codec::js_string)
                .filter(|ed| safe_integer(ed).is_some())
        } else {
            None
        };
        if let Some(ed) = early {
            let (base, _) = path_ed(&path);
            path = if base.is_empty() {
                "/".to_owned()
            } else {
                base
            };
            let separator = if path.ends_with(['?', '&']) {
                ""
            } else if path.contains('?') {
                "&"
            } else {
                "?"
            };
            path = format!("{path}{separator}ed={ed}");
        }
        if !path.is_empty() {
            query.push(format!("path={}", codec::uri_component(&path).ok()?));
        }
        let host = options["headers"]["Host"]
            .as_array()
            .and_then(|items| items.first())
            .unwrap_or(&options["headers"]["Host"]);
        if truthy(host) {
            query.push(format!(
                "host={}",
                codec::uri_component(&codec::js_string(host)).ok()?
            ));
        }
    }
    if truthy(&proxy["alpn"]) {
        query.push(format!(
            "alpn={}",
            codec::uri_component(&codec::js_string(&proxy["alpn"])).ok()?
        ));
    }
    if truthy(&proxy["client-fingerprint"]) {
        query.push(format!(
            "fp={}",
            codec::uri_component(&field_text(proxy, "client-fingerprint")).ok()?
        ));
    }
    if trojan {
        if truthy(&proxy["tls-fingerprint"]) {
            query.push(format!(
                "pcs={}",
                codec::uri_component(&field_text(proxy, "tls-fingerprint")).ok()?
            ));
        }
        let names = if proxy["_vcn"].is_array() {
            &proxy["_vcn"]
        } else {
            &proxy["name-cert-verify"]
        };
        if proxy["_vcn"].is_array() || truthy(names) {
            query.push(format!(
                "vcn={}",
                codec::uri_component(&codec::js_string(names)).ok()?
            ));
        }
    }
    if truthy(&proxy["reality-opts"]) {
        query.push("security=reality".to_owned());
        for (field, param) in [
            ("short-id", "sid"),
            ("public-key", "pbk"),
            ("_spider-x", "spx"),
        ] {
            if truthy(&proxy["reality-opts"][field]) {
                query.push(format!(
                    "{param}={}",
                    codec::uri_component(&field_text(&proxy["reality-opts"], field)).ok()?
                ));
            }
        }
        for param in ["mode", "extra"] {
            let field = format!("_{param}");
            if truthy(&proxy[&field]) {
                query.push(format!(
                    "{param}={}",
                    codec::uri_component(&field_text(proxy, &field)).ok()?
                ));
            }
        }
    } else if !trojan && truthy(&proxy["tls"]) {
        query.push("security=tls".to_owned());
    }
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
    proxies.retain(|proxy| {
        if proxy["type"] == "ss"
            && proxy["tls"] == true
            && proxy.get("plugin").is_none_or(Value::is_null)
            && proxy["network"]
                .as_str()
                .is_none_or(|network| codec::trim(network).eq_ignore_ascii_case("tcp"))
        {
            return false;
        }
        if proxy["type"] == "vless" {
            let options = &proxy["xhttp-opts"];
            for (download, reality) in [
                (false, &proxy["reality-opts"]),
                (true, &options["download-settings"]["reality-opts"]),
            ] {
                if truthy(reality)
                    && nonblank(&reality["public-key"]).is_none()
                    && !(download
                        && nonblank(&proxy["reality-opts"]["public-key"]).is_some()
                        && reality["public-key"] == "")
                {
                    return false;
                }
            }
            if proxy["network"] == "xhttp"
                && options["mode"] == "stream-one"
                && truthy(&options["download-settings"])
            {
                return false;
            }
        }
        true
    });
    for proxy in &mut proxies {
        if proxy["name"]
            .as_str()
            .is_none_or(|name| codec::trim(name).is_empty())
        {
            proxy["name"] = json!(format!(
                "{} {}:{}",
                field_text(proxy, "type"),
                proxy["server"].as_str().unwrap_or("undefined"),
                field_text(proxy, "port")
            ));
        }
        if truthy(&proxy["disable-sni"]) && proxy["type"] != "tuic" {
            proxy["sni"] = json!(if proxy["server"].as_str().is_some_and(is_ip) {
                field_text(proxy, "server")
            } else {
                "127.0.0.1".to_owned()
            });
        }
        if truthy(&proxy["ports"]) && !truthy(&proxy["port"]) {
            proxy["port"] = random_port(&field_text(proxy, "ports"))?;
        }
        if proxy["type"] == "wireguard" {
            normalize_wireguard(proxy);
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
    let mut lines: Vec<_> = proxies.iter().filter_map(uri).collect();
    if matches!(target, "v2" | "v2ray" | "V2Ray") {
        return Ok(json!(STANDARD.encode(lines.join("\n"))));
    }
    lines.retain(|line| !line.is_empty());
    Ok(if kind == "internal" {
        json!(lines)
    } else {
        json!(lines.join("\n"))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn port_hopping_selects_only_members_of_the_source_ranges() {
        for _ in 0..128 {
            let port = random_port("443/8000-8002")
                .ok()
                .and_then(|value| value.as_f64())
                .expect("native entropy and finite port");
            assert!(port == 443.0 || ((8000.0..=8002.0).contains(&port) && port.fract() == 0.0));
        }
        assert_eq!(
            random_port("0x1bb").ok().and_then(|value| value.as_f64()),
            Some(443.0)
        );
        assert!(matches!(random_port("443;443"), Err(Error::Pending(_))));
    }

    #[test]
    fn ipv6_validation_preserves_source_specific_address_forms() {
        for address in ["2001:db8::1", "fe80::1%eth0", "::ffff:192.168.00.1"] {
            assert!(ipv6(address), "{address}");
        }
        for address in ["host:123", "fe80::1%eth_0", "1:2:3:4:5:6:1.2.3.4"] {
            assert!(!ipv6(address), "{address}");
        }
    }
}
