use crate::conversion::{Error, field_text as text, path_ed, ss_over_tls, vmess_security};
use crate::json as codec;
use crate::state::truthy;
use serde_json::{Value, json};

pub(crate) const GRPC_METADATA: [&str; 2] = ["_grpc-type", "_grpc-authority"];

#[derive(Clone, Copy, PartialEq)]
pub enum Family {
    Clash,
    Meta,
    Stash,
    Shadowrocket,
}

impl Family {
    pub(crate) fn for_target(target: &str) -> Option<Self> {
        match target {
            "Clash" | "clash" => Some(Self::Clash),
            "meta" | "clashmeta" | "clash.meta" | "Clash.Meta" | "ClashMeta" | "mihomo"
            | "Mihomo" => Some(Self::Meta),
            "stash" | "Stash" => Some(Self::Stash),
            "shadowrocket" | "Shadowrocket" | "ShadowRocket" => Some(Self::Shadowrocket),
            _ => None,
        }
    }
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
    // Like the source, non-strings go through their string form, so true and false are not 1 and 0.
    let text = codec::js_string(value);
    let text = if value.is_string() {
        codec::trim(&text)
    } else {
        &text
    };
    value.is_null() || (!text.is_empty() && supported.contains(&codec::to_number(&json!(text))))
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

/// Mihomo accepts Snell v0-v6 with a PSK and a known obfs mode; v6 also limits the PSK size and mode.
fn meta_snell(proxy: &Value) -> bool {
    // A missing mode compares as "", any other non-string never matches.
    fn mode(value: &Value) -> Option<&str> {
        if value.is_null() {
            Some("")
        } else {
            value.as_str()
        }
    }
    if !supported_version(&proxy["version"], &[0.0, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0]) {
        return false;
    }
    let v6 = codec::to_number(&proxy["version"]) == 6.0;
    let psk = &proxy["psk"];
    let obfs = &proxy["obfs-opts"];
    // Buffer.byteLength counts a lone surrogate as the 3-byte replacement character.
    let psk_bytes: usize = char::decode_utf16(codec::units(&codec::js_string(psk)))
        .map(|unit| unit.map_or(3, char::len_utf8))
        .sum();
    ((psk.is_string() && codec::nonfinite(psk).is_none()) || psk.is_number())
        && (obfs.is_null() || obfs.is_object())
        && mode(&obfs["mode"]).is_some_and(|value| {
            ["", "shadow-tls", "restls", "jls"].contains(&value)
                || (!v6 && ["http", "tls"].contains(&value))
        })
        && (!v6
            || ((12..=255).contains(&psk_bytes)
                && mode(&proxy["mode"]).is_some_and(|value| {
                    ["", "default", "unshaped", "unsafe-raw"].contains(&value)
                })))
}

fn snell_conflict(proxy: &Value) -> bool {
    proxy["type"] == "snell"
        && proxy["plugin"] == "shadow-tls"
        && ["mode", "host", "path"]
            .iter()
            .any(|key| !proxy["obfs-opts"][*key].is_null())
}

pub(crate) fn supported(proxy: &Value, family: Family) -> bool {
    let kind = text(proxy, "type");
    let network = text(proxy, "network");
    let mode = &proxy["plugin-opts"]["mode"];
    if family != Family::Clash && proxy["type"] == "ss" && proxy["plugin"] == "v2ray-plugin" {
        let modes = if family == Family::Shadowrocket {
            &["websocket", "quic", "http2", "mkcp", "grpc"][..]
        } else {
            &["websocket"][..]
        };
        if !mode
            .as_str()
            .is_some_and(|mode| modes.contains(&codec::trim(mode).to_lowercase().as_str()))
        {
            return false;
        }
    }
    match family {
        Family::Clash => {
            if ![
                "ss",
                "ssr",
                "vmess",
                "vless",
                "socks5",
                "http",
                "snell",
                "trojan",
                "wireguard",
            ]
            .contains(&kind.as_str())
                || (kind == "snell" && number(proxy, "version") >= 4.0)
                || (kind == "vless"
                    && (proxy.get("flow").is_some() || truthy(&proxy["reality-opts"])))
                || (network == "ws" && truthy(&proxy["ws-opts"]["v2ray-http-upgrade"]))
                || truthy(&proxy["underlying-proxy"])
                || truthy(&proxy["dialer-proxy"])
            {
                return false;
            }
        }
        Family::Stash => {
            if ![
                "ss",
                "ssr",
                "vmess",
                "socks5",
                "http",
                "snell",
                "trojan",
                "tuic",
                "vless",
                "wireguard",
                "hysteria",
                "hysteria2",
                "ssh",
                "juicity",
                "anytls",
                "tailscale",
                "trusttunnel",
                "masque",
                "mieru",
            ]
            .contains(&kind.as_str())
                || (kind == "snell" && number(proxy, "version") > 6.0)
                || (kind == "vless"
                    && truthy(&proxy["reality-opts"])
                    && truthy(&proxy["network"])
                    && network != "tcp")
                || (network == "ws" && truthy(&proxy["ws-opts"]["v2ray-http-upgrade"]))
            {
                return false;
            }
        }
        Family::Shadowrocket => {
            if (kind == "snell"
                && ![1.0, 2.0, 3.0, 4.0, 5.0, 6.0]
                    .iter()
                    .any(|version| proxy["version"].as_f64() == Some(*version)))
                || snell_conflict(proxy)
                || [
                    "tailscale",
                    "sudoku",
                    "naive",
                    "openvpn",
                    "gost-relay",
                    "shadowquic",
                    "zerotier",
                    "masque-surge",
                    "easytier",
                ]
                .contains(&kind.as_str())
            {
                return false;
            }
            return true;
        }
        Family::Meta => {
            if ["h2-connect", "masque-surge", "juicity", "naive"].contains(&kind.as_str())
                || (kind == "trusttunnel"
                    && proxy["headers"]
                        .as_object()
                        .is_some_and(|headers| !headers.is_empty()))
                || (kind == "snell" && !meta_snell(proxy))
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
            if kind == "vless"
                && network == "xhttp"
                && shadow_tls(&proxy["xhttp-opts"]["download-settings"]).is_some_and(|options| {
                    !supported_version(&options["version"], &[0.0, 1.0, 2.0, 3.0])
                })
            {
                return false;
            }
        }
    }
    if matches!(family, Family::Meta | Family::Stash)
        && ((kind == "anytls"
            && truthy(&proxy["network"])
            && (network != "tcp" || truthy(&proxy["reality-opts"])))
            || (kind != "vless" && network == "xhttp"))
    {
        return false;
    }
    if kind != "ss" {
        return true;
    }
    let cipher = text(proxy, "cipher");
    let classic = [
        "aes-128-gcm",
        "aes-192-gcm",
        "aes-256-gcm",
        "aes-128-cfb",
        "aes-192-cfb",
        "aes-256-cfb",
        "aes-128-ctr",
        "aes-192-ctr",
        "aes-256-ctr",
        "rc4-md5",
        "chacha20-ietf",
        "xchacha20",
        "chacha20-ietf-poly1305",
        "xchacha20-ietf-poly1305",
    ];
    classic.contains(&cipher.as_str())
        || (matches!(family, Family::Meta | Family::Stash)
            && ["2022-blake3-aes-128-gcm", "2022-blake3-aes-256-gcm"].contains(&cipher.as_str()))
        || (family == Family::Meta
            && [
                "aes-128-ccm",
                "aes-192-ccm",
                "aes-256-ccm",
                "aes-128-gcm-siv",
                "aes-256-gcm-siv",
                "chacha20",
                "chacha8-ietf-poly1305",
                "xchacha8-ietf-poly1305",
                "2022-blake3-chacha20-poly1305",
                "lea-128-gcm",
                "lea-192-gcm",
                "lea-256-gcm",
                "rabbit128-poly1305",
                "aegis-128l",
                "aegis-256",
                "aez-384",
                "deoxys-ii-256-128",
                "none",
            ]
            .contains(&cipher.as_str()))
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

fn meta_snell_options(proxy: &mut Value) {
    let version = if proxy["version"].is_null() {
        1.0
    } else {
        codec::to_number(&proxy["version"])
    };
    if !proxy["version"].is_null() && !proxy["version"].is_number() && version.fract() == 0.0 {
        proxy["version"] = codec::numeric(version);
    }
    if version < 3.0 {
        remove(proxy, "udp");
    }
    if version < 6.0 {
        remove(proxy, "mode");
    }
    let Some(options) = shadow_tls(proxy) else {
        return;
    };
    // shortcut: a non-object plugin-opts becomes {} where the source spreads its indexes, port the spread if such input appears.
    let mut restored = options.as_object().cloned().unwrap_or_default();
    if !truthy(&proxy["plugin"]) {
        // The source's lastParse already moved this obfs-opts into plugin-opts: mode dropped, root alpn merged.
        restored.shift_remove("mode");
        if truthy(&proxy["alpn"]) && !restored.get("alpn").is_some_and(truthy) {
            restored.insert("alpn".to_owned(), proxy["alpn"].clone());
        }
        remove(proxy, "alpn");
    }
    restored.insert("mode".to_owned(), json!("shadow-tls"));
    proxy["obfs-opts"] = Value::Object(restored);
    remove(proxy, "plugin");
    remove(proxy, "plugin-opts");
}

fn transport(proxy: &mut Value, kind: &str, network: &str) {
    if ["vmess", "vless"].contains(&kind) && network == "http" {
        if !proxy["http-opts"]["path"].is_null() && !proxy["http-opts"]["path"].is_array() {
            proxy["http-opts"]["path"] = json!([proxy["http-opts"]["path"]]);
        }
        if !proxy["http-opts"]["headers"]["Host"].is_null()
            && !proxy["http-opts"]["headers"]["Host"].is_array()
        {
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
        if ["host", "Host"]
            .iter()
            .any(|key| !options["headers"][*key].is_null())
            || !options["host"].is_null()
        {
            let host = if !options["host"].is_null() {
                &options["host"]
            } else if !options["headers"]["host"].is_null() {
                &options["headers"]["host"]
            } else {
                &options["headers"]["Host"]
            };
            proxy["h2-opts"]["host"] = if host.is_array() {
                host.clone()
            } else {
                json!([host])
            };
        }
        if truthy(&proxy["h2-opts"]["headers"]) {
            remove(&mut proxy["h2-opts"]["headers"], "host");
            remove(&mut proxy["h2-opts"]["headers"], "Host");
            if proxy["h2-opts"]["headers"]
                .as_object()
                .is_some_and(|headers| headers.is_empty())
            {
                remove(&mut proxy["h2-opts"], "headers");
            }
        }
    }
    if network == "ws" {
        if !truthy(&proxy["ws-opts"]) {
            proxy["ws-opts"] = json!({});
        }
        websocket(&mut proxy["ws-opts"]);
    }
}

pub(crate) fn websocket(options: &mut Value) {
    if !truthy(&options["path"]) {
        options["path"] = json!("/");
    }
    let raw = text(options, "path");
    if let (path, Some(ed)) = path_ed(&raw) {
        options["path"] = json!(path);
        if truthy(&options["v2ray-http-upgrade"]) {
            options["v2ray-http-upgrade-fast-open"] = json!(true);
            if options["_v2ray-http-upgrade-ed"].is_null()
                || text(options, "_v2ray-http-upgrade-ed").is_empty()
            {
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

pub(crate) fn strip_metadata(family: Family, kind: &str, opts: &Value) -> bool {
    kind != "internal" || (family == Family::Meta && truthy(&opts["delete-underscore-fields"]))
}

fn wireguard(proxy: &mut Value, cidr: bool) {
    for (from, to) in [
        ("persistent-keepalive", "keepalive"),
        ("pre-shared-key", "preshared-key"),
    ] {
        if proxy[to].is_null() {
            copy(proxy, from, to, false);
        }
        copy(proxy, to, from, false);
    }
    if cidr {
        for (key, cidr) in [("ip", "ip-cidr"), ("ipv6", "ipv6-cidr")] {
            if proxy[key].as_str().is_some_and(|address| {
                crate::conversion::is_ip(address) && address.contains(':') == (key == "ipv6")
            }) && proxy[cidr].as_u64().is_some()
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
        if proxy["type"] == "vless"
            && proxy["network"] == "xhttp"
            && let Some(download) = proxy
                .get_mut("xhttp-opts")
                .and_then(|options| options.get_mut("download-settings"))
            && restore_shadow_tls(download, "servername")
        {
            download["tls"] = json!(true);
        }
    }
    if family == Family::Meta
        && truthy(&proxy["reality-opts"])
        && !truthy(&proxy["client-fingerprint"])
    {
        proxy["client-fingerprint"] = json!("chrome");
    }
    let proxy_type = text(proxy, "type");
    match proxy_type.as_str() {
        "vmess" => {
            if !proxy["aead"].is_null() {
                if truthy(&proxy["aead"]) {
                    proxy["alterId"] = json!(0);
                }
                remove(proxy, "aead");
            }
            if !proxy["sni"].is_null() {
                copy(proxy, "sni", "servername", true);
            }
            proxy["cipher"] = vmess_security(&proxy["cipher"]);
            if matches!(family, Family::Clash | Family::Stash) && proxy["cipher"] == "zero" {
                proxy["cipher"] = json!("auto");
            }
        }
        "vless" => {
            if !proxy["sni"].is_null() {
                copy(proxy, "sni", "servername", true);
            }
            if family == Family::Meta
                && proxy["network"] == "xhttp"
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
                if !proxy["alpn"].is_array() {
                    proxy["alpn"] = json!([proxy["alpn"]]);
                }
            } else if family == Family::Stash {
                proxy["alpn"] = json!(["h3"]);
            }
            alias(proxy, "tfo", "fast-open", family == Family::Stash);
            if (!truthy(&proxy["token"])
                || proxy["token"].as_str().is_some_and(str::is_empty)
                || proxy["token"].as_array().is_some_and(Vec::is_empty))
                && proxy["version"].is_null()
            {
                proxy["version"] = json!(5);
            }
        }
        "hysteria" | "hysteria2" if family != Family::Clash => {
            if proxy_type == "hysteria" {
                alias(proxy, "auth_str", "auth-str", false);
            }
            if family == Family::Stash && proxy_type == "hysteria2" {
                alias(proxy, "password", "auth", true);
            }
            if proxy_type == "hysteria" || family != Family::Meta {
                if (proxy_type == "hysteria" || family == Family::Shadowrocket)
                    && !proxy["alpn"].is_null()
                    && !proxy["alpn"].is_array()
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
                            Some(index) => json!(
                                raw[index..]
                                    .chars()
                                    .take_while(char::is_ascii_digit)
                                    .collect::<String>()
                            ),
                            None => json!(0),
                        };
                    }
                }
            }
        }
        "wireguard" => wireguard(proxy, matches!(family, Family::Meta | Family::Shadowrocket)),
        "snell" if family != Family::Meta => {
            if number(proxy, "version") < 3.0 {
                remove(proxy, "udp");
            }
            let options = if family == Family::Shadowrocket
                && proxy["plugin"] == "shadow-tls"
                && truthy(&proxy["plugin-opts"])
            {
                Some(proxy["plugin-opts"].clone())
            } else {
                None
            };
            if let Some(options) = options {
                let mut restored = json!({"mode":"shadow-tls"});
                for key in ["host", "password", "version"] {
                    if let Some(value) = options.get(key) {
                        restored[key] = value.clone();
                    }
                }
                if truthy(&options["alpn"]) {
                    restored["alpn"] = options["alpn"].clone();
                }
                proxy["obfs-opts"] = restored;
                remove(proxy, "plugin");
                remove(proxy, "plugin-opts");
            }
        }
        "ss" if family == Family::Shadowrocket && ss_over_tls(proxy) => {
            if !proxy["sni"].is_null() {
                copy(proxy, "sni", "servername", false);
            }
        }
        "anytls"
            if matches!(family, Family::Meta | Family::Shadowrocket)
                && !proxy["reuse"].is_null()
                && !truthy(&proxy["reuse"]) =>
        {
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
            Some(raw) if !raw.is_empty() && raw.bytes().all(|c| c.is_ascii_digit()) =>
                codec::to_number(&json!(raw)) != 0.0,
            _ => truthy(mux),
        });
    }
    if family == Family::Meta && proxy_type == "snell" {
        meta_snell_options(proxy);
    }
    let network = text(proxy, "network");
    transport(proxy, &proxy_type, &network);
    if truthy(&proxy["plugin-opts"]["tls"]) && !proxy["skip-cert-verify"].is_null() {
        proxy["plugin-opts"]["skip-cert-verify"] =
            if truthy(&proxy["plugin-opts"]["skip-cert-verify"]) {
                proxy["plugin-opts"]["skip-cert-verify"].clone()
            } else {
                proxy["skip-cert-verify"].clone()
            };
    }
    if [
        "trojan",
        "tuic",
        "hysteria",
        "hysteria2",
        "juicity",
        "anytls",
        "trusttunnel",
        "naive",
    ]
    .contains(&proxy_type.as_str())
        || (family == Family::Meta && ["masque", "shadowquic"].contains(&proxy_type.as_str()))
    {
        remove(proxy, "tls");
    }
    if truthy(&proxy["tls-fingerprint"]) {
        copy(
            proxy,
            "tls-fingerprint",
            if family == Family::Stash {
                "server-cert-fingerprint"
            } else {
                "fingerprint"
            },
            false,
        );
    }
    remove(proxy, "tls-fingerprint");
    if family != Family::Clash {
        if truthy(&proxy["underlying-proxy"]) {
            copy(proxy, "underlying-proxy", "dialer-proxy", false);
        }
        remove(proxy, "underlying-proxy");
    }
    if !proxy["tls"].is_null() && !proxy["tls"].is_boolean() {
        remove(proxy, "tls");
    }
    if family == Family::Stash {
        for (from, to) in [
            ("test-url", "benchmark-url"),
            ("test-timeout", "benchmark-timeout"),
        ] {
            if truthy(&proxy[from]) {
                copy(proxy, from, to, true);
            }
        }
    }
    for key in [
        "subName",
        "collectionName",
        "id",
        "resolved",
        "no-resolve",
        "ip-cidr",
        "ipv6-cidr",
    ] {
        remove(proxy, key);
    }
    if strip_metadata(family, kind, opts) {
        proxy
            .as_object_mut()
            .unwrap()
            .retain(|key, value| !value.is_null() && !key.starts_with('_'));
        if let Some(options) = proxy.get_mut(format!("{network}-opts")) {
            remove(options, "_v2ray-http-upgrade-ed");
        }
    }
    if network == "grpc"
        && let Some(options) = proxy.get_mut("grpc-opts")
    {
        for key in GRPC_METADATA {
            remove(options, key);
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
        if let Some(version) = version {
            proxy["ip-version"] = json!(version);
        }
    }
}

pub fn produce(
    proxies: Vec<Value>,
    kind: &str,
    opts: &Value,
    family: Family,
) -> Result<Value, Error> {
    let mut list: Vec<_> = proxies
        .into_iter()
        .filter(|proxy| truthy(&opts["include-unsupported-proxy"]) || supported(proxy, family))
        .collect();
    for proxy in &mut list {
        normalize(proxy, family, kind, opts);
    }
    finish(list, kind, opts)
}

pub(crate) fn finish(list: Vec<Value>, kind: &str, opts: &Value) -> Result<Value, Error> {
    if kind == "internal" {
        return Ok(Value::Array(list));
    }
    if opts.is_null() {
        return Err(Error::Legacy(
            "Cannot read properties of null (reading 'prettyYaml')".to_owned(),
        ));
    }
    if truthy(&opts["prettyYaml"]) || truthy(&opts["pretty-yaml"]) {
        return crate::clash::Parser::new()?
            .dump(&json!({"proxies":list}))
            .map(Value::String);
    }
    let mut output = "proxies:\n".to_owned();
    for proxy in list {
        if let Some(message) = crate::js_values::circular_error(&proxy) {
            return Err(Error::Legacy(message));
        }
        output.push_str("  - ");
        output.push_str(&codec::stringify(&proxy));
        output.push('\n');
    }
    Ok(json!(output))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meta(proxies: Vec<Value>) -> Vec<Value> {
        let output = produce(proxies, "internal", &json!({}), Family::Meta)
            .ok()
            .unwrap();
        output.as_array().unwrap().clone()
    }

    fn snell(base: Value, config: &Value) -> Value {
        let mut proxy = json!({"type":"snell","server":"snell.example.com","port":44046});
        for source in [&base, config] {
            for (key, value) in source.as_object().unwrap() {
                proxy[key] = value.clone();
            }
        }
        proxy
    }

    // Upstream e043346d, structured.spec.js.
    #[test]
    fn meta_keeps_snell_versions_one_through_six() {
        let proxies = (1..=7)
            .map(|version| {
                snell(
                    json!({"name":format!("Snell {version}"),"psk":"snell-test-psk","version":version,"udp":true}),
                    &json!({}),
                )
            })
            .collect();
        let versions: Vec<_> = meta(proxies)
            .iter()
            .map(|proxy| proxy["version"].as_f64().unwrap())
            .collect();
        assert_eq!(versions, [1.0, 2.0, 3.0, 4.0, 5.0, 6.0]);
    }

    #[test]
    fn meta_round_trips_snell_v6_modes_and_obfs_options() {
        let configs: Vec<_> = [
            json!({}),
            json!({"mode":"default","obfs-opts":{"mode":"shadow-tls","host":"mask.example.com","password":"shadow-pass",
                "version":3,"alpn":[],"name-cert-verify":"cert.example.com","skip-cert-verify":false}}),
            json!({"mode":"unshaped","obfs-opts":{"mode":"restls","host":"mask.example.com","password":"restls-pass",
                "version-hint":"tls13"}}),
            json!({"mode":"unsafe-raw","obfs-opts":{"mode":"jls","host":"mask.example.com","username":"test-user",
                "password":"jls-pass"}}),
        ]
        .iter()
        .enumerate()
        .map(|(index, config)| {
            let base = json!({"name":format!("Snell v6 {index}"),"version":6,"psk":"测试密码",
                "udp":index % 2 == 0,"reuse":index % 2 != 0});
            snell(base, config)
        })
        .collect();
        assert_eq!(meta(configs.clone()), configs);
        // Parsed input: lastParse moved the obfs-opts into plugin-opts, so the mode comes last.
        let output = &meta(vec![configs[1].clone()])[0];
        assert_eq!(
            output["obfs-opts"]
                .as_object()
                .unwrap()
                .keys()
                .next_back()
                .unwrap(),
            "mode"
        );
        // A plugin-style source keeps its options and gains the mode.
        let plugin = snell(
            json!({"name":"plugin","version":6,"psk":"snell-test-psk","plugin":"shadow-tls"}),
            &json!({"plugin-opts":{"host":"mask.example.com","password":"p","version":3}}),
        );
        let output = &meta(vec![plugin])[0];
        assert_eq!(
            output["obfs-opts"],
            json!({"host":"mask.example.com","password":"p","version":3,"mode":"shadow-tls"})
        );
        assert!(output.get("plugin").is_none() && output.get("plugin-opts").is_none());
    }

    #[test]
    fn meta_filters_invalid_snell_v6_and_drops_mode_before_v6() {
        let proxies = [
            json!({"mode":"unknown"}),
            json!({"obfs-opts":{"mode":"http"}}),
            json!({"obfs-opts":{"mode":"tls"}}),
            json!({"psk":"x".repeat(11)}),
            json!({"psk":"x".repeat(256)}),
            json!({"psk":"x".repeat(255)}),
            json!({"version":5}),
        ]
        .iter()
        .enumerate()
        .map(|(index, config)| {
            let base = json!({"name":format!("Snell {index}"),"version":"6","mode":"unshaped","psk":"snell-test-psk"});
            snell(base, config)
        })
        .collect();
        let result: Vec<_> = meta(proxies)
            .iter()
            .map(|proxy| {
                (
                    proxy["name"].clone(),
                    proxy["version"].as_f64(),
                    proxy.get("mode").cloned(),
                )
            })
            .collect();
        assert_eq!(
            result,
            [
                (json!("Snell 5"), Some(6.0), Some(json!("unshaped"))),
                (json!("Snell 6"), Some(5.0), None),
            ]
        );
    }

    #[test]
    fn meta_validates_legacy_snell_options_and_the_default_version() {
        let proxies = [
            json!({"name":"default","obfs-opts":{"mode":"http"}}),
            json!({"name":"zero","version":0,"obfs-opts":{"mode":"tls"}}),
            json!({"name":"v5","version":" 5 ","obfs-opts":{"mode":"http"}}),
            json!({"name":"invalid obfs","version":5,"obfs-opts":{"mode":"unknown"}}),
            json!({"name":"invalid obfs options","version":4,"obfs-opts":"http"}),
            json!({"name":"missing PSK","version":3,"psk":null}),
            // Not upstream: false is "false", not 0, for the version check.
            json!({"name":"false version","version":false}),
        ]
        .iter()
        .map(|config| snell(json!({"psk":"short","udp":true}), config))
        .collect();
        let result: Vec<_> = meta(proxies)
            .iter()
            .map(|proxy| {
                (
                    proxy["name"].clone(),
                    proxy["version"].as_f64(),
                    proxy.get("udp").cloned(),
                )
            })
            .collect();
        assert_eq!(
            result,
            [
                (json!("default"), None, None),
                (json!("zero"), Some(0.0), None),
                (json!("v5"), Some(5.0), Some(json!(true))),
            ]
        );
    }
}
