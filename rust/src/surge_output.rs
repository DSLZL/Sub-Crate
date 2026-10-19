use crate::conversion::{Error, field_text as text, is_ip, vmess_security};
use crate::json as codec;
use crate::state::{legacy_object_order, truthy};
use crate::text_output::{append, external, field, finish, param, present};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::{Value, json};

#[derive(Clone, Copy, PartialEq)]
enum Failure {
    Unsupported,
    Invalid,
}

fn unquote(value: &Value) -> String {
    let Some(raw) = value.as_str() else {
        return codec::js_string(value);
    };
    let raw = codec::trim(raw);
    if raw.starts_with(['\'', '"']) && raw.as_bytes().first() == raw.as_bytes().last() {
        raw.get(1..raw.len().saturating_sub(1))
            .unwrap_or("")
            .to_owned()
    } else {
        raw.to_owned()
    }
}

fn quoted(out: &mut String, proxy: &Value, path: &str, label: &str) {
    if let Some(value) = field(proxy, path).filter(|value| !value.is_null()) {
        out.push_str(&format!(",{label}=\"{}\"", unquote(value)));
    }
}

fn nonblank(proxy: &Value, key: &str) -> bool {
    proxy
        .get(key)
        .is_some_and(|value| !value.is_null() && !codec::trim(&codec::js_string(value)).is_empty())
}

fn ip(out: &mut String, proxy: &Value) {
    if present(proxy, "ip-version") {
        let value = text(proxy, "ip-version");
        let value = match value.as_str() {
            "ipv4" => "v4-only",
            "ipv6" => "v6-only",
            "ipv4-prefer" => "prefer-v4",
            "ipv6-prefer" => "prefer-v6",
            _ => &value,
        };
        out.push_str(&format!(",ip-version={value}"));
    }
}

fn alpn(proxy: &Value, path: &str) -> String {
    let values = match field(proxy, path) {
        Some(Value::Array(values)) => values
            .iter()
            .filter(|value| !value.is_null())
            .map(unquote)
            .collect::<Vec<_>>(),
        Some(value) if truthy(value) => unquote(value).split(',').map(str::to_owned).collect(),
        _ => Vec::new(),
    };
    values
        .iter()
        .map(|value| codec::trim(value))
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>()
        .join(",")
}

fn tls(out: &mut String, proxy: &Value, enabled: bool, surfboard: bool) {
    if !enabled {
        return;
    }
    param(
        out,
        proxy,
        "tls-fingerprint",
        "server-cert-fingerprint-sha256",
    );
    append(out, proxy, "sni", ",sni=\"", "\"");
    if !surfboard {
        quoted(out, proxy, "name-cert-verify", "server-cert-verify-name");
        if proxy["plugin"] != "shadow-tls" {
            let value = alpn(proxy, "alpn");
            if !value.is_empty() {
                out.push_str(&format!(",alpn=\"{value}\""));
            }
        }
    }
    param(out, proxy, "skip-cert-verify", "skip-cert-verify");
    if !surfboard {
        let key = if present(proxy, "keystore-client-cert") {
            "keystore-client-cert"
        } else {
            "client-cert"
        };
        quoted(out, proxy, key, "client-cert");
    }
}

fn shadow(out: &mut String, proxy: &Value, udp: bool) -> Result<(), Failure> {
    if proxy["plugin"] != "shadow-tls" || !truthy(&proxy["plugin-opts"]) {
        return Ok(());
    }
    let options = &proxy["plugin-opts"];
    if !truthy(&options["password"]) {
        return Ok(());
    }
    append(out, options, "password", ",shadow-tls-password=\"", "\"");
    if truthy(&options["host"]) {
        param(out, options, "host", "shadow-tls-sni");
    }
    if truthy(&options["version"]) {
        if codec::to_number(&options["version"]) < 2.0 {
            return Err(Failure::Unsupported);
        }
        param(out, options, "version", "shadow-tls-version");
    }
    let value = if present(options, "alpn") {
        alpn(options, "alpn")
    } else {
        alpn(proxy, "alpn")
    };
    if !value.is_empty() {
        out.push_str(&format!(",alpn=\"{value}\""));
    }
    if udp {
        param(out, proxy, "udp-port", "udp-port");
    }
    Ok(())
}

fn headers(proxy: &Value, separator: &str, surfboard: bool) -> String {
    let raw = &proxy["headers"];
    if !surfboard && !raw.is_object() && !raw.is_array() {
        return String::new();
    }
    let fields = crate::body::Body::Json(legacy_object_order(raw.clone())).fields();
    fields
        .iter()
        .filter(|(key, value)| surfboard || (!codec::trim(key).is_empty() && !value.is_null()))
        .map(|(key, value)| {
            if surfboard {
                let value = codec::js_string(value);
                format!(
                    "{key}:{}",
                    if key == "Host" {
                        format!("\"{value}\"")
                    } else {
                        value
                    }
                )
            } else {
                format!("{key}:\"{}\"", unquote(value))
            }
        })
        .collect::<Vec<_>>()
        .join(separator)
}

fn root_headers(out: &mut String, proxy: &Value) {
    let value = headers(proxy, ";", false);
    if !codec::trim(&value).is_empty() {
        out.push_str(&format!(",headers=\"{}\"", unquote(&json!(value))));
    }
}

fn transport(
    out: &mut String,
    proxy: &Value,
    include: bool,
    surfboard: bool,
) -> Result<(), Failure> {
    if !present(proxy, "network") {
        return Ok(());
    }
    match proxy["network"].as_str() {
        Some("ws") => {
            out.push_str(",ws=true");
            param(out, proxy, "ws-opts.path", "ws-path");
            if present(proxy, "ws-opts.headers") {
                let value = headers(
                    &json!({"headers":proxy["ws-opts"]["headers"]}),
                    "|",
                    surfboard,
                );
                if !codec::trim(&value).is_empty() {
                    if surfboard {
                        out.push_str(&format!(",ws-headers={value}"));
                    } else {
                        out.push_str(&format!(",ws-headers=\"{}\"", unquote(&json!(value))));
                    }
                }
            }
        }
        Some("http") if include && !surfboard => {}
        Some("tcp") if !truthy(&proxy["reality-opts"]) => {}
        _ => return Err(Failure::Unsupported),
    }
    Ok(())
}

fn common(out: &mut String, proxy: &Value) {
    for (path, label) in [
        ("test-url", "test-url"),
        ("test-timeout", "test-timeout"),
        ("test-udp", "test-udp"),
        ("hybrid", "hybrid"),
        ("tos", "tos"),
        ("allow-other-interface", "allow-other-interface"),
        ("interface-name", "interface"),
        ("interface", "interface"),
    ] {
        param(out, proxy, path, label);
    }
}

fn hopping(out: &mut String, proxy: &Value) {
    if nonblank(proxy, "ports") {
        out.push_str(&format!(
            ",port-hopping=\"{}\"",
            text(proxy, "ports").replace(',', ";")
        ));
    }
    if nonblank(proxy, "hop-interval") {
        param(out, proxy, "hop-interval", "port-hopping-interval");
    }
}

fn tfo(out: &mut String, proxy: &Value, fallback: bool) {
    if present(proxy, "tfo") {
        param(out, proxy, "tfo", "tfo");
    } else if fallback {
        param(out, proxy, "fast-open", "tfo");
    }
}

fn wireguard(proxy: &mut Value) -> Result<String, Failure> {
    if let Some(peer) = proxy["peers"]
        .as_array()
        .and_then(|peers| peers.first())
        .cloned()
    {
        if peer.is_null() {
            return Err(Failure::Invalid);
        }
        for (from, to) in [
            ("server", "server"),
            ("port", "port"),
            ("ip", "ip"),
            ("ipv6", "ipv6"),
            ("public-key", "public-key"),
            ("pre-shared-key", "preshared-key"),
            ("allowed-ips", "allowed-ips"),
            ("reserved", "reserved"),
        ] {
            if let Some(value) = peer.get(from) {
                proxy[to] = value.clone();
            } else {
                proxy.as_object_mut().unwrap().shift_remove(to);
            }
        }
    }
    let name = text(proxy, "name");
    let mut out = format!("# > WireGuard Proxy {name}\n# {name}=wireguard");
    if !proxy["section-name"]
        .as_str()
        .is_some_and(|value| !codec::trim(value).is_empty())
    {
        proxy["section-name"] = proxy["name"].clone();
    }
    param(&mut out, proxy, "section-name", "section-name");
    param(&mut out, proxy, "no-error-alert", "no-error-alert");
    ip(&mut out, proxy);
    common(&mut out, proxy);
    shadow(&mut out, proxy, false)?;
    param(&mut out, proxy, "block-quic", "block-quic");
    param(&mut out, proxy, "underlying-proxy", "underlying-proxy");
    out.push_str(&format!(
        "\n# > WireGuard Section {name}\n[WireGuard {}]\nprivate-key = {}",
        text(proxy, "section-name"),
        text(proxy, "private-key")
    ));
    append(&mut out, proxy, "ip", "\nself-ip = ", "");
    append(&mut out, proxy, "ipv6", "\nself-ip-v6 = ", "");
    if truthy(&proxy["dns"]) {
        let value = if let Some(values) = proxy["dns"].as_array() {
            values
                .iter()
                .map(|value| {
                    if value.is_null() {
                        String::new()
                    } else {
                        codec::js_string(value)
                    }
                })
                .collect::<Vec<_>>()
                .join(", ")
        } else {
            text(proxy, "dns")
        };
        proxy["dns"] = json!(value);
        out.push_str(&format!("\ndns-server = {value}"));
    }
    append(&mut out, proxy, "mtu", "\nmtu = ", "");
    if text(proxy, "ip-version") == "ipv6-prefer" || text(proxy, "ip-version") == "prefer-v6" {
        out.push_str("\nprefer-ipv6 = true");
    }
    let mut peer = Vec::new();
    if present(proxy, "public-key") {
        peer.push(format!("public-key = {}", text(proxy, "public-key")));
    }
    if truthy(&proxy["allowed-ips"]) && !text(proxy, "allowed-ips").is_empty() {
        peer.push(format!("allowed-ips = \"{}\"", text(proxy, "allowed-ips")));
    }
    peer.push(format!(
        "endpoint = {}:{}",
        text(proxy, "server"),
        text(proxy, "port")
    ));
    let keepalive = if truthy(&proxy["persistent-keepalive"]) {
        "persistent-keepalive"
    } else {
        "keepalive"
    };
    if present(proxy, keepalive) {
        peer.push(format!("keepalive = {}", text(proxy, keepalive)));
    }
    if present(proxy, "reserved") {
        let value = if let Some(values) = proxy["reserved"].as_array() {
            values
                .iter()
                .map(|value| {
                    if value.is_null() {
                        String::new()
                    } else {
                        codec::js_string(value)
                    }
                })
                .collect::<Vec<_>>()
                .join("/")
        } else {
            text(proxy, "reserved")
        };
        peer.push(format!("client-id = {value}"));
    }
    let psk = if present(proxy, "preshared-key") {
        "preshared-key"
    } else {
        "pre-shared-key"
    };
    if present(proxy, psk) {
        peer.push(format!("preshared-key = {}", text(proxy, psk)));
    }
    out.push_str(&format!("\npeer = ({})", peer.join(", ")));
    Ok(out)
}

fn single(proxy: &mut Value, include: bool, surfboard: bool) -> Result<String, Failure> {
    if proxy["network"] == "ws" && truthy(&proxy["ws-opts"]["v2ray-http-upgrade"]) {
        return Err(Failure::Unsupported);
    }
    let name = proxy["name"]
        .as_str()
        .ok_or(Failure::Invalid)?
        .replace(['=', ','], "");
    proxy["name"] = json!(name);
    if !surfboard && truthy(&proxy["ports"]) {
        proxy["ports"] = json!(text(proxy, "ports"));
    }
    let kind = text(proxy, "type");
    if kind == "wireguard" && include && !surfboard {
        return wireguard(proxy);
    }
    let protocol = match kind.as_str() {
        "http" if truthy(&proxy["tls"]) => "https",
        "socks5" if truthy(&proxy["tls"]) => "socks5-tls",
        "tuic"
            if surfboard
                || !truthy(&proxy["token"])
                || proxy["token"].as_array().is_some_and(Vec::is_empty)
                || proxy["token"]["length"] == 0 =>
        {
            "tuic-v5"
        }
        "wireguard-surge" => "wireguard",
        "trusttunnel" if !surfboard => "trust-tunnel",
        "masque-surge" if !surfboard => "masque",
        value => value,
    };
    let mut out = if kind == "direct" && !surfboard {
        format!("{name}=direct")
    } else if kind == "wireguard-surge" {
        format!("{name}=wireguard")
    } else {
        format!(
            "{name}={protocol},{},{}",
            text(proxy, "server"),
            text(proxy, "port")
        )
    };
    match kind.as_str() {
        "ss" => {
            if !surfboard && !truthy(&proxy["cipher"]) {
                proxy["cipher"] = json!("none");
            }
            let cipher = text(proxy, "cipher");
            if ![
                "aes-128-gcm",
                "aes-192-gcm",
                "aes-256-gcm",
                "chacha20-ietf-poly1305",
                "xchacha20-ietf-poly1305",
                "rc4",
                "rc4-md5",
                "aes-128-cfb",
                "aes-192-cfb",
                "aes-256-cfb",
                "aes-128-ctr",
                "aes-192-ctr",
                "aes-256-ctr",
                "bf-cfb",
                "camellia-128-cfb",
                "camellia-192-cfb",
                "camellia-256-cfb",
                "cast5-cfb",
                "des-cfb",
                "idea-cfb",
                "rc2-cfb",
                "seed-cfb",
                "salsa20",
                "chacha20",
                "chacha20-ietf",
                "none",
                "2022-blake3-aes-128-gcm",
                "2022-blake3-aes-256-gcm",
            ]
            .contains(&cipher.as_str())
                || (surfboard
                    && [
                        "none",
                        "cast5-cfb",
                        "des-cfb",
                        "idea-cfb",
                        "rc2-cfb",
                        "seed-cfb",
                    ]
                    .contains(&cipher.as_str()))
            {
                return Err(Failure::Unsupported);
            }
            out.push_str(&format!(",encrypt-method={cipher}"));
            append(&mut out, proxy, "password", ",password=\"", "\"");
        }
        "trojan" | "anytls" | "hysteria2" => {
            if kind == "trojan" && surfboard {
                param(&mut out, proxy, "password", "password");
            } else {
                append(&mut out, proxy, "password", ",password=\"", "\"");
            }
            if kind == "anytls"
                && truthy(&proxy["network"])
                && (proxy["network"] != "tcp" || truthy(&proxy["reality-opts"]))
            {
                return Err(Failure::Unsupported);
            }
        }
        "vmess" => {
            param(&mut out, proxy, "uuid", "username");
            if !surfboard {
                let cipher = vmess_security(&proxy["cipher"]);
                match cipher.as_str().unwrap() {
                    "aes-128-gcm" => out.push_str(",encrypt-method=aes-128-gcm"),
                    "chacha20-poly1305" => out.push_str(",encrypt-method=chacha20-ietf-poly1305"),
                    _ => {}
                }
            }
        }
        "http" | "socks5" | "h2-connect" | "trusttunnel" | "ssh" | "masque-surge" => {
            if surfboard && !["http", "socks5"].contains(&kind.as_str()) {
                return Err(Failure::Unsupported);
            }
            for key in ["username", "password"] {
                if surfboard {
                    append(&mut out, proxy, key, ",", "");
                } else {
                    append(&mut out, proxy, key, &format!(",{key}=\""), "\"");
                }
            }
            if !surfboard && ["http", "h2-connect", "trusttunnel"].contains(&kind.as_str()) {
                root_headers(&mut out, proxy);
            }
            if !surfboard && ["h2-connect", "trusttunnel"].contains(&kind.as_str()) {
                param(&mut out, proxy, "max-streams", "max-streams");
            }
            if kind == "trusttunnel" && proxy["network"] == "h3" {
                out.push_str(",h3=true");
            }
            if kind == "ssh" {
                let key = if present(proxy, "keystore-private-key") {
                    "keystore-private-key"
                } else {
                    "private-key"
                };
                quoted(&mut out, proxy, key, "private-key");
                param(&mut out, proxy, "idle-timeout", "idle-timeout");
                append(
                    &mut out,
                    proxy,
                    "server-fingerprint",
                    ",server-fingerprint=\"",
                    "\"",
                );
            }
        }
        "snell" => {
            let version = proxy
                .get("version")
                .map(codec::to_number)
                .unwrap_or(f64::NAN);
            if surfboard
                && present(proxy, "version")
                && ![1.0, 2.0, 3.0, 4.0, 5.0].contains(&version)
            {
                return Err(Failure::Unsupported);
            }
            if !surfboard
                && version == 6.0
                && ["obfs-opts.mode", "obfs-opts.host", "obfs-opts.path"]
                    .iter()
                    .any(|path| present(proxy, path))
            {
                return Ok(String::new());
            }
            param(&mut out, proxy, "version", "version");
            append(&mut out, proxy, "psk", ",psk=\"", "\"");
            if !surfboard && version == 6.0 {
                param(&mut out, proxy, "mode", "mode");
            }
        }
        "tuic" => {
            if surfboard
                && proxy.get("token").is_some_and(|value| {
                    value.as_str().is_some_and(|s| !codec::units(s).is_empty())
                        || value.as_array().is_some_and(|a| !a.is_empty())
                        || truthy(&value["length"])
                })
            {
                return Err(Failure::Unsupported);
            }
            param(&mut out, proxy, "uuid", "uuid");
            append(&mut out, proxy, "password", ",password=\"", "\"");
            if !surfboard {
                param(&mut out, proxy, "token", "token");
            } else if nonblank(proxy, "alpn") {
                append(&mut out, proxy, "alpn", ",alpn=\"", "\"");
            }
        }
        "wireguard-surge" => {
            param(&mut out, proxy, "section-name", "section-name");
        }
        "direct" if !surfboard => {}
        _ => return Err(Failure::Unsupported),
    }
    if ["tuic", "hysteria2", "masque-surge"].contains(&kind.as_str()) {
        hopping(&mut out, proxy);
    }
    if kind == "hysteria2" {
        let obfs = text(proxy, "obfs");
        let password = truthy(&proxy["obfs-password"]);
        if surfboard
            && ((truthy(&proxy["obfs"]) && obfs != "salamander")
                || (password && obfs != "salamander"))
        {
            return Err(Failure::Unsupported);
        }
        let field = match obfs.as_str() {
            "salamander" => "salamander-password",
            "gecko" if !surfboard => "gecko-password",
            _ => "",
        };
        if password && field.is_empty() {
            return Err(Failure::Unsupported);
        }
        if password {
            append(
                &mut out,
                proxy,
                "obfs-password",
                &format!(",{field}=\""),
                "\"",
            );
        }
    }
    if !surfboard {
        if kind != "wireguard-surge" {
            ip(&mut out, proxy);
        }
        param(&mut out, proxy, "no-error-alert", "no-error-alert");
        if kind == "wireguard-surge" {
            ip(&mut out, proxy);
        }
    }
    if kind == "ss" && present(proxy, "plugin") {
        if proxy["plugin"] == "obfs" {
            if proxy["plugin-opts"].is_null() {
                return Err(Failure::Invalid);
            }
            out.push_str(&format!(",obfs={}", text(&proxy["plugin-opts"], "mode")));
            param(&mut out, proxy, "plugin-opts.host", "obfs-host");
            param(&mut out, proxy, "plugin-opts.path", "obfs-uri");
        } else if surfboard || proxy["plugin"] != "shadow-tls" {
            return Err(Failure::Unsupported);
        }
    }
    if kind == "snell" {
        for (path, label) in [
            ("obfs-opts.mode", "obfs"),
            ("obfs-opts.host", "obfs-host"),
            ("obfs-opts.path", "obfs-uri"),
        ] {
            param(&mut out, proxy, path, label);
        }
    }
    if ["trojan", "vmess"].contains(&kind.as_str()) {
        transport(&mut out, proxy, kind == "vmess" && include, surfboard)?;
    }
    if kind == "vmess" {
        let aead = if present(proxy, "aead") {
            text(proxy, "aead")
        } else {
            (proxy["alterId"] == 0).to_string()
        };
        out.push_str(&format!(",vmess-aead={aead}"));
    }
    if ["trojan", "vmess"].contains(&kind.as_str()) {
        param(&mut out, proxy, "tls", "tls");
    }
    if [
        "trojan",
        "anytls",
        "tuic",
        "hysteria2",
        "h2-connect",
        "trusttunnel",
        "masque-surge",
    ]
    .contains(&kind.as_str())
    {
        tls(&mut out, proxy, true, surfboard);
    } else if ["vmess", "http", "socks5"].contains(&kind.as_str()) {
        tls(&mut out, proxy, truthy(&proxy["tls"]), surfboard);
    }
    let add_tfo = if surfboard {
        ["snell", "trojan", "anytls"].contains(&kind.as_str())
    } else {
        !["socks5", "h2-connect", "wireguard-surge"].contains(&kind.as_str())
    };
    if add_tfo {
        tfo(
            &mut out,
            proxy,
            !surfboard && ["tuic", "hysteria2", "masque-surge"].contains(&kind.as_str()),
        );
    }
    if surfboard {
        if kind == "hysteria2" {
            bandwidth(&mut out, proxy);
        }
        if kind != "wireguard-surge"
            && (kind != "snell"
                || proxy
                    .get("version")
                    .map(codec::to_number)
                    .unwrap_or(f64::NAN)
                    >= 3.0)
        {
            param(&mut out, proxy, "udp", "udp-relay");
        }
        if kind == "anytls" {
            param(&mut out, proxy, "reuse", "reuse");
        }
        if kind != "tuic" {
            param(&mut out, proxy, "block-quic", "block-quic");
        }
        return Ok(out);
    }
    if ["ss", "socks5", "h2-connect"].contains(&kind.as_str()) {
        param(&mut out, proxy, "udp", "udp-relay");
    }
    common(&mut out, proxy);
    if !["anytls", "ssh", "trusttunnel", "direct", "masque-surge"].contains(&kind.as_str()) {
        shadow(&mut out, proxy, kind == "ss")?;
    }
    param(&mut out, proxy, "block-quic", "block-quic");
    param(&mut out, proxy, "underlying-proxy", "underlying-proxy");
    if ["snell", "anytls", "trusttunnel"].contains(&kind.as_str()) {
        param(&mut out, proxy, "reuse", "reuse");
    }
    if kind == "hysteria2" {
        bandwidth(&mut out, proxy);
    }
    if ["tuic", "hysteria2", "masque-surge"].contains(&kind.as_str()) {
        param(&mut out, proxy, "ecn", "ecn");
    }
    Ok(out)
}

fn bandwidth(out: &mut String, proxy: &Value) {
    if present(proxy, "down") {
        let raw = text(proxy, "down");
        let number: String = raw
            .chars()
            .skip_while(|c| !c.is_ascii_digit())
            .take_while(char::is_ascii_digit)
            .collect();
        out.push_str(&format!(
            ",download-bandwidth={}",
            if number.is_empty() { "0" } else { &number }
        ));
    }
}

fn fallback(opts: &Value, proxy: &Value, key: &str, private: &str, default: Value) -> Value {
    opts.get(key)
        .filter(|v| truthy(v))
        .or_else(|| proxy.get(private).filter(|v| truthy(v)))
        .cloned()
        .unwrap_or(default)
}

fn mihomo(proxy: &mut Value, opts: &mut Value) -> Result<String, Failure> {
    let list = crate::clash_output::produce(
        vec![proxy.clone()],
        "internal",
        &json!({}),
        crate::clash_output::Family::Meta,
    )
    .map_err(|_| Failure::Invalid)?;
    let Some(mut converted) = list.as_array().and_then(|list| list.first()).cloned() else {
        return Ok(String::new());
    };
    let port = fallback(opts, proxy, "localPort", "_localPort", json!(65535));
    let ipv6 = !["ipv4", "v4-only"].contains(&text(proxy, "ip-version").as_str());
    let dns = json!({
        "enable":true,"ipv6":ipv6,
        "default-nameserver":fallback(opts,proxy,"defaultNameserver","_defaultNameserver",json!(["180.76.76.76","52.80.52.52","119.28.28.28","223.6.6.6"])),
        "nameserver":fallback(opts,proxy,"nameserver","_nameserver",json!(["https://doh.pub/dns-query","https://dns.alidns.com/dns-query","https://doh-pure.onedns.net/dns-query"]))
    });
    let exec = fallback(opts, proxy, "exec", "_exec", json!("/usr/local/bin/mihomo"));
    let config_override = fallback(opts, proxy, "config", "_config", Value::Null);
    let result;
    if truthy(&fallback(opts, proxy, "merge", "_merge", json!(false))) {
        if !opts.is_object() {
            return Err(Failure::Invalid);
        }
        let mut socks = json!({"name":proxy["name"],"type":"socks5","server":"127.0.0.1","port":port,"udp":true});
        result = single(
            &mut socks,
            truthy(&opts["include-unsupported-proxy"]),
            false,
        )?;
        if !truthy(&opts["_merged"]) {
            opts["_merged"] = json!({
                "name":fallback(opts,proxy,"mergeName","_mergeName",json!("mihomo merged")),"exec":exec,
                "config":{"ipv6":ipv6,"mode":"global","dns":dns,"proxies":[],"proxy-groups":[{"name":"GLOBAL","type":"fallback","proxies":[]}],"listeners":[]}
            });
        }
        let name = codec::js_string(&port);
        let listeners = opts
            .get_mut("_merged")
            .and_then(|v| v.get_mut("config"))
            .and_then(|v| v.get_mut("listeners"))
            .and_then(Value::as_array_mut)
            .ok_or(Failure::Invalid)?;
        listeners.push(json!({"name":format!("socks5-{name}"),"type":"socks","port":port,"listen":"127.0.0.1","udp":true,"proxy":name}));
        opts.get_mut("_merged")
            .and_then(|v| v.get_mut("config"))
            .and_then(|v| v.get_mut("proxy-groups"))
            .and_then(Value::as_array_mut)
            .and_then(|groups| groups.first_mut())
            .and_then(|group| group.get_mut("proxies"))
            .and_then(Value::as_array_mut)
            .ok_or(Failure::Invalid)?
            .push(json!(name));
        converted["name"] = json!(name);
        opts.get_mut("_merged")
            .and_then(|v| v.get_mut("config"))
            .and_then(|v| v.get_mut("proxies"))
            .and_then(Value::as_array_mut)
            .ok_or(Failure::Invalid)?
            .push(converted);
        if truthy(&config_override) {
            let mut fields =
                crate::body::Body::Json(opts["_merged"]["configOverride"].clone()).fields();
            fields.extend(crate::body::Body::Json(config_override).fields());
            opts["_merged"]["configOverride"] = Value::Object(fields);
        }
    } else {
        converted["name"] = json!("proxy");
        let mut config = crate::body::Body::Json(json!({
            "mixed-port":port,"ipv6":ipv6,"mode":"global","dns":dns,"proxies":[converted],
            "proxy-groups":[{"name":"GLOBAL","type":"select","proxies":["proxy"]}]
        }))
        .fields();
        config.extend(crate::body::Body::Json(config_override).fields());
        let addresses = if proxy["server"].as_str().is_some_and(is_ip) {
            json!([proxy["server"]])
        } else {
            json!([])
        };
        result = external(&json!({"name":proxy["name"],"type":"external","udp":true,"exec":exec,"local-port":port,"args":["-config",STANDARD.encode(codec::stringify(&legacy_object_order(Value::Object(config))))],"addresses":addresses})).map_err(|_| Failure::Invalid)?;
    }
    if opts.is_object() {
        opts["localPort"] = json!(codec::to_number(&port) - 1.0);
    }
    Ok(result)
}

pub fn produce(
    mut proxies: Vec<Value>,
    target: &str,
    kind: &str,
    options: &Value,
) -> Result<Value, Error> {
    let mut opts = if options.is_array() {
        Value::Object(crate::body::Body::Json(options.clone()).fields())
    } else {
        options.clone()
    };
    let mut lines = Vec::new();
    for proxy in &mut proxies {
        let result = if target == "SurgeMac" {
            if proxy["type"] == "external" {
                external(proxy).map_err(|_| Failure::Invalid)
            } else if truthy(&opts["mihomoExternal"]) || truthy(&proxy["_mihomoExternal"]) {
                mihomo(proxy, &mut opts)
            } else {
                match single(proxy, truthy(&opts["include-unsupported-proxy"]), false) {
                    Err(Failure::Unsupported) if truthy(&opts["useMihomoExternal"]) => {
                        mihomo(proxy, &mut opts)
                    }
                    result => result,
                }
            }
        } else {
            single(
                proxy,
                truthy(&opts["include-unsupported-proxy"]),
                target.eq_ignore_ascii_case("surfboard"),
            )
        };
        if let Ok(line) = result
            && !line.is_empty()
        {
            lines.push(line);
        }
    }
    let output = finish(lines, kind, &opts)?;
    if target.starts_with("Surge")
        && !proxies.is_empty()
        && proxies.iter().all(|proxy| proxy["type"] == "wireguard")
    {
        let first = &proxies[0];
        let desc = first
            .get("_desc")
            .filter(|v| !v.is_null())
            .map(codec::js_string)
            .unwrap_or_default();
        let category = first
            .get("_category")
            .filter(|v| !v.is_null())
            .map(codec::js_string)
            .unwrap_or_default();
        return Ok(json!(format!(
            "#!name={}\n#!desc={desc}\n#!category={category}\n{}",
            text(first, "_subName"),
            codec::js_string(&output)
        )));
    }
    Ok(output)
}
