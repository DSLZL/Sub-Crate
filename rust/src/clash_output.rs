use crate::conversion::{Error, field_text as text, path_ed, ss_over_tls, vmess_security};
use crate::json as codec;
use crate::state::truthy;
use serde_json::{Value, json};

#[derive(Clone, Copy, PartialEq)]
pub enum Family {
    Clash,
    Meta,
    Stash,
    Shadowrocket,
}

fn number(value: &Value, key: &str) -> f64 {
    value.get(key).map(codec::to_number).unwrap_or(f64::NAN)
}

fn remove(value: &mut Value, key: &str) {
    if let Some(object) = value.as_object_mut() {
        object.shift_remove(key);
    }
}

fn copy(value: &mut Value, from: &str, to: &str, delete: bool) {
    if let Some(field) = value.get(from).cloned() {
        value[to] = field;
    } else {
        remove(value, to);
    }
    if delete {
        remove(value, from);
    }
}

fn alias(value: &mut Value, from: &str, to: &str, delete: bool) {
    if value[to].is_null() && !value[from].is_null() {
        copy(value, from, to, delete);
    }
}

fn supported_version(value: &Value, supported: &[f64]) -> bool {
    value.is_null()
        || (!codec::trim(&codec::js_string(value)).is_empty()
            && supported.contains(&codec::to_number(value)))
}

fn shadow_tls(proxy: &Value) -> Option<&Value> {
    if proxy["plugin"] == "shadow-tls" && truthy(&proxy["plugin-opts"]) {
        Some(&proxy["plugin-opts"])
    } else if proxy["type"] == "snell" && proxy["obfs-opts"]["mode"] == "shadow-tls" {
        Some(&proxy["obfs-opts"])
    } else {
        None
    }
}

fn snell_conflict(proxy: &Value) -> bool {
    proxy["type"] == "snell"
        && proxy["plugin"] == "shadow-tls"
        && ["mode", "host", "path"].iter().any(|key| !proxy["obfs-opts"][*key].is_null())
}

fn supported(proxy: &Value, family: Family) -> bool {
    let kind = text(proxy, "type");
    let network = text(proxy, "network");
    let mode = &proxy["plugin-opts"]["mode"];
    if family != Family::Clash && proxy["type"] == "ss" && proxy["plugin"] == "v2ray-plugin" {
        let modes = if family == Family::Shadowrocket {
            &["websocket", "quic", "http2", "mkcp", "grpc"][..]
        } else {
            &["websocket"][..]
        };
        if !mode.as_str().is_some_and(|mode| modes.contains(&codec::trim(mode).to_lowercase().as_str())) {
            return false;
        }
    }
    match family {
        Family::Clash => {
            if !["ss", "ssr", "vmess", "vless", "socks5", "http", "snell", "trojan", "wireguard"].contains(&kind.as_str())
                || (kind == "snell" && number(proxy, "version") >= 4.0)
                || (kind == "vless" && (proxy.get("flow").is_some() || truthy(&proxy["reality-opts"])))
                || (network == "ws" && truthy(&proxy["ws-opts"]["v2ray-http-upgrade"]))
                || truthy(&proxy["underlying-proxy"])
                || truthy(&proxy["dialer-proxy"])
            {
                return false;
            }
        }
        Family::Stash => {
            if !["ss", "ssr", "vmess", "socks5", "http", "snell", "trojan", "tuic", "vless",
                "wireguard", "hysteria", "hysteria2", "ssh", "juicity", "anytls", "tailscale",
                "trusttunnel", "masque", "mieru"].contains(&kind.as_str())
                || (kind == "snell" && number(proxy, "version") > 6.0)
                || (kind == "vless" && truthy(&proxy["reality-opts"]) && truthy(&proxy["network"]) && network != "tcp")
                || (network == "ws" && truthy(&proxy["ws-opts"]["v2ray-http-upgrade"]))
            {
                return false;
            }
        }
        Family::Shadowrocket => {
            if (kind == "snell" && ![1.0, 2.0, 3.0, 4.0, 5.0, 6.0].iter().any(|version| proxy["version"].as_f64() == Some(*version)))
                || snell_conflict(proxy)
                || ["tailscale", "sudoku", "naive", "openvpn", "gost-relay", "shadowquic", "zerotier", "masque-surge", "easytier"].contains(&kind.as_str())
            {
                return false;
            }
            return true;
        }
        Family::Meta => {
            if ["h2-connect", "masque-surge", "juicity", "naive"].contains(&kind.as_str())
                || (kind == "trusttunnel" && proxy["headers"].as_object().is_some_and(|headers| !headers.is_empty()))
                || (kind == "snell" && !supported_version(&proxy["version"], &[1.0, 2.0, 3.0, 4.0, 5.0]))
                || snell_conflict(proxy)
            {
                return false;
            }
            if let Some(options) = shadow_tls(proxy) {
                let versions = if ["ss", "snell"].contains(&kind.as_str()) {
                    &[1.0, 2.0, 3.0][..]
                } else {
                    &[0.0, 1.0, 2.0, 3.0][..]
                };
                if !["ss", "snell", "vmess", "vless", "trojan", "anytls"].contains(&kind.as_str())
                    || !supported_version(&options["version"], versions)
                {
                    return false;
                }
            }
            if kind == "vless" && network == "xhttp"
                && shadow_tls(&proxy["xhttp-opts"]["download-settings"])
                    .is_some_and(|options| !supported_version(&options["version"], &[0.0, 1.0, 2.0, 3.0]))
            {
                return false;
            }
        }
    }
    if matches!(family, Family::Meta | Family::Stash)
        && ((kind == "anytls" && truthy(&proxy["network"])
            && (network != "tcp" || truthy(&proxy["reality-opts"])))
            || (kind != "vless" && network == "xhttp"))
    {
        return false;
    }
    if kind != "ss" {
        return true;
    }
    let cipher = text(proxy, "cipher");
    let classic = ["aes-128-gcm", "aes-192-gcm", "aes-256-gcm", "aes-128-cfb", "aes-192-cfb", "aes-256-cfb",
        "aes-128-ctr", "aes-192-ctr", "aes-256-ctr", "rc4-md5", "chacha20-ietf", "xchacha20",
        "chacha20-ietf-poly1305", "xchacha20-ietf-poly1305"];
    classic.contains(&cipher.as_str())
        || (matches!(family, Family::Meta | Family::Stash)
            && ["2022-blake3-aes-128-gcm", "2022-blake3-aes-256-gcm"].contains(&cipher.as_str()))
        || (family == Family::Meta
            && ["aes-128-ccm", "aes-192-ccm", "aes-256-ccm", "aes-128-gcm-siv", "aes-256-gcm-siv",
                "chacha20", "chacha8-ietf-poly1305", "xchacha8-ietf-poly1305", "2022-blake3-chacha20-poly1305",
                "lea-128-gcm", "lea-192-gcm", "lea-256-gcm", "rabbit128-poly1305", "aegis-128l", "aegis-256",
                "aez-384", "deoxys-ii-256-128", "none"].contains(&cipher.as_str()))
}

fn restore_shadow_tls(proxy: &mut Value, name: &str) -> bool {
    if proxy["plugin"] != "shadow-tls" || !truthy(&proxy["plugin-opts"]) {
        return false;
    }
    let options = proxy["plugin-opts"].clone();
    let enabled = truthy(&options["password"])
        || (!options["version"].is_null() && codec::to_number(&options["version"]) != 0.0);
    let mut restored = json!({});
    for key in ["password", "version"] {
        if let Some(value) = options.get(key) {
            restored[key] = value.clone();
        }
    }
    proxy["shadow-tls-opts"] = restored;
    for (from, to) in [("host", name), ("alpn", "alpn")] {
        if !options[from].is_null() {
            proxy[to] = options[from].clone();
        }
    }
    remove(proxy, "plugin");
    remove(proxy, "plugin-opts");
    enabled
}

fn transport(proxy: &mut Value, kind: &str, network: &str) {
    if ["vmess", "vless"].contains(&kind) && network == "http" {
        if !proxy["http-opts"]["path"].is_null() && !proxy["http-opts"]["path"].is_array() {
            proxy["http-opts"]["path"] = json!([proxy["http-opts"]["path"]]);
        }
        if !proxy["http-opts"]["headers"]["Host"].is_null() && !proxy["http-opts"]["headers"]["Host"].is_array() {
            proxy["http-opts"]["headers"]["Host"] = json!([proxy["http-opts"]["headers"]["Host"]]);
        }
    }
    if ["vmess", "vless"].contains(&kind) && network == "h2" {
        if let Some(paths) = proxy["h2-opts"]["path"].as_array() {
            if let Some(path) = paths.first().cloned() {
                proxy["h2-opts"]["path"] = path;
            } else {
                remove(&mut proxy["h2-opts"], "path");
            }
        }
        let options = &proxy["h2-opts"];
        if ["host", "Host"].iter().any(|key| !options["headers"][*key].is_null()) || !options["host"].is_null() {
            let host = if !options["host"].is_null() { &options["host"] }
                else if !options["headers"]["host"].is_null() { &options["headers"]["host"] }
                else { &options["headers"]["Host"] };
            proxy["h2-opts"]["host"] = if host.is_array() { host.clone() } else { json!([host]) };
        }
        if truthy(&proxy["h2-opts"]["headers"]) {
            remove(&mut proxy["h2-opts"]["headers"], "host");
            remove(&mut proxy["h2-opts"]["headers"], "Host");
            if proxy["h2-opts"]["headers"].as_object().is_some_and(|headers| headers.is_empty()) {
                remove(&mut proxy["h2-opts"], "headers");
            }
        }
    }
    if network == "ws" {
        if !truthy(&proxy["ws-opts"]) {
            proxy["ws-opts"] = json!({});
        }
        let options = &mut proxy["ws-opts"];
        if !truthy(&options["path"]) {
            options["path"] = json!("/");
        }
        let raw = text(options, "path");
        if let (path, Some(ed)) = path_ed(&raw) {
            options["path"] = json!(path);
            if truthy(&options["v2ray-http-upgrade"]) {
                options["v2ray-http-upgrade-fast-open"] = json!(true);
                if options["_v2ray-http-upgrade-ed"].is_null() || text(options, "_v2ray-http-upgrade-ed").is_empty() {
                    options["_v2ray-http-upgrade-ed"] = json!(ed);
                }
            } else {
                if options["early-data-header-name"].is_null() {
                    options["early-data-header-name"] = json!("Sec-WebSocket-Protocol");
                }
                if options["max-early-data"].is_null() {
                    options["max-early-data"] = json!(ed.parse::<u64>().unwrap());
                }
            }
        }
        if truthy(&options["v2ray-http-upgrade"]) {
            remove(options, "early-data-header-name");
            remove(options, "max-early-data");
        }
    }
}

fn wireguard(proxy: &mut Value, cidr: bool) {
    for (from, to) in [("persistent-keepalive", "keepalive"), ("pre-shared-key", "preshared-key")] {
        if proxy[to].is_null() {
            copy(proxy, from, to, false);
        }
        copy(proxy, to, from, false);
    }
    if cidr {
        for (key, cidr) in [("ip", "ip-cidr"), ("ipv6", "ipv6-cidr")] {
            if proxy[key].as_str().is_some_and(|address| crate::conversion::is_ip(address) && address.contains(':') == (key == "ipv6"))
                && proxy[cidr].as_u64().is_some()
            {
                proxy[key] = json!(format!("{}/{}", text(proxy, key), text(proxy, cidr)));
            } else {
                remove(proxy, key);
            }
        }
    }
}

fn normalize(proxy: &mut Value, family: Family, kind: &str, opts: &Value) {
    if matches!(family, Family::Meta | Family::Shadowrocket) {
        if ["vmess", "vless", "trojan", "anytls"].contains(&text(proxy, "type").as_str()) {
            let enabled = restore_shadow_tls(proxy, "sni");
            if enabled && matches!(proxy["type"].as_str(), Some("vmess" | "vless")) {
                proxy["tls"] = json!(true);
            }
        }
        if proxy["type"] == "vless" && proxy["network"] == "xhttp"
            && let Some(download) = proxy.get_mut("xhttp-opts").and_then(|options| options.get_mut("download-settings"))
            && restore_shadow_tls(download, "servername")
        {
            download["tls"] = json!(true);
        }
    }
    if family == Family::Meta && truthy(&proxy["reality-opts"]) && !truthy(&proxy["client-fingerprint"]) {
        proxy["client-fingerprint"] = json!("chrome");
    }
    let proxy_type = text(proxy, "type");
    match proxy_type.as_str() {
        "vmess" => {
            if !proxy["aead"].is_null() {
                if truthy(&proxy["aead"]) { proxy["alterId"] = json!(0); }
                remove(proxy, "aead");
            }
            if !proxy["sni"].is_null() { copy(proxy, "sni", "servername", true); }
            proxy["cipher"] = vmess_security(&proxy["cipher"]);
            if matches!(family, Family::Clash | Family::Stash) && proxy["cipher"] == "zero" {
                proxy["cipher"] = json!("auto");
            }
        }
        "vless" => {
            if !proxy["sni"].is_null() { copy(proxy, "sni", "servername", true); }
            if family == Family::Meta && proxy["network"] == "xhttp"
                && truthy(&proxy["xhttp-opts"]["download-settings"])
                && truthy(&proxy["tls"])
                && truthy(&proxy["xhttp-opts"]["download-settings"]["tls"])
                && truthy(&proxy["reality-opts"])
                && !truthy(&proxy["xhttp-opts"]["download-settings"]["reality-opts"])
            {
                proxy["xhttp-opts"]["download-settings"]["reality-opts"] = json!({"public-key":""});
            }
        }
        "tuic" if family != Family::Clash => {
            if !proxy["alpn"].is_null() {
                if !proxy["alpn"].is_array() { proxy["alpn"] = json!([proxy["alpn"]]); }
            } else if family == Family::Stash {
                proxy["alpn"] = json!(["h3"]);
            }
            alias(proxy, "tfo", "fast-open", family == Family::Stash);
            if (!truthy(&proxy["token"]) || proxy["token"].as_str().is_some_and(str::is_empty)
                || proxy["token"].as_array().is_some_and(Vec::is_empty))
                && proxy["version"].is_null()
            {
                proxy["version"] = json!(5);
            }
        }
        "hysteria" | "hysteria2" if family != Family::Clash => {
            if proxy_type == "hysteria" { alias(proxy, "auth_str", "auth-str", false); }
            if family == Family::Stash && proxy_type == "hysteria2" { alias(proxy, "password", "auth", true); }
            if proxy_type == "hysteria" || family != Family::Meta {
                if (proxy_type == "hysteria" || family == Family::Shadowrocket)
                    && !proxy["alpn"].is_null() && !proxy["alpn"].is_array()
                {
                    proxy["alpn"] = json!([proxy["alpn"]]);
                }
                alias(proxy, "tfo", "fast-open", family == Family::Stash);
            }
            if family == Family::Stash {
                for (from, to) in [("down", "down-speed"), ("up", "up-speed")] {
                    alias(proxy, from, to, true);
                    if !proxy[to].is_null() {
                        let raw = text(proxy, to);
                        proxy[to] = match raw.find(|c: char| c.is_ascii_digit()) {
                            Some(index) => json!(raw[index..].chars().take_while(char::is_ascii_digit).collect::<String>()),
                            None => json!(0),
                        };
                    }
                }
            }
        }
        "wireguard" => wireguard(proxy, matches!(family, Family::Meta | Family::Shadowrocket)),
        "snell" => {
            if number(proxy, "version") < 3.0 { remove(proxy, "udp"); }
            let options = if family == Family::Meta { shadow_tls(proxy).cloned() }
                else if family == Family::Shadowrocket && proxy["plugin"] == "shadow-tls" && truthy(&proxy["plugin-opts"]) {
                    Some(proxy["plugin-opts"].clone())
                } else { None };
            if let Some(options) = options {
                let mut restored = json!({"mode":"shadow-tls"});
                for key in ["host", "password", "version"] {
                    if let Some(value) = options.get(key) { restored[key] = value.clone(); }
                }
                if truthy(&options["alpn"]) { restored["alpn"] = options["alpn"].clone(); }
                proxy["obfs-opts"] = restored;
                remove(proxy, "plugin");
                remove(proxy, "plugin-opts");
            }
        }
        "ss" if family == Family::Shadowrocket && ss_over_tls(proxy) => {
                if !proxy["sni"].is_null() { copy(proxy, "sni", "servername", false); }
            }
        "anytls" if matches!(family, Family::Meta | Family::Shadowrocket)
            && !proxy["reuse"].is_null() && !truthy(&proxy["reuse"]) => {
                proxy["disable-reuse"] = json!(true);
                remove(proxy, "reuse");
            }
        _ => {}
    }
    if family == Family::Meta && !proxy["plugin-opts"]["mux"].is_null() {
        let mux = &proxy["plugin-opts"]["mux"];
        let normalized = mux.as_str().map(|value| codec::trim(value).to_lowercase());
        proxy["plugin-opts"]["mux"] = json!(match normalized.as_deref() {
            Some("true") => true,
            Some("false") => false,
            Some(raw) if !raw.is_empty() && raw.bytes().all(|c| c.is_ascii_digit()) => codec::to_number(&json!(raw)) != 0.0,
            _ => truthy(mux),
        });
    }
    let network = text(proxy, "network");
    transport(proxy, &proxy_type, &network);
    if truthy(&proxy["plugin-opts"]["tls"]) && !proxy["skip-cert-verify"].is_null() {
        proxy["plugin-opts"]["skip-cert-verify"] = if truthy(&proxy["plugin-opts"]["skip-cert-verify"]) {
            proxy["plugin-opts"]["skip-cert-verify"].clone()
        } else { proxy["skip-cert-verify"].clone() };
    }
    if ["trojan", "tuic", "hysteria", "hysteria2", "juicity", "anytls", "trusttunnel", "naive"].contains(&proxy_type.as_str())
        || (family == Family::Meta && ["masque", "shadowquic"].contains(&proxy_type.as_str()))
    {
        remove(proxy, "tls");
    }
    if truthy(&proxy["tls-fingerprint"]) {
        copy(proxy, "tls-fingerprint", if family == Family::Stash { "server-cert-fingerprint" } else { "fingerprint" }, false);
    }
    remove(proxy, "tls-fingerprint");
    if family != Family::Clash {
        if truthy(&proxy["underlying-proxy"]) { copy(proxy, "underlying-proxy", "dialer-proxy", false); }
        remove(proxy, "underlying-proxy");
    }
    if !proxy["tls"].is_null() && !proxy["tls"].is_boolean() { remove(proxy, "tls"); }
    if family == Family::Stash {
        for (from, to) in [("test-url", "benchmark-url"), ("test-timeout", "benchmark-timeout")] {
            if truthy(&proxy[from]) { copy(proxy, from, to, true); }
        }
    }
    for key in ["subName", "collectionName", "id", "resolved", "no-resolve", "ip-cidr", "ipv6-cidr"] { remove(proxy, key); }
    if kind != "internal" || (family == Family::Meta && truthy(&opts["delete-underscore-fields"])) {
        proxy.as_object_mut().unwrap().retain(|key, value| !value.is_null() && !key.starts_with('_'));
        if let Some(options) = proxy.get_mut(format!("{network}-opts")) { remove(options, "_v2ray-http-upgrade-ed"); }
    }
    if network == "grpc" {
        if let Some(options) = proxy.get_mut("grpc-opts") {
            remove(options, "_grpc-type");
            remove(options, "_grpc-authority");
        }
    }
    if family == Family::Meta && truthy(&proxy["ip-version"]) {
        let version = match proxy["ip-version"].as_str() {
            Some("v4-only") => Some("ipv4"),
            Some("v6-only") => Some("ipv6"),
            Some("prefer-v4") => Some("ipv4-prefer"),
            Some("prefer-v6") => Some("ipv6-prefer"),
            _ => None,
        };
        if let Some(version) = version { proxy["ip-version"] = json!(version); }
    }
}

pub fn produce(proxies: Vec<Value>, kind: &str, opts: &Value, family: Family) -> Result<Value, Error> {
    let mut list: Vec<_> = proxies.into_iter()
        .filter(|proxy| truthy(&opts["include-unsupported-proxy"]) || supported(proxy, family)).collect();
    for proxy in &mut list { normalize(proxy, family, kind, opts); }
    if kind == "internal" { return Ok(json!(list)); }
    if opts.is_null() {
        return Err(Error::Legacy("Cannot read properties of null (reading 'prettyYaml')".to_owned()));
    }
    if truthy(&opts["prettyYaml"]) || truthy(&opts["pretty-yaml"]) {
        return crate::clash::Parser::new()?.dump(&json!({"proxies":list})).map(Value::String);
    }
    let mut output = "proxies:\n".to_owned();
    for proxy in list {
        output.push_str("  - ");
        output.push_str(&codec::stringify(&proxy));
        output.push('\n');
    }
    Ok(json!(output))
}
