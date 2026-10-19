use crate::classic::{boolean, digits, space, strip_quotes};
use crate::json as codec;
use crate::state::truthy;
use serde_json::{Value, json};

fn quoted_key(raw: &str) -> Option<usize> {
    let quote = *raw.as_bytes().first()?;
    if !b"\"'".contains(&quote) {
        return None;
    }
    let end = raw.as_bytes()[1..].iter().position(|byte| *byte == quote)? + 1;
    (end > 1).then_some(end + 1)
}

fn header_key(raw: &str) -> bool {
    let raw = raw.trim_start_matches(codec::is_space);
    let end = if let Some(end) = quoted_key(raw) {
        end
    } else {
        let end = raw
            .bytes()
            .take_while(|byte| byte.is_ascii_alphanumeric() || b"!#$%&'*+-.^_|~".contains(byte))
            .count();
        if end == 0 {
            return false;
        }
        end
    };
    raw[end..]
        .trim_start_matches(codec::is_space)
        .starts_with(':')
}

fn quoted_header_key(raw: &str) -> bool {
    let raw = codec::trim(raw);
    quoted_key(raw).is_some_and(|end| {
        raw[end..]
            .trim_start_matches(codec::is_space)
            .starts_with(':')
    })
}

fn option_start(raw: &str) -> bool {
    let raw = raw.trim_start_matches(codec::is_space);
    let end = raw
        .bytes()
        .take_while(|byte| byte.is_ascii_alphanumeric() || *byte == b'-')
        .count();
    end > 0
        && raw[end..]
            .trim_start_matches(codec::is_space)
            .starts_with('=')
}

fn quote_end(raw: &str, index: usize, separator: u8, comma: bool, container: Option<u8>) -> bool {
    let rest = raw[index + 1..].trim_start_matches(codec::is_space);
    if rest.is_empty() {
        return true;
    }
    if comma && rest.starts_with(',') && option_start(&rest[1..]) {
        return true;
    }
    if rest.as_bytes().first() == Some(&separator) && header_key(&rest[1..]) {
        return true;
    }
    container.is_some_and(|quote| {
        rest.as_bytes().first() == Some(&quote) && {
            let rest = rest[1..].trim_start_matches(codec::is_space);
            rest.is_empty() || rest.starts_with(',')
        }
    })
}

fn headers_end(raw: &str, separator: u8) -> usize {
    let whitespace = raw.len() - raw.trim_start_matches(codec::is_space).len();
    let trimmed = &raw[whitespace..];
    let container = trimmed
        .as_bytes()
        .first()
        .copied()
        .filter(|byte| b"\"'".contains(byte) && !quoted_header_key(trimmed));
    let mut index = if container.is_some() {
        whitespace + 1
    } else {
        0
    };
    let mut quote = None;
    let mut value = false;
    let mut colon = false;
    while index < raw.len() {
        let byte = raw.as_bytes()[index];
        if let Some(active) = quote {
            if byte == active
                && (!value || quote_end(raw, index, separator, container.is_none(), container))
            {
                quote = None;
            }
        } else if container == Some(byte) && {
            let rest = raw[index + 1..].trim_start_matches(codec::is_space);
            rest.is_empty() || rest.starts_with(',')
        } {
            return index + 1;
        } else if b"\"'".contains(&byte) {
            quote = Some(byte);
            value = colon;
        } else if byte == b':' && !colon {
            colon = true;
        } else if byte == separator && header_key(&raw[index + 1..]) {
            colon = false;
        } else if byte == b',' && container.is_none() {
            break;
        }
        index += 1;
    }
    index
}

fn parse_headers(raw: &str, separator: u8) -> Value {
    let raw = codec::trim(raw);
    let raw = if !quoted_header_key(raw) {
        strip_quotes(raw, true, false)
    } else {
        raw
    };
    let mut pairs = Vec::new();
    let mut start = 0;
    let mut quote = None;
    let mut value = false;
    let mut colon = false;
    for (index, byte) in raw.bytes().enumerate() {
        if let Some(active) = quote {
            if byte == active && (!value || quote_end(raw, index, separator, false, None)) {
                quote = None;
            }
        } else if b"\"'".contains(&byte) {
            quote = Some(byte);
            value = colon;
        } else if byte == b':' && !colon {
            colon = true;
        } else if byte == separator && header_key(&raw[index + 1..]) {
            pairs.push(&raw[start..index]);
            start = index + 1;
            colon = false;
        }
    }
    pairs.push(&raw[start..]);
    let mut headers = json!({});
    for pair in pairs {
        let mut quote = None;
        let separator = pair.bytes().enumerate().find_map(|(index, byte)| {
            if quote == Some(byte) {
                quote = None;
                None
            } else if quote.is_some() {
                None
            } else if b"\"'".contains(&byte) {
                quote = Some(byte);
                None
            } else {
                (byte == b':').then_some(index)
            }
        });
        if let Some(index) = separator {
            let key = strip_quotes(&pair[..index], true, true);
            if !key.is_empty() && key != "__proto__" {
                headers[key] = json!(strip_quotes(&pair[index + 1..], true, true));
            }
        }
    }
    headers
}

fn allowed(kind: &str, key: &str) -> bool {
    if [
        "ip-version",
        "underlying-proxy",
        "tos",
        "allow-other-interface",
        "interface",
        "test-url",
        "test-udp",
        "test-timeout",
        "hybrid",
        "no-error-alert",
        "block-quic",
    ]
    .contains(&key)
    {
        return true;
    }
    if ["fast-open", "tfo"].contains(&key) {
        return kind != "wireguard";
    }
    if [
        "shadow-tls-version",
        "shadow-tls-sni",
        "shadow-tls-password",
    ]
    .contains(&key)
    {
        return !["direct", "anytls", "trust-tunnel", "masque"].contains(&kind);
    }
    if [
        "sni",
        "server-cert-verify-name",
        "server-cert-fingerprint-sha256",
        "skip-cert-verify",
        "client-cert",
    ]
    .contains(&key)
    {
        return [
            "vmess",
            "trojan",
            "https",
            "h2-connect",
            "tuic",
            "tuic-v5",
            "hysteria2",
            "socks5-tls",
            "anytls",
            "trust-tunnel",
            "masque",
        ]
        .contains(&kind);
    }
    if key == "alpn" {
        return !["http", "ssh", "socks5", "wireguard", "direct"].contains(&kind);
    }
    if key == "udp-relay" {
        return [
            "ss",
            "vmess",
            "trojan",
            "snell",
            "socks5",
            "socks5-tls",
            "masque",
            "direct",
        ]
        .contains(&kind);
    }
    match kind {
        "ss" => [
            "encrypt-method",
            "password",
            "obfs",
            "obfs-host",
            "obfs-uri",
            "udp-port",
        ]
        .contains(&key),
        "vmess" => [
            "username",
            "vmess-aead",
            "ws",
            "ws-path",
            "ws-headers",
            "encrypt-method",
            "tls",
        ]
        .contains(&key),
        "trojan" => ["password", "ws", "ws-path", "ws-headers", "tls"].contains(&key),
        "https" | "http" => key == "headers",
        "h2-connect" => ["headers", "max-streams"].contains(&key),
        "ssh" => ["server-fingerprint", "idle-timeout", "private-key"].contains(&key),
        "snell" => [
            "version",
            "mode",
            "psk",
            "obfs",
            "obfs-host",
            "obfs-uri",
            "reuse",
        ]
        .contains(&key),
        "tuic" => ["token", "ecn", "port-hopping-interval"].contains(&key),
        "tuic-v5" => ["password", "uuid", "ecn", "port-hopping-interval"].contains(&key),
        "wireguard" => key == "section-name",
        "hysteria2" => [
            "password",
            "download-bandwidth",
            "ecn",
            "port-hopping-interval",
            "salamander-password",
            "gecko-password",
        ]
        .contains(&key),
        "anytls" => ["password", "reuse"].contains(&key),
        "trust-tunnel" => [
            "username",
            "password",
            "headers",
            "max-streams",
            "reuse",
            "h3",
        ]
        .contains(&key),
        "masque" => ["username", "password", "port-hopping-interval", "ecn"].contains(&key),
        _ => false,
    }
}

fn next_option<'a>(rest: &mut &'a str, kind: &str) -> Option<(&'a str, &'a str)> {
    let raw = space(rest).strip_prefix(',')?;
    let raw = space(raw);
    let equals = raw.find('=')?;
    if raw[..equals].contains(',') || raw[..equals].is_empty() {
        return None;
    }
    let key = raw[..equals].trim_end_matches([' ', '\r', '\t']);
    let value = space(&raw[equals + 1..]);
    let end = if allowed(kind, key) && ["headers", "ws-headers"].contains(&key) {
        headers_end(value, if key == "headers" { b';' } else { b'|' })
    } else if allowed(kind, key) && key == "alpn" && value.starts_with(['"', '\'']) {
        let quote = value.as_bytes()[0];
        value.as_bytes()[1..]
            .iter()
            .position(|byte| *byte == quote)
            .map_or_else(|| value.find(',').unwrap_or(value.len()), |end| end + 2)
    } else {
        value.find(',').unwrap_or(value.len())
    };
    if end == 0 {
        return None;
    }
    *rest = &value[end..];
    Some((key, &value[..end]))
}

fn hopping(raw: &str) -> (String, Option<String>) {
    let mut offset = 0;
    while let Some(comma) = raw[offset..].find(',').map(|index| offset + index) {
        let tail = raw[comma + 1..].trim_start_matches(codec::is_space);
        let Some(tail) = tail.strip_prefix("port-hopping") else {
            offset = comma + 1;
            continue;
        };
        let Some(tail) = tail.trim_start_matches(codec::is_space).strip_prefix('=') else {
            offset = comma + 1;
            continue;
        };
        let tail = tail.trim_start_matches(codec::is_space);
        let tail = tail
            .strip_prefix(['"', '\''])
            .unwrap_or(tail)
            .trim_start_matches(codec::is_space);
        let start = raw.len() - tail.len();
        let bytes = tail.as_bytes();
        let range_end = |start: usize| {
            let mut end = start;
            while bytes.get(end).is_some_and(u8::is_ascii_digit) {
                end += 1;
            }
            if end == start {
                return None;
            }
            if bytes.get(end) == Some(&b'-') {
                let next = end + 1;
                let mut last = next;
                while bytes.get(last).is_some_and(u8::is_ascii_digit) {
                    last += 1;
                }
                if last > next {
                    end = last;
                }
            }
            Some(end)
        };
        let Some(mut end) = range_end(0) else {
            offset = comma + 1;
            continue;
        };
        while matches!(bytes.get(end), Some(b',' | b';')) {
            if let Some(next) = range_end(end + 1) {
                end = next;
            } else {
                break;
            }
        }
        let after = &raw[start + end..];
        let after = after.strip_prefix(['"', '\'']).unwrap_or(after);
        let mut line = raw[..comma].to_owned();
        line.push_str(after);
        return (line, Some(raw[start..start + end].replace(';', ",")));
    }
    (raw.to_owned(), None)
}

pub fn matches(line: &str) -> bool {
    let first = line.split(',').next().unwrap_or("");
    let Some((_, kind)) = first.rsplit_once('=') else {
        return false;
    };
    let kind = kind.trim_start_matches(codec::is_space);
    let known = [
        "direct",
        "anytls",
        "trust-tunnel",
        "masque",
        "h2-connect",
        "ssh",
        "ss",
        "vmess",
        "trojan",
        "http",
        "snell",
        "tuic",
        "wireguard",
        "hysteria2",
        "socks5",
        "external",
    ];
    if !known.iter().any(|known| kind.starts_with(known)) {
        return false;
    }
    if kind.starts_with("vmess") && !line.contains("username") {
        return false;
    }
    if kind.starts_with("http") || kind.starts_with("socks5") {
        let loon_only = [
            "fast-open",
            "over-tls",
            "tls-name",
            "ip-mode",
            "tls-cert-sha256",
            "tls-pubkey-sha256",
            "server-dns",
        ];
        if line.split(',').any(|part| {
            part.split_once('=').is_some_and(|(key, _)| {
                loon_only
                    .iter()
                    .any(|option| codec::trim(key).eq_ignore_ascii_case(option))
            })
        }) {
            return false;
        }
    }
    true
}

pub fn parse(line: &str) -> Result<Option<Value>, crate::conversion::Error> {
    Ok(parse_line(line))
}

fn parse_line(line: &str) -> Option<Value> {
    let (name, raw) = line.split_once('=')?;
    if name.contains(',') {
        return None;
    }
    let raw = space(raw);
    let (kind, rest) = raw
        .find(',')
        .map_or((raw, ""), |index| (&raw[..index], &raw[index..]));
    let kind = kind.trim_end_matches([' ', '\r', '\t']);
    if kind == "external" {
        return external(name, rest);
    }
    if ![
        "direct",
        "anytls",
        "trust-tunnel",
        "masque",
        "h2-connect",
        "ssh",
        "ss",
        "vmess",
        "trojan",
        "http",
        "https",
        "snell",
        "tuic",
        "tuic-v5",
        "wireguard",
        "hysteria2",
        "socks5",
        "socks5-tls",
    ]
    .contains(&kind)
    {
        return None;
    }
    let (hopped, ports) = if ["masque", "tuic", "tuic-v5", "hysteria2"].contains(&kind) {
        hopping(rest)
    } else {
        (rest.to_owned(), None)
    };
    let mut rest = hopped.as_str();
    let mut proxy = json!({"name":codec::trim(name)});
    if !["wireguard", "direct"].contains(&kind) {
        let tail = space(rest).strip_prefix(',').map(space)?;
        let (server, tail) = tail.split_once(',')?;
        proxy["server"] = json!(codec::trim(server));
        let tail = space(tail);
        let end = tail.bytes().take_while(u8::is_ascii_digit).count();
        if end == 0 {
            return None;
        }
        if let Ok(port) = tail[..end].parse::<u16>() {
            proxy["port"] = json!(port);
        }
        rest = &tail[end..];
    }
    if ["http", "https", "h2-connect", "socks5", "socks5-tls", "ssh"].contains(&kind) {
        if let Some(tail) = space(rest).strip_prefix(',')
            && let Some((user, tail)) = tail.split_once(',').filter(|(user, _)| !user.contains('='))
        {
            proxy["username"] = json!(strip_quotes(user, true, true));
            let tail = space(tail);
            let end = tail.find(',').unwrap_or(tail.len());
            if end == 0 {
                return None;
            }
            proxy["password"] = json!(strip_quotes(&tail[..end], true, false));
            rest = &tail[end..];
        }
        let initial = rest;
        if let Some((key, value)) = next_option(&mut rest, kind) {
            if key == "username" {
                proxy["username"] = json!(strip_quotes(value, true, false));
                if let Some((key, value)) = next_option(&mut rest, kind) {
                    if key == "password" {
                        proxy["password"] = json!(strip_quotes(value, true, false));
                    } else {
                        rest = initial;
                    }
                } else {
                    rest = initial;
                }
            } else {
                rest = initial;
            }
        } else {
            rest = initial;
        }
    }
    let mut obfs = json!({});
    let mut shadow = json!({});
    while !rest.is_empty() {
        let (key, raw) = next_option(&mut rest, kind)?;
        if !allowed(kind, key) {
            if raw.contains('=') {
                return None;
            }
            continue;
        }
        let output_key = match key {
            "fast-open" => "tfo",
            "udp-relay" => "udp",
            "server-cert-verify-name" => "name-cert-verify",
            "server-cert-fingerprint-sha256" => "tls-fingerprint",
            "client-cert" => "keystore-client-cert",
            "private-key" => "keystore-private-key",
            "download-bandwidth" => "down",
            "port-hopping-interval" => "hop-interval",
            "vmess-aead" => "aead",
            "encrypt-method" => "cipher",
            "username" if kind == "vmess" => "uuid",
            _ => key,
        };
        match key {
            "tls"
            | "skip-cert-verify"
            | "fast-open"
            | "tfo"
            | "udp-relay"
            | "reuse"
            | "ecn"
            | "allow-other-interface"
            | "hybrid"
            | "vmess-aead"
            | "h3"
            | "ws" => {
                if let Some(value) = boolean(raw)? {
                    if key == "ws" {
                        obfs["type"] = json!("ws");
                    } else if key == "h3" {
                        if value == true {
                            proxy["network"] = json!("h3");
                        }
                    } else {
                        proxy[output_key] = value;
                    }
                } else if raw.contains('=') {
                    return None;
                }
            }
            "version"
            | "test-timeout"
            | "tos"
            | "idle-timeout"
            | "udp-port"
            | "shadow-tls-version"
            | "port-hopping-interval" => {
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
            "max-streams" => {
                let raw = raw.trim_end_matches([' ', '\r', '\t']);
                let quoted = raw.starts_with(['"', '\'']);
                let number = if quoted {
                    let raw = strip_quotes(raw, true, false);
                    if raw.is_empty() || !raw.bytes().all(|byte| byte.is_ascii_digit()) {
                        None
                    } else {
                        digits(raw)?
                    }
                } else {
                    digits(raw)?
                };
                if let Some(value) = number {
                    proxy[output_key] = value;
                } else if raw.contains('=') {
                    return None;
                }
            }
            "headers" => {
                if !codec::trim(raw).is_empty() {
                    proxy["headers"] = parse_headers(raw, b';');
                }
            }
            "ws-headers" => {
                if !codec::trim(raw).is_empty() {
                    obfs["headers"] = parse_headers(raw, b'|');
                }
            }
            "ws-path" => {
                obfs["path"] = json!(strip_quotes(raw, true, true));
            }
            "obfs-uri" => {
                obfs["path"] = json!(raw);
            }
            "obfs-host" => {
                obfs["host"] = json!(strip_quotes(raw, false, false));
            }
            "obfs" => {
                let raw = raw.trim_end_matches([' ', '\r', '\t']);
                if ["http", "tls"].contains(&raw) {
                    obfs["type"] = json!(raw);
                } else if ["http", "tls"].iter().any(|prefix| raw.starts_with(prefix))
                    || raw.contains('=')
                {
                    return None;
                }
            }
            "sni" => {
                let sni = strip_quotes(raw, false, false);
                if sni == "off" {
                    proxy["disable-sni"] = json!(true);
                } else {
                    proxy["sni"] = json!(sni);
                }
            }
            "shadow-tls-sni" => {
                shadow["host"] = json!(raw);
            }
            "shadow-tls-password" => {
                shadow["password"] = json!(strip_quotes(raw, true, false));
            }
            "encrypt-method" if kind == "vmess" => {
                proxy["cipher"] = json!(match codec::trim(raw).to_lowercase().as_str() {
                    "aes-128-gcm" => "aes-128-gcm",
                    "chacha20-ietf-poly1305" => "chacha20-poly1305",
                    _ => "auto",
                });
            }
            "encrypt-method" => {
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
                    "bf-cfb",
                    "camellia-128-cfb",
                    "camellia-192-cfb",
                    "camellia-256-cfb",
                    "cast5-cfb",
                    "chacha20-ietf-poly1305",
                    "chacha20-ietf",
                    "chacha20-poly1305",
                    "chacha20",
                    "des-cfb",
                    "idea-cfb",
                    "none",
                    "rc2-cfb",
                    "rc4-md5",
                    "rc4",
                    "salsa20",
                    "seed-cfb",
                    "xchacha20-ietf-poly1305",
                    "2022-blake3-aes-128-gcm",
                    "2022-blake3-aes-256-gcm",
                ];
                let raw = raw.trim_end_matches([' ', '\r', '\t']);
                if ciphers.contains(&raw) {
                    proxy["cipher"] = json!(raw);
                } else if ciphers.iter().any(|cipher| raw.starts_with(cipher)) || raw.contains('=')
                {
                    return None;
                }
            }
            "mode" => {
                let mode = codec::trim(strip_quotes(raw, true, true));
                if ["default", "unshaped", "unsafe-raw"].contains(&mode) {
                    proxy["mode"] = json!(mode);
                }
            }
            "alpn" => {
                let raw = if raw.starts_with(['"', '\'']) {
                    strip_quotes(raw, true, false)
                } else {
                    raw
                };
                let values: Vec<_> = strip_quotes(raw, true, true)
                    .split(',')
                    .map(codec::trim)
                    .filter(|value| !value.is_empty())
                    .collect();
                if !values.is_empty() {
                    proxy["alpn"] = json!(values);
                }
            }
            "server-cert-verify-name" | "client-cert" | "private-key" => {
                proxy[output_key] = json!(strip_quotes(raw, true, true));
            }
            "server-fingerprint" => {
                proxy[output_key] = json!(strip_quotes(raw, false, false));
            }
            "server-cert-fingerprint-sha256" => {
                proxy[output_key] = json!(codec::trim(raw));
            }
            "password" | "username" | "psk" => {
                proxy[output_key] = json!(if kind == "vmess" && key == "username" {
                    raw
                } else {
                    strip_quotes(raw, true, false)
                });
            }
            "salamander-password" | "gecko-password" => {
                proxy["obfs-password"] = json!(strip_quotes(raw, true, false));
                proxy["obfs"] = json!(if key == "salamander-password" {
                    "salamander"
                } else {
                    "gecko"
                });
            }
            _ => {
                proxy[output_key] = json!(raw);
            }
        }
    }
    proxy["type"] = json!(match kind {
        "https" => "http",
        "socks5-tls" => "socks5",
        "tuic-v5" => "tuic",
        "wireguard" => "wireguard-surge",
        "trust-tunnel" => "trusttunnel",
        "masque" => "masque-surge",
        kind => kind,
    });
    if kind == "vmess" {
        if !truthy(&proxy["cipher"]) {
            proxy["cipher"] = json!("auto");
        }
        proxy["alterId"] = json!(if truthy(&proxy["aead"]) { 0 } else { 1 });
    }
    if [
        "https",
        "h2-connect",
        "socks5-tls",
        "anytls",
        "trust-tunnel",
        "masque",
    ]
    .contains(&kind)
    {
        proxy["tls"] = json!(true);
    }
    if kind == "tuic-v5" {
        proxy["version"] = json!(5);
    }
    if ["ss", "snell"].contains(&kind) && matches!(obfs["type"].as_str(), Some("http" | "tls")) {
        let options_key = if kind == "ss" {
            proxy["plugin"] = json!("obfs");
            "plugin-opts"
        } else {
            "obfs-opts"
        };
        let mut options = json!({"mode":obfs["type"]});
        for key in ["host", "path"] {
            if let Some(value) = obfs.get(key) {
                options[key] = value.clone();
            }
        }
        proxy[options_key] = options;
    }
    if ["vmess", "trojan"].contains(&kind) && obfs["type"] == "ws" {
        proxy["network"] = json!("ws");
        let mut options = json!({});
        if let Some(value) = obfs.get("path") {
            options["path"] = value.clone();
        }
        if let Some(value) = obfs.get("headers") {
            options["headers"] = value.clone();
        }
        if let Some(host) = options["headers"]["Host"].as_str() {
            options["headers"]["Host"] = json!(strip_quotes(host, false, false));
        }
        proxy["ws-opts"] = options;
    }
    if truthy(&shadow["password"]) {
        if !truthy(&shadow["version"]) {
            shadow["version"] = json!(2);
        }
        if shadow["version"]
            .as_f64()
            .is_some_and(|version| version < 2.0)
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
    if let Some(ports) = ports {
        proxy["ports"] = json!(ports);
    }
    Some(proxy)
}

fn external_fields<'a>(raw: &'a str, key: &str) -> Vec<(&'a str, Option<&'a str>)> {
    let mut fields = Vec::new();
    let mut offset = 0;
    while offset < raw.len() {
        let tail = raw[offset..].trim_start_matches(codec::is_space);
        if let Some(value) = tail
            .strip_prefix(key)
            .and_then(|tail| tail.trim_start_matches(codec::is_space).strip_prefix('='))
        {
            let value = value.trim_start_matches(codec::is_space);
            let plain_end = value.find(',').unwrap_or(value.len());
            let plain = value[..plain_end].trim_end_matches(codec::is_space);
            let quoted = value.strip_prefix('"').and_then(|quoted| {
                quoted.bytes().enumerate().find_map(|(index, byte)| {
                    if byte != b'"' {
                        return None;
                    }
                    let after = quoted[index + 1..].trim_start_matches(codec::is_space);
                    (after.is_empty() || after.starts_with(',')).then_some(&quoted[..index])
                })
            });
            fields.push((plain, quoted));
            offset = raw.len() - value.len() + quoted.map_or(plain_end, |value| value.len() + 2);
        }
        let Some(next) = raw[offset..].find(',') else {
            break;
        };
        offset += next + 1;
    }
    fields
}

fn external(name: &str, raw: &str) -> Option<Value> {
    let raw = raw.strip_prefix(',')?;
    let first = |key| {
        let fields = external_fields(raw, key);
        fields
            .iter()
            .find_map(|(_, quoted)| *quoted)
            .filter(|value| !value.is_empty())
            .or_else(|| fields.first().map(|(plain, _)| *plain))
            .map(|value| json!(value))
    };
    let mut proxy = json!({"type":"external","name":codec::trim(name)});
    if let Some(value) = first("exec") {
        proxy["exec"] = value;
    }
    if let Some(value) = first("local-port") {
        proxy["local-port"] = value;
    }
    proxy["args"] = json!(
        external_fields(raw, "args")
            .iter()
            .map(|(plain, quoted)| quoted.unwrap_or(plain))
            .collect::<Vec<_>>()
    );
    proxy["addresses"] = json!(
        external_fields(raw, "addresses")
            .iter()
            .filter_map(|(plain, quoted)| {
                let value = codec::trim(quoted.unwrap_or(plain))
                    .trim_start_matches('[')
                    .trim_end_matches(']');
                crate::conversion::is_ip(value).then_some(value)
            })
            .collect::<Vec<_>>()
    );
    Some(proxy)
}

pub fn is_full_config(raw: &str) -> bool {
    raw.split('\n')
        .any(|line| line.starts_with("[server_local]") || line.starts_with("[Proxy]"))
}

pub fn full_config(raw: &str) -> Option<&str> {
    if !is_full_config(raw) {
        return None;
    }
    // Preserve the frozen alternation: server_local leaves the entire input intact.
    for (index, _) in raw.char_indices() {
        let tail = &raw[index..];
        if (index == 0 || raw.as_bytes()[index - 1] == b'\n')
            && tail
                .get(..13)
                .is_some_and(|prefix| prefix.eq_ignore_ascii_case("[server_local"))
        {
            return Some(raw);
        }
        if !tail
            .get(..6)
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case("Proxy]"))
        {
            continue;
        }
        let start = index + 6;
        let mut offset = start;
        for line in raw[start..].split_inclusive('\n') {
            let section = line.strip_suffix('\n').unwrap_or(line);
            let section = section.strip_suffix('\r').unwrap_or(section);
            if offset > start
                && (offset == 0 || raw.as_bytes()[offset - 1] == b'\n')
                && section.starts_with('[')
                && section.ends_with(']')
                && section.len() > 2
            {
                return Some(&raw[start..offset]);
            }
            offset += line.len();
        }
    }
    Some(raw)
}
