use crate::conversion::Error;
use crate::json as codec;
use crate::state::truthy;
use serde_json::{Value, json};

#[derive(Clone, Copy, PartialEq)]
pub enum Parser {
    Surge,
    Loon(&'static str),
    Qx,
}

pub(crate) fn space(raw: &str) -> &str {
    raw.trim_start_matches([' ', '\r', '\t'])
}

pub(crate) fn strip_quotes(raw: &str, single: bool, trim: bool) -> &str {
    let raw = if trim { codec::trim(raw) } else { raw };
    let quoted = raw.as_bytes().first().is_some_and(|byte| {
        (*byte == b'"' || (single && *byte == b'\'')) && raw.as_bytes().last() == Some(byte)
    });
    if quoted && raw.len() >= 2 {
        &raw[1..raw.len() - 1]
    } else {
        raw
    }
}

pub(crate) fn digits(raw: &str) -> Option<Option<Value>> {
    let end = raw.bytes().take_while(u8::is_ascii_digit).count();
    if end == 0 {
        return Some(None);
    }
    if !space(&raw[end..]).is_empty() {
        return None;
    }
    let number = raw[..end].parse::<f64>().ok()?;
    Some(Some(codec::numeric(number)))
}

pub(crate) fn boolean(raw: &str) -> Option<Option<Value>> {
    for (text, value) in [("true", true), ("false", false)] {
        if let Some(rest) = raw.strip_prefix(text) {
            return space(rest).is_empty().then_some(Some(json!(value)));
        }
    }
    Some(None)
}

fn comma(raw: &str) -> Option<&str> {
    space(raw).strip_prefix(',').map(space)
}

fn parameter_start(raw: &str, loon: bool) -> bool {
    let raw = space(raw);
    let end = if loon {
        raw.bytes()
            .take_while(|byte| byte.is_ascii_alphanumeric() || b"_-".contains(byte))
            .count()
    } else {
        raw.find(['=', ',']).unwrap_or(raw.len())
    };
    end > 0 && space(&raw[end..]).starts_with('=')
}

fn field<'a>(rest: &mut &'a str, kind: &str, loon: bool) -> Option<(&'a str, &'a str)> {
    let raw = comma(rest)?;
    let equals = raw.find('=')?;
    if raw[..equals].contains(',') || raw[..equals].is_empty() {
        return None;
    }
    let key = raw[..equals].trim_end_matches([' ', '\r', '\t']);
    let value = space(&raw[equals + 1..]);
    let list = (loon && key == "server-dns")
        || (!loon && key == "password" && !["vmess", "vless"].contains(&kind));
    let quoted = loon
        && (key == "server-dns"
            || (loon_allowed(kind, key) && ["alpn", "server-ports"].contains(&key)));
    let quoted_end = if quoted && value.starts_with('"') {
        value[1..]
            .find('"')
            .filter(|end| key != "server-ports" || *end > 0)
            .map(|end| end + 2)
    } else {
        None
    };
    let mut end = quoted_end.unwrap_or_else(|| {
        value
            .bytes()
            .enumerate()
            .find_map(|(index, byte)| {
                (byte == b',' && (!list || parameter_start(&value[index + 1..], loon)))
                    .then_some(index)
            })
            .unwrap_or(value.len())
    });
    if loon && key == "server-dns" && quoted_end.is_none() {
        end = value.find('"').map_or(end, |quote| end.min(quote));
    }
    if end == 0 && key != "server-dns" {
        return None;
    }
    *rest = &value[end..];
    Some((key, &value[..end]))
}

fn quoted<'a>(rest: &mut &'a str, nonempty: bool) -> Option<&'a str> {
    let value = comma(rest)?.strip_prefix('"')?;
    let end = value.find('"')?;
    if nonempty && end == 0 {
        return None;
    }
    *rest = &value[end + 1..];
    Some(&value[..end])
}

fn loon_kind(line: &str) -> Option<&'static str> {
    let first = line.split(',').next()?;
    let (_, kind) = first.split_once('=')?;
    match codec::trim(kind).to_ascii_lowercase().as_str() {
        "shadowsocks" => Some("shadowsocks"),
        "shadowsocksr" => Some("shadowsocksr"),
        "vmess" if !line.contains("username") => Some("vmess"),
        "vless" => Some("vless"),
        "trojan" => Some("trojan"),
        "hysteria2" => Some("hysteria2"),
        "anytls" => Some("anytls"),
        "http" | "https" => Some("http"),
        "socks5" => Some("socks5"),
        "wireguard" => Some("wireguard"),
        _ => None,
    }
}

fn loon_allowed(kind: &str, key: &str) -> bool {
    if ["fast-open", "udp", "ip-mode", "block-quic"].contains(&key) {
        return true;
    }
    if ["tls-profile", "alpn"].contains(&key) {
        return kind != "http";
    }
    if [
        "tls-name",
        "sni",
        "skip-cert-verify",
        "tls-cert-sha256",
        "tls-pubkey-sha256",
    ]
    .contains(&key)
    {
        return [
            "vmess",
            "vless",
            "trojan",
            "anytls",
            "hysteria2",
            "https",
            "socks5",
        ]
        .contains(&kind);
    }
    if [
        "shadow-tls-version",
        "shadow-tls-sni",
        "shadow-tls-password",
        "obfs-host",
        "obfs-uri",
        "udp-port",
    ]
    .contains(&key)
    {
        return ["shadowsocks", "shadowsocksr"].contains(&kind);
    }
    if ["transport", "host", "path", "public-key", "short-id"].contains(&key) {
        return ["vmess", "vless", "trojan", "anytls"].contains(&kind);
    }
    if key == "over-tls" {
        return ["vmess", "vless", "trojan", "anytls", "socks5"].contains(&kind);
    }
    match kind {
        "shadowsocks" => ["obfs-name", "udp-over-tcp"].contains(&key),
        "shadowsocksr" => ["protocol", "protocol-param", "obfs", "obfs-param"].contains(&key),
        "vmess" => key == "alterId",
        "vless" => key == "flow",
        "anytls" => [
            "idle-session-check-interval",
            "idle-session-timeout",
            "min-idle-session",
            "max-stream-count",
        ]
        .contains(&key),
        "hysteria2" => [
            "download-bandwidth",
            "server-ports",
            "hop-interval",
            "salamander-password",
            "ecn",
        ]
        .contains(&key),
        _ => false,
    }
}

fn fixed_token<'a>(raw: &'a str, values: &[&str]) -> Option<Option<&'a str>> {
    let value = raw.trim_end_matches([' ', '\r', '\t']);
    if values.contains(&value) {
        Some(Some(value))
    } else if values.iter().any(|value| raw.starts_with(value)) {
        None
    } else {
        Some(None)
    }
}

fn loon_line(line: &str) -> Option<Value> {
    let (name, raw) = line.split_once('=')?;
    if name.contains(',') {
        return None;
    }
    let (kind, mut rest) = space(raw).split_once(',')?;
    let kind = kind
        .trim_end_matches([' ', '\r', '\t'])
        .to_ascii_lowercase();
    let kind = kind.as_str();
    if kind == "wireguard" {
        return loon_wireguard(line);
    }
    if ![
        "shadowsocks",
        "shadowsocksr",
        "vmess",
        "vless",
        "trojan",
        "hysteria2",
        "anytls",
        "http",
        "https",
        "socks5",
    ]
    .contains(&kind)
    {
        return None;
    }
    let (server, tail) = space(rest).split_once(',')?;
    let tail = space(tail);
    let end = tail.bytes().take_while(u8::is_ascii_digit).count();
    if end == 0 {
        return None;
    }
    let port = tail[..end].parse::<u16>().ok()?;
    rest = &tail[end..];
    let mut proxy = json!({"name":codec::trim(name),"server":codec::trim(server),"port":port});
    if ["shadowsocks", "shadowsocksr", "vmess"].contains(&kind) {
        let tail = comma(rest)?;
        let (cipher, _) = tail.split_once(',')?;
        rest = &tail[cipher.len()..];
        if kind == "vmess" {
            proxy["cipher"] = json!(match codec::trim(cipher).to_ascii_lowercase().as_str() {
                "none" => "none",
                "auto" => "auto",
                "aes-128-gcm" => "aes-128-gcm",
                "chacha20-ietf-poly1305" => "chacha20-poly1305",
                _ => "auto",
            });
            proxy["uuid"] = json!(quoted(&mut rest, true)?);
        } else {
            let ciphers = [
                "aes-128-cfb",
                "aes-128-ctr",
                "aes-128-gcm",
                "aes-192-cfb",
                "aes-192-ctr",
                "aes-192-gcm",
                "aes-256-cfb",
                "aes-256-ctr",
                "aes-256-gcm",
                "auto",
                "bf-cfb",
                "camellia-128-cfb",
                "camellia-192-cfb",
                "camellia-256-cfb",
                "chacha20-ietf-poly1305",
                "chacha20-ietf",
                "chacha20-poly1305",
                "chacha20",
                "none",
                "rc4-md5",
                "rc4",
                "salsa20",
                "xchacha20-ietf-poly1305",
                "2022-blake3-aes-128-gcm",
                "2022-blake3-aes-256-gcm",
            ];
            proxy["cipher"] = json!(fixed_token(cipher, &ciphers)??);
            proxy["password"] = json!(quoted(&mut rest, false)?);
        }
    } else if kind == "vless" {
        proxy["uuid"] = json!(quoted(&mut rest, true)?);
    } else if ["trojan", "hysteria2", "anytls"].contains(&kind) {
        proxy["password"] = json!(quoted(&mut rest, false)?);
    } else if let Some(tail) = space(rest).strip_prefix(',')
        && let Some((user, tail)) = tail.split_once(',').filter(|(user, _)| !user.contains('='))
    {
        proxy["username"] = json!(user);
        let password_start = line.len() - tail.len() - 1;
        rest = &line[password_start..];
        proxy["password"] = json!(quoted(&mut rest, false)?);
    }
    let mut obfs = json!({});
    let mut transport = json!({});
    let mut shadow = json!({});
    if kind == "shadowsocks"
        && let Some(tail) = comma(rest)
        && let Some((mode, after)) = tail.split_once(',')
        && let Some(mode) = fixed_token(mode, &["http", "tls"]).flatten()
    {
        obfs["type"] = json!(mode);
        let after = space(after);
        let end = after.find(',').unwrap_or(after.len());
        if end == 0 {
            return None;
        }
        obfs["host"] = json!(&after[..end]);
        rest = &after[end..];
    }
    while !rest.is_empty() {
        let (key, raw) = field(&mut rest, kind, true)?;
        if key == "server-dns" {
            let raw = strip_quotes(raw, false, false);
            proxy[key] = json!(
                raw.split(',')
                    .map(codec::trim)
                    .filter(|value| !value.is_empty())
                    .collect::<Vec<_>>()
            );
            continue;
        }
        if !loon_allowed(kind, key) {
            if raw.contains('=') {
                return None;
            }
            continue;
        }
        let output_key = match key {
            "fast-open" => "tfo",
            "ip-mode" => "ip-version",
            "over-tls" => "tls",
            "tls-name" => "sni",
            "tls-cert-sha256" => "tls-fingerprint",
            "server-ports" => "ports",
            "download-bandwidth" => "down",
            _ => key,
        };
        match key {
            "fast-open" | "udp" | "over-tls" | "skip-cert-verify" | "ecn" | "block-quic"
            | "udp-over-tcp" => {
                if let Some(value) = boolean(raw)? {
                    if key == "block-quic" {
                        proxy[key] = json!(if value == true { "on" } else { "off" });
                    } else if key == "udp-over-tcp" {
                        proxy[key] = json!(true);
                        proxy["udp-over-tcp-version"] = json!(2);
                    } else {
                        proxy[output_key] = value;
                    }
                } else if raw.contains('=') {
                    return None;
                }
            }
            "alterId"
            | "udp-port"
            | "hop-interval"
            | "shadow-tls-version"
            | "idle-session-check-interval"
            | "idle-session-timeout"
            | "min-idle-session"
            | "max-stream-count" => {
                if let Some(value) = digits(raw)? {
                    if key == "shadow-tls-version" {
                        shadow["version"] = value;
                    } else {
                        proxy[output_key] = value;
                    }
                } else if raw.contains('=') {
                    return None;
                }
            }
            "obfs-name" | "obfs" | "protocol" | "transport" => {
                let values: &[&str] = match key {
                    "obfs-name" => &["http", "tls"],
                    "obfs" => &[
                        "plain",
                        "http_simple",
                        "http_post",
                        "random_head",
                        "tls1.2_ticket_auth",
                        "tls1.2_ticket_fastauth",
                    ],
                    "protocol" => &[
                        "origin",
                        "auth_sha1_v4",
                        "auth_aes128_md5",
                        "auth_aes128_sha1",
                        "auth_chain_a",
                        "auth_chain_b",
                    ],
                    _ => &["tcp", "ws", "http"],
                };
                if let Some(value) = fixed_token(raw, values)? {
                    match key {
                        "protocol" => proxy[key] = json!(value),
                        "transport" => transport["type"] = json!(value),
                        _ => obfs["type"] = json!(value),
                    }
                } else if raw.contains('=') {
                    return None;
                }
            }
            "obfs-host" => obfs["host"] = json!(strip_quotes(raw, false, false)),
            "obfs-uri" => obfs["path"] = json!(raw),
            "host" => transport["host"] = json!(strip_quotes(raw, false, false)),
            "path" => transport["path"] = json!(raw),
            "public-key" | "short-id" => {
                proxy["reality-opts"][key] = json!(strip_quotes(raw, false, false))
            }
            "shadow-tls-sni" => shadow["host"] = json!(raw),
            "shadow-tls-password" => shadow["password"] = json!(strip_quotes(raw, true, false)),
            "tls-profile" => {
                let profile = codec::trim(strip_quotes(raw, false, false));
                proxy["_loon_tls_profile"] = json!(profile);
                let fingerprint = match profile {
                    "chrome" | "chrome147" => Some("chrome"),
                    "safari-ios18" | "safari-ios-26" => Some("ios"),
                    _ => None,
                };
                if let Some(fingerprint) = fingerprint {
                    proxy["client-fingerprint"] = json!(fingerprint);
                }
            }
            "alpn" | "server-ports" => {
                if raw.starts_with('"') && raw.ends_with('"') && raw.len() >= 2 {
                    let value = &raw[1..raw.len() - 1];
                    if key == "alpn" {
                        let values: Vec<_> = value
                            .split(',')
                            .map(codec::trim)
                            .filter(|value| !value.is_empty())
                            .collect();
                        if !values.is_empty() {
                            proxy[key] = json!(values);
                        }
                    } else if !value.is_empty() {
                        let parts: Vec<_> = codec::trim(value)
                            .split(',')
                            .map(|part| {
                                part.split('-')
                                    .map(codec::trim)
                                    .collect::<Vec<_>>()
                                    .join("-")
                            })
                            .collect();
                        proxy["ports"] = json!(parts.join(","));
                    }
                } else if raw.contains('=') {
                    return None;
                }
            }
            "salamander-password" => {
                proxy["obfs-password"] = json!(raw);
                proxy["obfs"] = json!("salamander");
            }
            "sni" | "tls-name" | "tls-cert-sha256" | "tls-pubkey-sha256" | "flow" => {
                proxy[output_key] = json!(strip_quotes(raw, false, false))
            }
            _ => {
                if key == "protocol-param" && raw.contains('=') {
                    return None;
                }
                proxy[output_key] = json!(raw);
            }
        }
    }
    proxy["type"] = json!(match kind {
        "shadowsocks" => "ss",
        "shadowsocksr" => "ssr",
        "https" => "http",
        kind => kind,
    });
    if kind == "https" {
        proxy["tls"] = json!(true);
    }
    if kind == "vmess" {
        if !truthy(&proxy["cipher"]) {
            proxy["cipher"] = json!("auto");
        }
        if !truthy(&proxy["alterId"]) {
            proxy["alterId"] = json!(0);
        }
    }
    if kind == "shadowsocksr" {
        if let Some(value) = obfs.get("type") {
            proxy["obfs"] = value.clone();
        }
    } else if kind == "shadowsocks" && matches!(obfs["type"].as_str(), Some("http" | "tls")) {
        proxy["plugin"] = json!("obfs");
        let mut options = json!({"mode":obfs["type"]});
        for key in ["host", "path"] {
            if let Some(value) = obfs.get(key) {
                options[key] = value.clone();
            }
        }
        proxy["plugin-opts"] = options;
    }
    if ["vmess", "vless", "trojan", "anytls"].contains(&kind)
        && matches!(transport["type"].as_str(), Some("ws" | "http"))
    {
        let network = transport["type"].as_str()?;
        proxy["network"] = json!(network);
        let mut options = json!({});
        if let Some(value) = transport.get("path") {
            options["path"] = value.clone();
        }
        let mut headers = json!({});
        if let Some(value) = transport.get("host") {
            headers["Host"] = value.clone();
        }
        options["headers"] = headers;
        proxy[format!("{network}-opts")] = options;
    }
    if truthy(&shadow["password"]) {
        if shadow["version"]
            .as_f64()
            .is_some_and(|number| number < 2.0)
        {
            return None;
        }
        proxy["plugin"] = json!("shadow-tls");
        let mut options = json!({});
        for key in ["host", "password", "version"] {
            if let Some(value) = shadow.get(key) {
                options[key] = value.clone();
            }
        }
        if let Some(value) = proxy.as_object_mut()?.shift_remove("alpn") {
            options["alpn"] = value;
        }
        proxy["plugin-opts"] = options;
    }
    Some(proxy)
}

fn qx_allowed(kind: &str, key: &str) -> bool {
    if [
        "password",
        "tls-pubkey-sha256",
        "tls-alpn",
        "tls-no-session-ticket",
        "tls-no-session-reuse",
        "tls-cert-sha256",
        "tls-verification",
        "udp-relay",
        "fast-open",
        "tag",
        "server_check_url",
        "reality-base64-pubkey",
        "reality-hex-shortid",
    ]
    .contains(&key)
    {
        return true;
    }
    if key == "over-tls" || key == "tls-host" {
        return kind != "shadowsocks";
    }
    if key == "udp-over-tcp" {
        return kind != "anytls";
    }
    if ["obfs", "obfs-host", "obfs-uri"].contains(&key) {
        return ["shadowsocks", "vmess", "vless", "trojan"].contains(&kind);
    }
    if key == "method" {
        return ["shadowsocks", "vmess", "vless"].contains(&kind);
    }
    if key == "aead" {
        return ["vmess", "vless"].contains(&kind);
    }
    if key == "username" {
        return ["http", "socks5"].contains(&kind);
    }
    if key == "ssr-protocol" || key == "ssr-protocol-param" {
        return kind == "shadowsocks";
    }
    key == "vless-flow" && kind == "vless"
}

fn qx_line(line: &str) -> Option<Value> {
    let (kind, tail) = line.split_once('=')?;
    let kind = kind.trim_end_matches([' ', '\r', '\t']);
    if ![
        "shadowsocks",
        "vmess",
        "vless",
        "trojan",
        "anytls",
        "http",
        "socks5",
    ]
    .contains(&kind)
    {
        return None;
    }
    let tail = space(tail);
    let address_end = tail.find(',').unwrap_or(tail.len());
    let address = &tail[..address_end];
    let colon = address.rfind(':')?;
    if colon == 0 {
        return None;
    }
    let port = address[colon + 1..].trim_end_matches([' ', '\r', '\t']);
    if port.is_empty() || !port.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let mut proxy = json!({"server":codec::trim(&address[..colon])});
    if let Ok(port) = port.parse::<u16>() {
        proxy["port"] = json!(port);
    }
    let mut rest = &tail[address_end..];
    let mut obfs = json!({});
    while !rest.is_empty() {
        let (key, raw) = field(&mut rest, kind, false)?;
        if !qx_allowed(kind, key) {
            if raw.contains('=') {
                return None;
            }
            continue;
        }
        match key {
            "password" => {
                proxy[if ["vmess", "vless"].contains(&kind) {
                    "uuid"
                } else {
                    "password"
                }] = json!(codec::trim(raw));
            }
            "username" => proxy[key] = json!(codec::trim(raw)),
            "tag"
            | "tls-pubkey-sha256"
            | "tls-alpn"
            | "ssr-protocol-param"
            | "reality-base64-pubkey"
            | "reality-hex-shortid"
            | "vless-flow"
            | "server_check_url" => {
                if raw.contains('=') {
                    return None;
                }
                match key {
                    "reality-base64-pubkey" => proxy["reality-opts"]["public-key"] = json!(raw),
                    "reality-hex-shortid" => proxy["reality-opts"]["short-id"] = json!(raw),
                    _ => {
                        let output = match key {
                            "tag" => "name",
                            "ssr-protocol-param" => "protocol-param",
                            "vless-flow" => "flow",
                            "server_check_url" => "test-url",
                            _ => key,
                        };
                        proxy[output] = json!(raw);
                    }
                }
            }
            "tls-host" => proxy["sni"] = json!(strip_quotes(raw, false, false)),
            "tls-cert-sha256" => proxy["tls-fingerprint"] = json!(codec::trim(raw)),
            "tls-verification" => {
                let value = codec::trim(raw);
                if ["true", "false"].contains(&value) {
                    proxy["skip-cert-verify"] = json!(value != "true");
                } else {
                    proxy["name-cert-verify"] = json!(value);
                }
            }
            "tls-no-session-ticket"
            | "tls-no-session-reuse"
            | "over-tls"
            | "udp-relay"
            | "fast-open"
            | "aead" => {
                if let Some(value) = boolean(raw)? {
                    let output = match key {
                        "over-tls" => "tls",
                        "udp-relay" => "udp",
                        "fast-open" => "tfo",
                        _ => key,
                    };
                    proxy[output] = value;
                } else if raw.contains('=') {
                    return None;
                }
            }
            "udp-over-tcp" => {
                if kind == "shadowsocks" {
                    if raw.contains('=') {
                        return None;
                    }
                    match raw {
                        "sp.v1" | "sp.v2" => {
                            proxy[key] = json!(true);
                            proxy["udp-over-tcp-version"] =
                                json!(if raw == "sp.v1" { 1 } else { 2 });
                        }
                        "true" => proxy["_ssr_python_uot"] = json!(true),
                        _ => return None,
                    }
                } else if boolean(raw)?.is_some() || raw.contains('=') {
                    return None;
                }
            }
            "method" => {
                let values = [
                    "aes-128-cfb",
                    "aes-128-ctr",
                    "aes-128-gcm",
                    "aes-192-cfb",
                    "aes-192-ctr",
                    "aes-192-gcm",
                    "aes-256-cfb",
                    "aes-256-ctr",
                    "aes-256-gcm",
                    "bf-cfb",
                    "cast5-cfb",
                    "chacha20-ietf-poly1305",
                    "chacha20-ietf",
                    "chacha20-poly1305",
                    "chacha20",
                    "des-cfb",
                    "none",
                    "rc2-cfb",
                    "rc4-md5-6",
                    "rc4-md5",
                    "salsa20",
                    "xchacha20-ietf-poly1305",
                    "2022-blake3-aes-128-gcm",
                    "2022-blake3-aes-256-gcm",
                ];
                if let Some(value) = fixed_token(raw, &values)? {
                    proxy["cipher"] = json!(value);
                } else if raw.contains('=') {
                    return None;
                }
            }
            "ssr-protocol" => {
                let values = [
                    "origin",
                    "auth_sha1_v4",
                    "auth_aes128_md5",
                    "auth_aes128_sha1",
                    "auth_chain_a",
                    "auth_chain_b",
                ];
                if let Some(value) = fixed_token(raw, &values)? {
                    proxy["protocol"] = json!(value);
                } else if raw.contains('=') {
                    return None;
                }
            }
            "obfs" => {
                let values: &[&str] = if kind == "shadowsocks" {
                    &[
                        "plain",
                        "http_simple",
                        "http_post",
                        "random_head",
                        "tls1.2_ticket_auth",
                        "tls1.2_ticket_fastauth",
                        "tls",
                        "wss",
                        "ws",
                        "over-tls",
                        "http",
                        "vmess-http",
                        "vemss-http",
                        "shadowsocks-http",
                    ]
                } else if ["vmess", "vless"].contains(&kind) {
                    &[
                        "wss",
                        "ws",
                        "over-tls",
                        "http",
                        "vmess-http",
                        "vemss-http",
                        "shadowsocks-http",
                    ]
                } else {
                    &["wss", "ws", "over-tls", "http"]
                };
                if let Some(value) = fixed_token(raw, values)? {
                    if kind == "shadowsocks"
                        && [
                            "plain",
                            "http_simple",
                            "http_post",
                            "random_head",
                            "tls1.2_ticket_auth",
                            "tls1.2_ticket_fastauth",
                        ]
                        .contains(&value)
                    {
                        proxy["type"] = json!("ssr");
                    }
                    if kind != "trojan"
                        && ["http", "vmess-http", "vemss-http", "shadowsocks-http"].contains(&value)
                    {
                        proxy["_qx_obfs_http"] = json!(value);
                        obfs["type"] = json!("http");
                    } else {
                        obfs["type"] = json!(value);
                    }
                } else if raw.contains('=') {
                    return None;
                }
            }
            "obfs-host" => obfs["host"] = json!(strip_quotes(raw, false, false)),
            "obfs-uri" => obfs["path"] = json!(raw),
            _ => return None,
        }
    }
    if kind == "shadowsocks" {
        if truthy(&proxy["protocol"]) || proxy["type"] == "ssr" {
            proxy["type"] = json!("ssr");
            if !truthy(&proxy["protocol"]) {
                proxy["protocol"] = json!("origin");
            }
            if truthy(&obfs["host"]) {
                proxy["obfs-param"] = obfs["host"].clone();
            }
            if truthy(&obfs["type"]) {
                proxy["obfs"] = obfs["type"].clone();
            }
        } else {
            proxy["type"] = json!("ss");
            let mode = obfs["type"].as_str();
            if matches!(mode, Some("http" | "tls")) {
                proxy["plugin"] = json!("obfs");
                proxy["plugin-opts"] = json!({"mode":obfs["type"]});
            } else if matches!(mode, Some("ws" | "wss")) {
                proxy["plugin"] = json!("v2ray-plugin");
                proxy["plugin-opts"] = json!({"mode":"websocket"});
                if mode == Some("wss") {
                    proxy["plugin-opts"]["tls"] = json!(true);
                }
            } else if mode == Some("over-tls") {
                proxy["tls"] = json!(true);
                if truthy(&obfs["host"]) {
                    proxy["sni"] = obfs["host"].clone();
                }
            }
            if mode.is_some() && mode != Some("over-tls") {
                for key in ["host", "path"] {
                    if let Some(value) = obfs.get(key) {
                        proxy["plugin-opts"][key] = value.clone();
                    }
                }
            }
        }
    } else {
        proxy["type"] = json!(kind);
        if ["vmess", "vless"].contains(&kind) {
            if !truthy(&proxy["cipher"]) {
                proxy["cipher"] = json!("none");
            }
            if kind == "vmess" {
                proxy["alterId"] = json!(if proxy["aead"] == false { 1 } else { 0 });
            }
        }
        if kind == "anytls" {
            proxy["tls"] = json!(true);
        }
        if ["trojan", "vmess", "vless"].contains(&kind) {
            let mode = obfs["type"].as_str();
            if matches!(mode, Some("ws" | "wss" | "http")) {
                let network = if mode == Some("http") { "http" } else { "ws" };
                proxy["network"] = json!(network);
                if mode == Some("wss") {
                    proxy["tls"] = json!(true);
                }
                // The grammar creates path before headers even when path is absent.
                let mut options = json!({"path":null});
                if let Some(value) = obfs.get("path") {
                    options["path"] = value.clone();
                }
                let mut headers = json!({});
                if let Some(value) = obfs.get("host") {
                    headers["Host"] = value.clone();
                }
                options["headers"] = headers;
                proxy[format!("{network}-opts")] = options;
            } else if mode == Some("over-tls") {
                proxy["tls"] = json!(true);
                if truthy(&obfs["host"]) && !truthy(&proxy["sni"]) {
                    proxy["sni"] = obfs["host"].clone();
                }
            }
        }
    }
    if let Some(alpn) = proxy
        .get("tls-alpn")
        .and_then(Value::as_str)
        .and_then(qx_alpn)
    {
        proxy["alpn"] = json!(alpn);
    }
    Some(proxy)
}

fn qx_alpn(raw: &str) -> Option<Vec<String>> {
    let raw = codec::trim(raw).replace(':', "");
    if raw.is_empty()
        || !raw.len().is_multiple_of(2)
        || !raw.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return None;
    }
    let (chunks, _) = raw.as_bytes().as_chunks::<2>();
    let bytes: Vec<_> = chunks
        .iter()
        .map(|hex| u8::from_str_radix(std::str::from_utf8(hex).ok()?, 16).ok())
        .collect::<Option<_>>()?;
    let mut result = Vec::new();
    let mut offset = 0;
    while offset < bytes.len() {
        let length = usize::from(bytes[offset]);
        offset += 1;
        if length == 0 || offset + length > bytes.len() {
            return None;
        }
        result.push(String::from_utf8_lossy(&bytes[offset..offset + length]).into_owned());
        offset += length;
    }
    Some(result)
}

fn wg_values<'a>(raw: &'a str, key: &str) -> Vec<&'a str> {
    std::iter::once(0)
        .chain(raw.match_indices(',').map(|(index, _)| index + 1))
        .filter_map(|start| {
            let part = raw[start..].trim_start_matches(char::is_whitespace);
            if !part.get(..key.len())?.eq_ignore_ascii_case(key) {
                return None;
            }
            part[key.len()..]
                .trim_start_matches(char::is_whitespace)
                .strip_prefix('=')
        })
        .collect()
}

fn wg_text<'a>(raw: &'a str, key: &str) -> Option<&'a str> {
    for tail in wg_values(raw, key) {
        for open in [true, false] {
            let value = if open {
                match tail.strip_prefix('"') {
                    Some(value) => value,
                    None => continue,
                }
            } else {
                tail
            };
            for (end, _) in value
                .char_indices()
                .skip(1)
                .chain(std::iter::once((value.len(), '\0')))
            {
                if end == 0 {
                    continue;
                }
                let rest = &value[end..];
                let rest = rest
                    .strip_prefix('"')
                    .unwrap_or(rest)
                    .trim_start_matches(char::is_whitespace);
                if rest.is_empty() || rest.starts_with(',') {
                    return Some(&value[..end]);
                }
            }
        }
    }
    None
}

fn wg_number(raw: &str, key: &str) -> Option<Value> {
    for tail in wg_values(raw, key) {
        let value = tail.trim_start_matches(char::is_whitespace);
        let value = value.strip_prefix('"').unwrap_or(value);
        let end = value.bytes().take_while(u8::is_ascii_digit).count();
        if end == 0 {
            continue;
        }
        let rest = value[end..]
            .strip_prefix('"')
            .unwrap_or(&value[end..])
            .trim_start_matches(char::is_whitespace);
        if rest.is_empty() || rest.starts_with(',') {
            let number = value[..end].parse::<f64>().ok()?;
            return Some(codec::numeric(number));
        }
    }
    None
}

fn wg_quoted<'a>(raw: &'a str, key: &str) -> Option<&'a str> {
    for tail in wg_values(raw, key) {
        let Some(value) = tail
            .trim_start_matches(char::is_whitespace)
            .strip_prefix('"')
        else {
            continue;
        };
        for (end, _) in value.match_indices('"') {
            if end == 0 {
                continue;
            }
            let rest = value[end + 1..].trim_start_matches(char::is_whitespace);
            if rest.is_empty() || rest.starts_with(',') {
                return Some(&value[..end]);
            }
        }
    }
    None
}

fn wg_server_dns(raw: &str) -> Option<Value> {
    for tail in wg_values(raw, "server-dns") {
        let tail = tail.trim_start_matches(char::is_whitespace);
        let (value, rest) = if let Some(quoted) = tail.strip_prefix('"') {
            let end = quoted.find('"')?;
            (&quoted[..end], &quoted[end + 1..])
        } else {
            let end = tail
                .match_indices(',')
                .find_map(|(index, _)| parameter_start(&tail[index + 1..], true).then_some(index))
                .unwrap_or(tail.len());
            if tail[..end].contains('"') {
                continue;
            }
            (&tail[..end], &tail[end..])
        };
        if rest.trim_start_matches(char::is_whitespace).is_empty()
            || rest
                .trim_start_matches(char::is_whitespace)
                .strip_prefix(',')
                .is_some_and(|part| parameter_start(part, true))
        {
            return Some(json!(
                value
                    .split(',')
                    .map(codec::trim)
                    .filter(|value| !value.is_empty())
                    .collect::<Vec<_>>()
            ));
        }
    }
    None
}

fn loon_wireguard(line: &str) -> Option<Value> {
    let (name, raw) = line.split_once('=')?;
    let (_, raw) = raw.split_once(',')?;
    let peer_tail = wg_values(raw, "peers").into_iter().find_map(|tail| {
        tail.trim_start_matches(char::is_whitespace)
            .strip_prefix('[')?
            .trim_start_matches(char::is_whitespace)
            .strip_prefix('{')
    })?;
    let peer_end = peer_tail.match_indices('}').find_map(|(index, _)| {
        peer_tail[index + 1..]
            .trim_start_matches(char::is_whitespace)
            .starts_with(']')
            .then_some(index)
    })?;
    if peer_end == 0 {
        return None;
    }
    let peers = &peer_tail[..peer_end];
    let endpoint = wg_text(peers, "endpoint");
    let endpoint = endpoint
        .and_then(|value| value.rsplit_once(':'))
        .filter(|(server, port)| {
            !server.is_empty() && !port.is_empty() && port.bytes().all(|byte| byte.is_ascii_digit())
        });
    let mut proxy = json!({"type":"wireguard","name":codec::trim(name)});
    if let Some((server, port)) = endpoint {
        proxy["server"] = json!(server);
        proxy["port"] = codec::numeric(port.parse::<f64>().ok()?);
    } else {
        proxy["port"] = codec::numeric(f64::NAN);
    }
    for (output, input) in [
        ("ip", "interface-ip"),
        ("ipv6", "interface-ipv6"),
        ("private-key", "private-key"),
    ] {
        if let Some(value) = wg_text(raw, input) {
            proxy[output] = json!(value);
        }
    }
    if let Some(value) = wg_text(peers, "public-key") {
        proxy["public-key"] = json!(value);
    }
    for key in ["mtu", "keepalive"] {
        if let Some(value) = wg_number(raw, key) {
            proxy[key] = value;
        }
    }
    for tail in wg_values(peers, "reserved") {
        let tail = tail.trim_start_matches(char::is_whitespace);
        let tail = tail.strip_prefix('"').unwrap_or(tail);
        if let Some(value) = tail.strip_prefix('[')
            && let Some(end) = value.find(']')
            && end > 0
        {
            let rest = value[end + 1..]
                .strip_prefix('"')
                .unwrap_or(&value[end + 1..])
                .trim_start_matches(char::is_whitespace);
            if rest.is_empty() || rest.starts_with(',') {
                proxy["reserved"] = codec::parse(&format!("[{}]", &value[..end])).ok()?;
                break;
            }
        }
    }
    if let Some(value) = wg_quoted(peers, "allowed-ips") {
        proxy["allowed-ips"] = json!(value.split(',').map(codec::trim).collect::<Vec<_>>());
    }
    if let Some(value) = wg_text(peers, "preshared-key") {
        proxy["preshared-key"] = json!(value);
    }
    let dns: Vec<_> = ["dns", "dnsv6"]
        .into_iter()
        .filter_map(|key| wg_text(raw, key))
        .collect();
    if !dns.is_empty() {
        proxy["dns"] = json!(dns);
    }
    if let Some(value) = wg_server_dns(raw) {
        proxy["server-dns"] = value;
    }
    proxy["udp"] = json!(true);
    let mut peer = json!({});
    for (output, input) in [
        ("server", "server"),
        ("port", "port"),
        ("ip", "ip"),
        ("ipv6", "ipv6"),
        ("public-key", "public-key"),
        ("pre-shared-key", "preshared-key"),
        ("allowed-ips", "allowed-ips"),
        ("reserved", "reserved"),
    ] {
        if let Some(value) = proxy.get(input) {
            peer[output] = value.clone();
        }
    }
    proxy["peers"] = json!([peer]);
    if !dns.is_empty() {
        proxy["remote-dns-resolve"] = json!(true);
    }
    Some(proxy)
}

fn try_loon(line: &str) -> Result<Option<(Value, Parser)>, Error> {
    let Some(kind) = loon_kind(line) else {
        return Ok(None);
    };
    let proxy = loon_line(line);
    Ok(proxy.map(|proxy| (proxy, Parser::Loon(kind))))
}

pub fn cached(line: &str, last: Option<Parser>) -> Result<Option<(Value, Parser)>, Error> {
    if let Some(Parser::Loon(kind)) = last
        && loon_kind(line) == Some(kind)
        && let Some(parsed) = try_loon(line)?
    {
        return Ok(Some(parsed));
    }
    Ok(None)
}

pub fn parse(line: &str) -> Result<Option<(Value, Parser)>, Error> {
    if crate::surge::matches(line)
        && let Some(proxy) = crate::surge::parse(line)?
    {
        return Ok(Some((proxy, Parser::Surge)));
    }
    if let Some(parsed) = try_loon(line)? {
        return Ok(Some(parsed));
    }
    Ok(qx_line(line).map(|proxy| (proxy, Parser::Qx)))
}
