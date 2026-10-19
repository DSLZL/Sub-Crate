use crate::conversion::{field_text as text, is_ip, ss_over_tls, vmess_security};
use crate::json as codec;
use crate::state::truthy;
use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::{Value, json};

fn field<'a>(proxy: &'a Value, path: &str) -> Option<&'a Value> {
    proxy
        .get(path)
        .or_else(|| path.split('.').try_fold(proxy, |value, key| value.get(key)))
}

fn present(proxy: &Value, path: &str) -> bool {
    field(proxy, path).is_some_and(|value| !value.is_null())
}

fn append(out: &mut String, proxy: &Value, path: &str, prefix: &str, suffix: &str) {
    if let Some(value) = field(proxy, path).filter(|value| !value.is_null()) {
        out.push_str(prefix);
        out.push_str(&codec::js_string(value));
        out.push_str(suffix);
    }
}

fn param(out: &mut String, proxy: &Value, path: &str, label: &str) {
    append(out, proxy, path, &format!(",{label}="), "");
}

fn first(value: Option<&Value>) -> String {
    value
        .and_then(|value| {
            value
                .as_array()
                .map_or(Some(value), |values| values.first())
        })
        .map_or_else(|| "undefined".to_owned(), codec::js_string)
}

fn alpn(value: Option<&Value>) -> String {
    match value {
        Some(Value::Array(values)) => values.iter().map(codec::js_string).collect::<Vec<_>>(),
        Some(value) if truthy(value) => codec::js_string(value)
            .split(',')
            .map(str::to_owned)
            .collect(),
        _ => Vec::new(),
    }
    .iter()
    .map(|value| codec::trim(value))
    .filter(|value| !value.is_empty())
    .collect::<Vec<_>>()
    .join(",")
}

fn qx_tls(out: &mut String, proxy: &Value, host: bool) -> Result<(), String> {
    if !truthy(&proxy["tls"]) {
        return Ok(());
    }
    param(out, proxy, "tls-pubkey-sha256", "tls-pubkey-sha256");
    if present(proxy, "tls-alpn") {
        param(out, proxy, "tls-alpn", "tls-alpn");
    } else if let Some(value) = proxy.get("alpn").filter(|value| !value.is_null()) {
        let values = match value {
            Value::Array(values) => values.iter().map(codec::js_string).collect::<Vec<_>>(),
            value => codec::js_string(value)
                .split(',')
                .map(str::to_owned)
                .collect(),
        };
        let mut encoded = String::new();
        for value in values {
            let value = codec::trim(&value);
            if value.is_empty() {
                continue;
            }
            let value = String::from_utf16_lossy(&codec::units(value));
            if value.len() > 255 {
                return Err("QX ALPN protocol exceeds 255 bytes".to_owned());
            }
            encoded.push_str(&format!("{:02x}", value.len()));
            for byte in value.bytes() {
                encoded.push_str(&format!("{byte:02x}"));
            }
        }
        if !encoded.is_empty() {
            out.push_str(&format!(",tls-alpn={encoded}"));
        }
    }
    for (path, label) in [
        ("tls-no-session-ticket", "tls-no-session-ticket"),
        ("tls-no-session-reuse", "tls-no-session-reuse"),
        ("tls-fingerprint", "tls-cert-sha256"),
    ] {
        param(out, proxy, path, label);
    }
    let verification = if present(proxy, "name-cert-verify") {
        Some(codec::js_string(&proxy["name-cert-verify"]))
    } else if present(proxy, "skip-cert-verify") {
        Some((!truthy(&proxy["skip-cert-verify"])).to_string())
    } else {
        None
    };
    if let Some(value) = verification {
        out.push_str(&format!(",tls-verification={value}"));
    }
    if host {
        param(out, proxy, "sni", "tls-host");
    }
    Ok(())
}

fn qx_http(proxy: &Value) -> &str {
    match proxy["_qx_obfs_http"].as_str() {
        Some(token @ ("http" | "vmess-http" | "vemss-http" | "shadowsocks-http")) => token,
        _ => "http",
    }
}

fn qx_transport(out: &mut String, proxy: &Value, http: bool, trojan: bool) -> Result<(), String> {
    let tls = truthy(&proxy["tls"]);
    if present(proxy, "network") {
        let network = text(proxy, "network");
        match network.as_str() {
            "ws" => out.push_str(if tls { ",obfs=wss" } else { ",obfs=ws" }),
            "http" if http => out.push_str(&format!(",obfs={}", qx_http(proxy))),
            "tcp" if !trojan && tls => out.push_str(",obfs=over-tls"),
            "tcp" => {}
            _ => return Err(format!("network {network} is unsupported")),
        }
        if network != "tcp" || !trojan {
            for (path, label) in [("path", "obfs-uri"), ("headers.Host", "obfs-host")] {
                let path = format!("{network}-opts.{path}");
                if present(proxy, &path) {
                    let value = if trojan {
                        field(proxy, &path).map(codec::js_string).unwrap()
                    } else {
                        first(field(proxy, &path))
                    };
                    out.push_str(&format!(",{label}={value}"));
                }
            }
        }
    } else if tls && !trojan {
        out.push_str(",obfs=over-tls");
    }
    if trojan && proxy["network"] != "ws" && tls {
        out.push_str(",over-tls=true");
    }
    Ok(())
}

fn qx(proxy: &Value) -> Result<String, String> {
    if proxy["network"] == "ws" && truthy(&proxy["ws-opts"]["v2ray-http-upgrade"]) {
        return Err("QX HTTP upgrade is unsupported".to_owned());
    }
    let kind = text(proxy, "type");
    let endpoint = format!("{}:{}", text(proxy, "server"), text(proxy, "port"));
    let mut out;
    let mut local = proxy.clone();
    match kind.as_str() {
        "ss" => {
            if !truthy(&local["cipher"]) {
                local["cipher"] = json!("none");
            }
            let cipher = text(&local, "cipher");
            if ![
                "none",
                "rc4-md5",
                "rc4-md5-6",
                "aes-128-cfb",
                "aes-192-cfb",
                "aes-256-cfb",
                "aes-128-ctr",
                "aes-192-ctr",
                "aes-256-ctr",
                "bf-cfb",
                "cast5-cfb",
                "des-cfb",
                "rc2-cfb",
                "salsa20",
                "chacha20",
                "chacha20-ietf",
                "aes-128-gcm",
                "aes-192-gcm",
                "aes-256-gcm",
                "chacha20-ietf-poly1305",
                "xchacha20-ietf-poly1305",
                "2022-blake3-aes-128-gcm",
                "2022-blake3-aes-256-gcm",
            ]
            .contains(&cipher.as_str())
            {
                return Err(format!("cipher {cipher} is not supported"));
            }
            out = format!(
                "shadowsocks={endpoint},method={cipher},password={}",
                text(proxy, "password")
            );
            if ss_over_tls(proxy) {
                out.push_str(",obfs=over-tls");
                if present(proxy, "sni") {
                    param(&mut out, proxy, "sni", "obfs-host");
                } else {
                    param(&mut out, proxy, "servername", "obfs-host");
                }
            } else if present(proxy, "plugin") {
                let options = &proxy["plugin-opts"];
                if options.is_null() {
                    return Err("plugin options are missing".to_owned());
                }
                if proxy["plugin"] == "obfs" {
                    let mode = if options["mode"] == "http" {
                        qx_http(proxy).to_owned()
                    } else {
                        text(options, "mode")
                    };
                    out.push_str(&format!(",obfs={mode}"));
                } else if proxy["plugin"] == "v2ray-plugin" && options["mode"] == "websocket" {
                    out.push_str(if truthy(&options["tls"]) {
                        ",obfs=wss"
                    } else {
                        ",obfs=ws"
                    });
                } else {
                    return Err("plugin is not supported".to_owned());
                }
                param(&mut out, proxy, "plugin-opts.host", "obfs-host");
                param(&mut out, proxy, "plugin-opts.path", "obfs-uri");
            }
            qx_tls(&mut out, proxy, !ss_over_tls(proxy))?;
        }
        "ssr" => {
            out = format!(
                "shadowsocks={endpoint},method={},password={},ssr-protocol={}",
                text(proxy, "cipher"),
                text(proxy, "password"),
                text(proxy, "protocol")
            );
            for (path, label) in [
                ("protocol-param", "ssr-protocol-param"),
                ("obfs", "obfs"),
                ("obfs-param", "obfs-host"),
            ] {
                param(&mut out, proxy, path, label);
            }
        }
        "trojan" => {
            out = format!("trojan={endpoint},password={}", text(proxy, "password"));
            qx_transport(&mut out, proxy, false, true)?;
            qx_tls(&mut out, proxy, true)?;
        }
        "vmess" | "vless" => {
            if kind == "vless" && truthy(&proxy["encryption"]) && proxy["encryption"] != "none" {
                return Err("VLESS encryption is not supported".to_owned());
            }
            let cipher = if kind == "vless" {
                "none".to_owned()
            } else {
                if vmess_security(&proxy["cipher"]) == "none" {
                    "none".to_owned()
                } else {
                    "chacha20-poly1305".to_owned()
                }
            };
            out = format!(
                "{kind}={endpoint},method={cipher},password={}",
                text(proxy, "uuid")
            );
            qx_transport(&mut out, proxy, true, false)?;
            qx_tls(&mut out, proxy, true)?;
            if kind == "vmess" {
                let aead = if present(proxy, "aead") {
                    text(proxy, "aead")
                } else {
                    (proxy["alterId"] == 0).to_string()
                };
                out.push_str(&format!(",aead={aead}"));
            } else {
                param(&mut out, proxy, "flow", "vless-flow");
            }
        }
        "anytls" => {
            if let Some(network) = proxy.get("network").filter(|value| !value.is_null()) {
                let network = network
                    .as_str()
                    .ok_or_else(|| "AnyTLS network must be text".to_owned())?;
                let normalized = codec::trim(network).to_lowercase();
                if !normalized.is_empty() && normalized != "tcp" {
                    return Err("AnyTLS transport is unsupported".to_owned());
                }
            }
            out = format!(
                "anytls={endpoint},password={},over-tls=true",
                text(proxy, "password")
            );
            local["tls"] = json!(true);
            qx_tls(&mut out, &local, true)?;
        }
        "http" | "socks5" => {
            out = format!("{kind}={endpoint}");
            param(&mut out, proxy, "username", "username");
            param(&mut out, proxy, "password", "password");
            param(&mut out, proxy, "tls", "over-tls");
            qx_tls(&mut out, proxy, true)?;
        }
        _ => return Err(format!("Platform QX does not support proxy type: {kind}")),
    }
    param(&mut out, proxy, "tfo", "fast-open");
    param(&mut out, proxy, "udp", "udp-relay");
    if kind == "ss" {
        if truthy(&proxy["_ssr_python_uot"]) {
            out.push_str(",udp-over-tcp=true");
        } else if truthy(&proxy["udp-over-tcp"]) {
            if !truthy(&proxy["udp-over-tcp-version"]) || proxy["udp-over-tcp-version"] == 1 {
                out.push_str(",udp-over-tcp=sp.v1");
            } else if proxy["udp-over-tcp-version"] == 2 {
                out.push_str(",udp-over-tcp=sp.v2");
            }
        }
    }
    param(&mut out, proxy, "test-url", "server_check_url");
    out.push_str(&format!(",tag={}", text(proxy, "name")));
    if truthy(&proxy["flow"]) && proxy["flow"] != "xtls-rprx-vision" {
        return Err("QX flow is unsupported".to_owned());
    }
    for (path, label) in [
        ("reality-opts.public-key", "reality-base64-pubkey"),
        ("reality-opts.short-id", "reality-hex-shortid"),
    ] {
        if field(proxy, path).is_some_and(truthy) {
            param(&mut out, proxy, path, label);
        }
    }
    Ok(out)
}

fn loon_profile(out: &mut String, proxy: &Value) {
    let raw = proxy
        .get("_loon_tls_profile")
        .filter(|value| truthy(value))
        .map(codec::js_string)
        .unwrap_or_default();
    let raw = codec::trim(&raw);
    let fingerprint = proxy
        .get("client-fingerprint")
        .filter(|value| truthy(value))
        .map(codec::js_string)
        .unwrap_or_default();
    let fingerprint = codec::trim(&fingerprint);
    let profile = if [
        "global",
        "default",
        "safari-ios18",
        "safari-ios-26",
        "chrome",
        "chrome147",
    ]
    .contains(&raw)
    {
        raw
    } else if truthy(&proxy["reality-opts"]["support-x25519mlkem768"]) {
        if ["safari", "ios"].contains(&fingerprint) {
            "safari-ios-26"
        } else {
            "chrome147"
        }
    } else {
        match fingerprint {
            "chrome" => "chrome",
            "safari" | "ios" => "safari-ios18",
            _ => "",
        }
    };
    if !profile.is_empty() {
        out.push_str(&format!(",tls-profile={profile}"));
    }
}

fn loon_alpn(out: &mut String, proxy: &Value) {
    let value = alpn(proxy.get("alpn"));
    if !value.is_empty() {
        out.push_str(&format!(",alpn=\"{value}\""));
    }
}

fn loon_shadow(out: &mut String, proxy: &Value) -> Result<(), String> {
    if proxy["plugin"] != "shadow-tls" || !truthy(&proxy["plugin-opts"]) {
        return Ok(());
    }
    let options = &proxy["plugin-opts"];
    if !truthy(&options["password"]) {
        return Ok(());
    }
    param(out, options, "password", "shadow-tls-password");
    if truthy(&options["host"]) {
        param(out, options, "host", "shadow-tls-sni");
    }
    if truthy(&options["version"]) {
        if codec::to_number(&options["version"]) < 2.0 {
            return Err("ShadowTLS version is unsupported".to_owned());
        }
        param(out, options, "version", "shadow-tls-version");
    }
    loon_profile(out, proxy);
    let value = alpn(
        options
            .get("alpn")
            .filter(|value| !value.is_null())
            .or_else(|| proxy.get("alpn")),
    );
    if !value.is_empty() {
        out.push_str(&format!(",alpn=\"{value}\""));
    }
    param(out, proxy, "udp-port", "udp-port");
    Ok(())
}

fn loon_block(out: &mut String, proxy: &Value) {
    match proxy["block-quic"].as_str() {
        Some("on") => out.push_str(",block-quic=true"),
        Some("off") => out.push_str(",block-quic=false"),
        _ => {}
    }
}

fn loon_ip(out: &mut String, proxy: &Value) {
    if present(proxy, "ip-version") {
        let value = text(proxy, "ip-version");
        let value = match value.as_str() {
            "ipv4" => "v4-only",
            "ipv6" => "v6-only",
            "ipv4-prefer" => "prefer-v4",
            "ipv6-prefer" => "prefer-v6",
            _ => &value,
        };
        out.push_str(&format!(",ip-mode={value}"));
    }
}

fn loon_certificate(out: &mut String, proxy: &Value) {
    if truthy(&proxy["reality-opts"]) {
        param(out, proxy, "sni", "sni");
        append(
            out,
            proxy,
            "reality-opts.public-key",
            ",public-key=\"",
            "\"",
        );
        param(out, proxy, "reality-opts.short-id", "short-id");
    } else {
        param(out, proxy, "sni", "tls-name");
        param(out, proxy, "tls-fingerprint", "tls-cert-sha256");
        param(out, proxy, "tls-pubkey-sha256", "tls-pubkey-sha256");
    }
}

fn loon_transport(out: &mut String, proxy: &Value, trojan: bool) -> Result<(), String> {
    let network = text(proxy, "network");
    if present(proxy, "network") && network != "tcp" {
        if network != "ws" && (trojan || network != "http") {
            return Err("Loon transport is unsupported".to_owned());
        }
        out.push_str(&format!(",transport={network}"));
        for (path, label) in [("path", "path"), ("headers.Host", "host")] {
            let path = format!("{network}-opts.{path}");
            if present(proxy, &path) {
                let value = if network == "http" {
                    first(field(proxy, &path))
                } else {
                    field(proxy, &path).map(codec::js_string).unwrap()
                };
                out.push_str(&format!(",{label}={value}"));
            }
        }
    } else if !trojan {
        out.push_str(",transport=tcp");
    }
    Ok(())
}

fn loon_wireguard(proxy: &Value) -> Result<String, String> {
    let mut proxy = proxy.clone();
    if let Some(peer) = proxy["peers"]
        .as_array()
        .and_then(|peers| peers.first())
        .cloned()
    {
        if peer.is_null() {
            return Err("WireGuard peer is null".to_owned());
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
    let mut out = format!("{}=wireguard", text(&proxy, "name"));
    param(&mut out, &proxy, "ip", "interface-ip");
    param(&mut out, &proxy, "ipv6", "interface-ipv6");
    append(&mut out, &proxy, "private-key", ",private-key=\"", "\"");
    param(&mut out, &proxy, "mtu", "mtu");
    if let Some(values) = proxy["dns"].as_array() {
        let v6 = values
            .iter()
            .find(|v| v.as_str().is_some_and(|v| v.contains(':') && is_ip(v)))
            .cloned();
        let dns = values
            .iter()
            .find(|v| v.as_str().is_some_and(|v| !v.contains(':') && is_ip(v)))
            .or_else(|| values.iter().find(|v| !v.as_str().is_some_and(is_ip)))
            .cloned();
        for (key, value) in [("dns", dns), ("dnsv6", v6)] {
            if let Some(value) = value {
                proxy[key] = value;
            } else {
                proxy.as_object_mut().unwrap().shift_remove(key);
            }
        }
    }
    for (path, label) in [
        ("dns", "dns"),
        ("dnsv6", "dnsv6"),
        ("persistent-keepalive", "keepalive"),
        ("keepalive", "keepalive"),
    ] {
        param(&mut out, &proxy, path, label);
    }
    let allowed = proxy
        .get("allowed-ips")
        .filter(|v| !v.is_null())
        .map(codec::js_string)
        .unwrap_or_else(|| "0.0.0.0/0,::/0".to_owned());
    let reserved = proxy
        .get("reserved")
        .filter(|value| !value.is_null())
        .map(|value| {
            let text = codec::js_string(value);
            if (value.is_array() && !text.is_empty()) || (!value.is_array() && truthy(value)) {
                format!(",reserved=[{text}]")
            } else {
                text
            }
        })
        .unwrap_or_default();
    let psk = proxy
        .get("preshared-key")
        .filter(|v| !v.is_null())
        .or_else(|| proxy.get("pre-shared-key").filter(|value| !value.is_null()));
    let psk = psk
        .map(|v| {
            if truthy(v) {
                format!(",preshared-key=\"{}\"", codec::js_string(v))
            } else {
                codec::js_string(v)
            }
        })
        .unwrap_or_default();
    out.push_str(&format!(
        ",peers=[{{public-key=\"{}\",allowed-ips=\"{allowed}\",endpoint={}:{}{reserved}{psk}}}]",
        text(&proxy, "public-key"),
        text(&proxy, "server"),
        text(&proxy, "port")
    ));
    loon_ip(&mut out, &proxy);
    loon_block(&mut out, &proxy);
    Ok(out)
}

fn loon(proxy: &Value) -> Result<String, String> {
    if proxy["network"] == "ws" && truthy(&proxy["ws-opts"]["v2ray-http-upgrade"]) {
        return Err("Loon HTTP upgrade is unsupported".to_owned());
    }
    let kind = text(proxy, "type");
    let mut out;
    match kind.as_str() {
        "ss" | "ssr" => {
            let cipher = text(proxy, "cipher");
            if kind == "ss"
                && ![
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
                    "salsa20",
                    "chacha20",
                    "chacha20-ietf",
                    "aes-128-gcm",
                    "aes-192-gcm",
                    "aes-256-gcm",
                    "chacha20-ietf-poly1305",
                    "xchacha20-ietf-poly1305",
                    "2022-blake3-aes-128-gcm",
                    "2022-blake3-aes-256-gcm",
                ]
                .contains(&cipher.as_str())
            {
                return Err("Loon cipher is unsupported".to_owned());
            }
            let protocol = if kind == "ss" {
                "shadowsocks"
            } else {
                "shadowsocksr"
            };
            out = format!(
                "{}={protocol},{},{},{cipher},\"{}\"",
                text(proxy, "name"),
                text(proxy, "server"),
                text(proxy, "port"),
                text(proxy, "password")
            );
            if kind == "ss" && present(proxy, "plugin") {
                if proxy["plugin"] == "obfs" {
                    if proxy["plugin-opts"].is_null() {
                        return Err("plugin options are missing".to_owned());
                    }
                    out.push_str(&format!(
                        ",obfs-name={}",
                        text(&proxy["plugin-opts"], "mode")
                    ));
                    param(&mut out, proxy, "plugin-opts.host", "obfs-host");
                    param(&mut out, proxy, "plugin-opts.path", "obfs-uri");
                } else if proxy["plugin"] != "shadow-tls" {
                    return Err("Loon plugin is unsupported".to_owned());
                }
            }
            if kind == "ssr" {
                out.push_str(&format!(",protocol={}", text(proxy, "protocol")));
                for path in ["protocol-param", "obfs", "obfs-param"] {
                    param(&mut out, proxy, path, path);
                }
            }
            loon_shadow(&mut out, proxy)?;
            if kind == "ss"
                && truthy(&proxy["udp-over-tcp"])
                && proxy["udp-over-tcp-version"] == 2
                && proxy["plugin"] != "obfs"
            {
                out.push_str(",udp-over-tcp=true");
            }
        }
        "trojan" | "vmess" | "vless" | "anytls" => {
            let password = if kind == "vmess" || kind == "vless" {
                text(proxy, "uuid")
            } else {
                text(proxy, "password")
            };
            let mut security = String::new();
            if kind == "vmess" {
                let value = vmess_security(&proxy["cipher"]);
                security = format!(
                    "{},",
                    if value == "chacha20-poly1305" {
                        "chacha20-ietf-poly1305".to_owned()
                    } else if value == "zero" {
                        "auto".to_owned()
                    } else {
                        codec::js_string(&value)
                    }
                );
            }
            if kind == "vless" && truthy(&proxy["encryption"]) && proxy["encryption"] != "none" {
                return Err("VLESS encryption is unsupported".to_owned());
            }
            if kind == "vless" && proxy.get("flow").is_some() && proxy["flow"] != "xtls-rprx-vision"
            {
                return Err("VLESS flow is unsupported".to_owned());
            }
            out = format!(
                "{}={kind},{},{},{security}\"{password}\"",
                text(proxy, "name"),
                text(proxy, "server"),
                text(proxy, "port")
            );
            if kind == "anytls" {
                if truthy(&proxy["network"]) && proxy["network"] != "tcp" {
                    return Err("AnyTLS transport is unsupported".to_owned());
                }
                for key in ["idle-session-timeout", "max-stream-count"] {
                    if proxy[key].as_f64().is_some_and(|n| n.fract() == 0.0) {
                        param(&mut out, proxy, key, key);
                    }
                }
            } else {
                loon_transport(&mut out, proxy, kind == "trojan")?;
            }
            if kind == "vmess" || kind == "vless" {
                param(&mut out, proxy, "tls", "over-tls");
            }
            param(&mut out, proxy, "skip-cert-verify", "skip-cert-verify");
            if kind == "trojan"
                || kind == "anytls"
                || truthy(&proxy["tls"])
                || truthy(&proxy["reality-opts"])
                || (kind == "vless" && proxy.get("flow").is_some())
            {
                loon_profile(&mut out, proxy);
                loon_alpn(&mut out, proxy);
            }
            if kind == "vless" {
                param(&mut out, proxy, "flow", "flow");
            }
            loon_certificate(&mut out, proxy);
            if kind == "vmess" {
                let value = if present(proxy, "aead") {
                    if truthy(&proxy["aead"]) {
                        "0".to_owned()
                    } else {
                        "1".to_owned()
                    }
                } else {
                    text(proxy, "alterId")
                };
                out.push_str(&format!(",alterId={value}"));
            }
        }
        "http" | "socks5" => {
            let scheme = if kind == "http" && truthy(&proxy["tls"]) {
                "https"
            } else {
                &kind
            };
            out = format!(
                "{}={scheme},{},{}",
                text(proxy, "name"),
                text(proxy, "server"),
                text(proxy, "port")
            );
            append(&mut out, proxy, "username", ",", "");
            append(&mut out, proxy, "password", ",\"", "\"");
            if kind == "socks5" {
                param(&mut out, proxy, "tls", "over-tls");
            }
            param(&mut out, proxy, "sni", "sni");
            param(&mut out, proxy, "skip-cert-verify", "skip-cert-verify");
            if truthy(&proxy["tls"]) {
                loon_profile(&mut out, proxy);
                loon_alpn(&mut out, proxy);
            }
        }
        "wireguard" => {
            out = loon_wireguard(proxy)?;
        }
        "hysteria2" => {
            if truthy(&proxy["obfs-password"]) && proxy["obfs"] != "salamander" {
                return Err("only salamander obfs is supported".to_owned());
            }
            out = format!(
                "{}=Hysteria2,{},{}",
                text(proxy, "name"),
                text(proxy, "server"),
                text(proxy, "port")
            );
            append(&mut out, proxy, "password", ",\"", "\"");
            if present(proxy, "ports") && !codec::trim(&text(proxy, "ports")).is_empty() {
                append(&mut out, proxy, "ports", ",server-ports=\"", "\"");
            }
            if present(proxy, "hop-interval")
                && !codec::trim(&text(proxy, "hop-interval")).is_empty()
            {
                param(&mut out, proxy, "hop-interval", "hop-interval");
            }
            for (path, label) in [
                ("sni", "tls-name"),
                ("tls-fingerprint", "tls-cert-sha256"),
                ("tls-pubkey-sha256", "tls-pubkey-sha256"),
                ("skip-cert-verify", "skip-cert-verify"),
            ] {
                param(&mut out, proxy, path, label);
            }
            loon_profile(&mut out, proxy);
            loon_alpn(&mut out, proxy);
            if truthy(&proxy["obfs-password"]) {
                param(&mut out, proxy, "obfs-password", "salamander-password");
            }
        }
        _ => return Err(format!("Platform Loon does not support proxy type: {kind}")),
    }
    if kind != "wireguard" {
        param(
            &mut out,
            proxy,
            "tfo",
            if kind == "http" || kind == "socks5" {
                "tfo"
            } else {
                "fast-open"
            },
        );
        loon_block(&mut out, proxy);
        if kind != "http" && truthy(&proxy["udp"]) {
            out.push_str(",udp=true");
        }
        if kind == "hysteria2" {
            if present(proxy, "down") {
                let value = text(proxy, "down");
                let value: String = value
                    .chars()
                    .skip_while(|c| !c.is_ascii_digit())
                    .take_while(char::is_ascii_digit)
                    .collect();
                out.push_str(&format!(
                    ",download-bandwidth={}",
                    if value.is_empty() { "0" } else { &value }
                ));
            }
            param(&mut out, proxy, "ecn", "ecn");
        }
        loon_ip(&mut out, proxy);
    }
    if let Some(dns) = proxy["server-dns"].as_array().filter(|v| !v.is_empty()) {
        out.push_str(&format!(
            ",server-dns=\"{}\"",
            codec::js_string(&json!(dns))
        ));
    }
    Ok(out)
}

fn external(proxy: &Value) -> Result<String, String> {
    if !truthy(&proxy["exec"]) || !truthy(&proxy["local-port"]) {
        return Err("external: exec and local-port are required".to_owned());
    }
    let mut out = format!(
        "{}=external,exec=\"{}\",local-port={}",
        text(proxy, "name"),
        text(proxy, "exec"),
        text(proxy, "local-port")
    );
    for (key, quoted) in [("args", true), ("addresses", false)] {
        if let Some(values) = proxy[key].as_array() {
            for value in values {
                let value = codec::js_string(value);
                if quoted {
                    out.push_str(&format!(",{key}=\"{value}\""));
                } else {
                    out.push_str(&format!(",{key}={value}"));
                }
            }
        }
    }
    param(&mut out, proxy, "no-error-alert", "no-error-alert");
    param(&mut out, proxy, "udp", "udp-relay");
    if present(proxy, "tfo") {
        param(&mut out, proxy, "tfo", "tfo");
    } else {
        param(&mut out, proxy, "fast-open", "tfo");
    }
    param(&mut out, proxy, "test-url", "test-url");
    param(&mut out, proxy, "block-quic", "block-quic");
    Ok(out)
}

pub fn produce(
    proxies: &[Value],
    kind: &str,
    opts: &Value,
    is_loon: bool,
) -> Result<Value, crate::conversion::Error> {
    if opts.is_null() {
        return Err(crate::conversion::Error::Legacy(
            "Cannot read properties of null (reading '_merged')".to_owned(),
        ));
    }
    let mut list: Vec<String> = proxies
        .iter()
        .filter_map(|proxy| if is_loon { loon(proxy) } else { qx(proxy) }.ok())
        .filter(|line| !line.is_empty())
        .collect();
    if truthy(&opts["_merged"]) && codec::to_number(&opts["localPort"]) >= 1.0 {
        let merged = &opts["_merged"];
        let mut config = crate::body::Body::Json(merged["config"].clone()).fields();
        config.extend(crate::body::Body::Json(merged["configOverride"].clone()).fields());
        config.insert("mixed-port".to_owned(), opts["localPort"].clone());
        let config = crate::state::legacy_object_order(Value::Object(config));
        let mut proxy = json!({"type":"external","udp":true,"local-port":opts["localPort"],"args":["-config",STANDARD.encode(codec::stringify(&config))],"addresses":[]});
        for key in ["name", "exec"] {
            if let Some(value) = merged.get(key) {
                proxy[key] = value.clone();
            }
        }
        list.push(external(&proxy).map_err(crate::conversion::Error::Legacy)?);
    }
    Ok(if kind == "internal" {
        json!(list)
    } else {
        json!(list.join("\n"))
    })
}
