use crate::json as codec;
use crate::state::truthy;
use serde_json::{Map, Value, json};

#[derive(Clone, Copy)]
enum Kind {
    Text,
    AnyText,
    True,
    StrictTextRange,
    StrictRange,
    PositiveRange,
    NonNegativeRange,
}

const FIELDS: &[(&str, &str, Kind)] = &[
    ("noGRPCHeader", "no-grpc-header", Kind::True),
    ("xPaddingBytes", "x-padding-bytes", Kind::StrictTextRange),
    ("xPaddingObfsMode", "x-padding-obfs-mode", Kind::True),
    ("xPaddingKey", "x-padding-key", Kind::Text),
    ("xPaddingHeader", "x-padding-header", Kind::Text),
    ("xPaddingPlacement", "x-padding-placement", Kind::Text),
    ("xPaddingMethod", "x-padding-method", Kind::Text),
    ("uplinkHTTPMethod", "uplink-http-method", Kind::Text),
    ("sessionIDPlacement", "session-placement", Kind::Text),
    ("sessionIDKey", "session-key", Kind::Text),
    ("sessionIDTable", "session-table", Kind::AnyText),
    ("sessionIDLength", "session-length", Kind::StrictTextRange),
    ("seqPlacement", "seq-placement", Kind::Text),
    ("seqKey", "seq-key", Kind::Text),
    ("uplinkDataPlacement", "uplink-data-placement", Kind::Text),
    ("uplinkDataKey", "uplink-data-key", Kind::Text),
    ("uplinkChunkSize", "uplink-chunk-size", Kind::NonNegativeRange),
    ("scMaxEachPostBytes", "sc-max-each-post-bytes", Kind::StrictRange),
    ("scMinPostsIntervalMs", "sc-min-posts-interval-ms", Kind::PositiveRange),
];

const REUSE: &[(&str, &str)] = &[
    ("maxConnections", "max-connections"),
    ("maxConcurrency", "max-concurrency"),
    ("cMaxReuseTimes", "c-max-reuse-times"),
    ("hMaxRequestTimes", "h-max-request-times"),
    ("hMaxReusableSecs", "h-max-reusable-secs"),
];

pub fn integer(value: &Value, negative: bool) -> Option<i64> {
    let number = match value {
        Value::Number(value) => value.as_f64()?,
        Value::String(value) => {
            let raw = codec::trim(value);
            if !negative && raw.starts_with('-') {
                return None;
            }
            let digits = raw.strip_prefix('+').or_else(|| raw.strip_prefix('-')).unwrap_or(raw);
            if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
                return None;
            }
            raw.parse::<f64>().ok()?
        }
        _ => return None,
    };
    (number.is_finite()
        && number.fract() == 0.0
        && number.abs() <= 9_007_199_254_740_991.0
        && (negative || number >= 0.0))
        .then_some(number as i64)
}

fn range(value: &Value, lower: i64, upper: i64, text_only: bool) -> Option<Value> {
    if !value.is_string() && (text_only || !value.is_number()) {
        return None;
    }
    let raw = codec::js_string(value);
    let parts: Vec<_> = codec::trim(&raw).split('-').collect();
    if !(1..=2).contains(&parts.len()) {
        return None;
    }
    let parse = |raw: &str| integer(&json!(codec::trim(raw)), false);
    let min = parse(parts[0])?;
    let max = parse(parts.get(1).unwrap_or(&parts[0]))?;
    if min < lower || max < upper || min > max {
        return None;
    }
    Some(if min != max {
        json!(format!("{min}-{max}"))
    } else if text_only {
        json!(min.to_string())
    } else {
        json!(min)
    })
}

fn nonblank(value: &Value) -> bool {
    value.as_str().is_some_and(|raw| !codec::trim(raw).is_empty())
}

fn parsed_field(value: &Value, kind: Kind) -> Option<Value> {
    match kind {
        Kind::Text | Kind::AnyText => value.as_str().map(|_| value.clone()),
        Kind::True => (value == true).then(|| value.clone()),
        Kind::StrictTextRange => range(value, 1, 1, true),
        Kind::StrictRange => range(value, 1, 1, false),
        Kind::PositiveRange => range(value, 0, 1, false),
        Kind::NonNegativeRange => range(value, 0, 0, false),
    }
}

fn compact(value: Value) -> Option<Value> {
    match value {
        Value::Object(object) => {
            let object: Map<_, _> = object.into_iter()
                .filter_map(|(key, value)| compact(value).map(|value| (key, value)))
                .collect();
            (!object.is_empty()).then_some(Value::Object(object))
        }
        Value::Array(values) => Some(Value::Array(values.into_iter().filter_map(compact).collect())),
        value => Some(value),
    }
}

fn reuse(value: &Value, output: bool) -> Option<Value> {
    value.as_object()?;
    let mut result = json!({});
    for (wire, node) in REUSE {
        if let Some(value) = range(&value[if output { *node } else { *wire }], 0, 0, false) {
            result[if output { *wire } else { *node }] = json!(codec::js_string(&value));
        }
    }
    if let Some(value) = integer(&value[if output { "h-keep-alive-period" } else { "hKeepAlivePeriod" }], true) {
        result[if output { "hKeepAlivePeriod" } else { "h-keep-alive-period" }] = json!(value);
    }
    compact(result)
}

fn unsupported_reuse(value: &Value) -> Option<Value> {
    if value.is_null() {
        return None;
    }
    let Some(object) = value.as_object() else {
        return Some(value.clone());
    };
    compact(Value::Object(object.iter().filter_map(|(key, value)| {
        let supported = if REUSE.iter().any(|(wire, _)| wire == key) {
            range(value, 0, 0, false).is_some()
        } else if key == "hKeepAlivePeriod" {
            integer(value, true).is_some()
        } else {
            false
        };
        (!supported).then(|| (key.clone(), value.clone()))
    }).collect()))
}

fn headers(value: &Value, unsupported: bool, exclude_host: bool) -> Option<Value> {
    if value.is_null() {
        return None;
    }
    let Some(object) = value.as_object() else {
        return unsupported.then(|| value.clone());
    };
    compact(Value::Object(object.iter().filter(|(key, value)| {
        value.is_string() != unsupported && !(exclude_host && key.eq_ignore_ascii_case("host"))
    }).map(|(key, value)| (key.clone(), value.clone())).collect()))
}

fn unsupported_extra(value: &Value) -> Option<Value> {
    if value.is_null() {
        return None;
    }
    let Some(object) = value.as_object() else {
        return Some(value.clone());
    };
    let mut result = json!({});
    for (key, value) in object {
        let unsupported = match key.as_str() {
            "headers" => headers(value, true, false),
            "xmux" => unsupported_reuse(value),
            "sessionPlacement" | "sessionKey" => (!value.is_string()).then(|| value.clone()),
            _ => {
                let supported = FIELDS.iter().find(|(wire, _, _)| *wire == key)
                    .is_some_and(|(_, _, kind)| parsed_field(value, *kind).is_some());
                (!supported).then(|| value.clone())
            }
        };
        if let Some(value) = unsupported {
            result[key] = value;
        }
    }
    compact(result)
}

fn apply_extra(target: &mut Value, extra: &Value) {
    if !extra.is_object() {
        return;
    }
    if let Some(Value::Object(parsed)) = headers(&extra["headers"], false, false) {
        let mut combined = target["headers"].as_object().cloned().unwrap_or_default();
        for (key, value) in parsed {
            if key.eq_ignore_ascii_case("host") {
                if !combined.contains_key("Host") && !combined.contains_key("host") {
                    combined.insert("Host".to_owned(), value);
                }
            } else {
                combined.insert(key, value);
            }
        }
        if !combined.is_empty() {
            target["headers"] = Value::Object(combined);
        }
    }
    for (wire, node, kind) in FIELDS {
        let mut raw = &extra[*wire];
        if *wire == "sessionIDPlacement" && !nonblank(raw) {
            raw = &extra["sessionPlacement"];
        } else if *wire == "sessionIDKey" && !nonblank(raw) {
            raw = &extra["sessionKey"];
        }
        if matches!(kind, Kind::Text) && !nonblank(raw) {
            continue;
        }
        if let Some(value) = parsed_field(raw, *kind) {
            target[*node] = value;
        }
    }
    if let Some(value) = reuse(&extra["xmux"], false) {
        target["reuse-settings"] = value;
    }
}

pub fn ech_options(fields: &Value) -> Option<Value> {
    let raw = fields["echConfigList"].as_str()?;
    if codec::trim(raw).is_empty() {
        return None;
    }
    let mut result = json!({"enable":true});
    if !raw.contains("://") {
        result["config"] = json!(raw);
    } else {
        let parts: Vec<_> = raw.split('+').collect();
        if !(1..=2).contains(&parts.len()) || parts.iter().any(|p| codec::trim(p).is_empty()) {
            return None;
        }
        result["_dns"] = json!(parts[parts.len() - 1]);
        if parts.len() == 2 {
            result["query-server-name"] = json!(parts[0]);
        }
    }
    if matches!(fields["echForceQuery"].as_str(), Some("none" | "half" | "full")) {
        result["_force-query"] = fields["echForceQuery"].clone();
    }
    if fields["echSockopt"].is_object() {
        result["_sockopt"] = fields["echSockopt"].clone();
    }
    Some(result)
}

pub fn ech_fields(options: &Value, fallback: &Value) -> Value {
    let mut result = json!({});
    if !options.is_object() {
        if nonblank(fallback) {
            result["echConfigList"] = fallback.clone();
        }
        return result;
    }
    let enabled = options["enable"] == true
        || options["enable"].as_f64().is_some_and(|value| value.fract() == 0.0 && value != 0.0);
    if !enabled {
        return result;
    }
    if nonblank(&options["config"]) {
        result["echConfigList"] = options["config"].clone();
    } else {
        let query = &options["query-server-name"];
        let dns = &options["_dns"];
        if nonblank(dns) || nonblank(query) {
            let dns = if nonblank(dns) {
                codec::js_string(dns)
            } else {
                "https://dns.alidns.com/dns-query".to_owned()
            };
            result["echConfigList"] = json!(if nonblank(query) {
                format!("{}+{dns}", codec::js_string(query))
            } else {
                dns
            });
        }
    }
    if result.get("echConfigList").is_some() {
        if matches!(options["_force-query"].as_str(), Some("none" | "half" | "full")) {
            result["echForceQuery"] = options["_force-query"].clone();
        }
        if options["_sockopt"].is_object() {
            result["echSockopt"] = options["_sockopt"].clone();
        }
    }
    result
}

fn valid_alpn(value: &Value) -> bool {
    value.as_array().is_some_and(|values| !values.is_empty()
        && values.iter().all(|value| value.as_str().is_some_and(|raw| !raw.is_empty())))
}

fn parse_download(raw: &Value) -> Option<Value> {
    raw.as_object()?;
    let mut result = json!({});
    if matches!(raw["network"].as_str().map(str::to_lowercase).as_deref(), Some("xhttp" | "splithttp")) {
        result["network"] = json!("xhttp");
    }
    if nonblank(&raw["address"]) {
        result["server"] = raw["address"].clone();
    }
    if let Some(port) = integer(&raw["port"], false) {
        result["port"] = json!(port);
    }
    let security = raw["security"].as_str().map(str::to_lowercase).unwrap_or_default();
    if matches!(security.as_str(), "tls" | "reality") {
        result["tls"] = json!(true);
    }
    let tls = &raw["tlsSettings"];
    if tls.is_object() {
        for (wire, node) in [("serverName", "servername"), ("fingerprint", "client-fingerprint")] {
            if nonblank(&tls[wire]) {
                result[node] = tls[wire].clone();
            }
        }
        if valid_alpn(&tls["alpn"]) {
            result["alpn"] = tls["alpn"].clone();
        }
        if tls["allowInsecure"] == true {
            result["skip-cert-verify"] = json!(true);
        }
        if let Some(options) = ech_options(tls) {
            result["ech-opts"] = options;
        }
    }
    let reality = &raw["realitySettings"];
    if reality.is_object() {
        let mut options = json!({});
        for (wire, node) in [("publicKey", "public-key"), ("shortId", "short-id")] {
            if nonblank(&reality[wire]) {
                options[node] = reality[wire].clone();
            }
        }
        for (wire, node) in [("serverName", "servername"), ("fingerprint", "client-fingerprint")] {
            if nonblank(&reality[wire]) {
                result[node] = reality[wire].clone();
            }
        }
        if security == "reality" || options.as_object().is_some_and(|o| !o.is_empty()) {
            result["reality-opts"] = options;
        }
    }
    let settings = &raw["xhttpSettings"];
    if settings.is_object() {
        for key in ["path", "host", "mode"] {
            if nonblank(&settings[key]) {
                result[key] = settings[key].clone();
            }
        }
        apply_extra(&mut result, settings);
        apply_extra(&mut result, &settings["extra"]);
    }
    (!result.as_object().unwrap().is_empty()).then_some(result)
}

fn unsupported_download(raw: &Value) -> Option<Value> {
    if raw.is_null() {
        return None;
    }
    let Some(object) = raw.as_object() else {
        return Some(raw.clone());
    };
    let mut result = json!({});
    for (key, value) in object {
        let unsupported = match key.as_str() {
            "address" => (!nonblank(value)).then(|| value.clone()),
            "port" => integer(value, false).is_none().then(|| value.clone()),
            "security" => (!matches!(value.as_str().map(str::to_lowercase).as_deref(), Some("tls" | "reality"))).then(|| value.clone()),
            "network" => (!matches!(value.as_str().map(str::to_lowercase).as_deref(), Some("xhttp" | "splithttp"))).then(|| value.clone()),
            "tlsSettings" | "realitySettings" => {
                if let Some(fields) = value.as_object() {
                    let mut unsupported = json!({});
                    let valid_ech = ech_options(value).is_some();
                    for (field, entry) in fields {
                        let supported = if key == "tlsSettings" {
                            match field.as_str() {
                                "serverName" | "fingerprint" => nonblank(entry),
                                "echConfigList" => valid_ech,
                                "echForceQuery" => valid_ech && matches!(entry.as_str(), Some("none" | "half" | "full")),
                                "echSockopt" => valid_ech && entry.is_object(),
                                "alpn" => valid_alpn(entry),
                                "allowInsecure" => entry == true,
                                _ => false,
                            }
                        } else {
                            matches!(field.as_str(), "publicKey" | "shortId" | "serverName" | "fingerprint") && nonblank(entry)
                        };
                        if !supported {
                            unsupported[field] = entry.clone();
                        }
                    }
                    compact(unsupported)
                } else {
                    Some(value.clone())
                }
            }
            "xhttpSettings" => {
                if let Some(fields) = value.as_object() {
                    let mut inline = json!({});
                    let mut unsupported = json!({});
                    for (field, entry) in fields {
                        if ["path", "host", "mode"].contains(&field.as_str()) {
                            if !nonblank(entry) {
                                unsupported[field] = entry.clone();
                            }
                        } else if field != "extra" {
                            inline[field] = entry.clone();
                        }
                    }
                    if let Some(Value::Object(fields)) = unsupported_extra(&inline) {
                        for (field, entry) in fields {
                            unsupported[&field] = entry;
                        }
                    }
                    if let Some(entry) = fields.get("extra").and_then(unsupported_extra) {
                        unsupported["extra"] = entry;
                    }
                    compact(unsupported)
                } else {
                    Some(value.clone())
                }
            }
            _ => Some(value.clone()),
        };
        if let Some(value) = unsupported {
            result[key] = value;
        }
    }
    compact(result)
}

pub fn parse_extra(proxy: &mut Value, extra: &str, mode: &Value) {
    let parsed = codec::parse(extra);
    let raw = parsed.as_ref().cloned().unwrap_or_else(|_| json!({}));
    let mut options = proxy["xhttp-opts"].clone();
    if !options.is_object() {
        options = json!({});
    }
    if truthy(mode) {
        options["mode"] = mode.clone();
    }
    apply_extra(&mut options, &raw);
    if let Some(download) = parse_download(&raw["downloadSettings"]) {
        options["download-settings"] = download;
    }
    if options.as_object().is_some_and(|object| !object.is_empty()) {
        proxy["xhttp-opts"] = options;
    }
    if parsed.is_err() && !extra.is_empty() {
        proxy["_extra"] = json!(extra);
    }
    if let Some(object) = raw.as_object() {
        let mut inline = Value::Object(object.clone());
        inline.as_object_mut().unwrap().shift_remove("downloadSettings");
        let mut unsupported = unsupported_extra(&inline).unwrap_or_else(|| json!({}));
        if let Some(download) = object.get("downloadSettings").and_then(unsupported_download) {
            unsupported["downloadSettings"] = download;
        }
        if let Some(value) = compact(unsupported) {
            proxy["_extra_unsupported"] = value;
        }
    }
}

pub fn transport_host<'a>(network: &str, options: &'a Value) -> &'a Value {
    let values = match network {
        "h2" => [&options["host"], &options["headers"]["host"], &options["headers"]["Host"]],
        "xhttp" => [&options["host"], &options["headers"]["Host"], &options["headers"]["host"]],
        _ => [&options["headers"]["Host"], &options["headers"]["host"], &options["host"]],
    };
    values.into_iter().find(|value| !value.is_null()).unwrap_or(&options["host"])
}

fn emit_extra(target: &mut Value, options: &Value, nested: bool) {
    if let Some(value) = headers(&options["headers"], false, true) {
        target["headers"] = value;
    }
    for (wire, node, kind) in FIELDS {
        let raw = &options[*node];
        let value = match kind {
            Kind::StrictTextRange if *wire == "sessionIDLength" => range(raw, 1, 1, false),
            Kind::StrictRange | Kind::PositiveRange | Kind::NonNegativeRange => parsed_field(raw, *kind),
            Kind::AnyText => raw.as_str().map(|_| raw.clone()),
            Kind::True => (raw == true).then(|| raw.clone()),
            _ => truthy(raw).then(|| raw.clone()),
        };
        if let Some(value) = value {
            target[*wire] = value;
        }
    }
    if let Some(value) = reuse(&options["reuse-settings"], true) {
        if nested {
            target["extra"] = json!({"xmux":value});
        } else {
            target["xmux"] = value;
        }
    }
}

fn emit_download(raw: &Value, outer: &Value) -> Option<Value> {
    raw.as_object()?;
    let mut result = json!({});
    if truthy(&raw["server"]) {
        result["address"] = raw["server"].clone();
    }
    result["network"] = json!("xhttp");
    if let Some(port) = integer(&raw["port"], false) {
        result["port"] = json!(port);
    }
    let reality = raw["reality-opts"].is_object();
    if reality {
        result["security"] = json!("reality");
    } else if truthy(&raw["tls"]) {
        result["security"] = json!("tls");
    }
    let mut tls = json!({});
    for (node, wire) in [("servername", "serverName"), ("client-fingerprint", "fingerprint")] {
        if truthy(&raw[node]) {
            tls[wire] = raw[node].clone();
        }
    }
    if truthy(&raw["skip-cert-verify"]) {
        tls["allowInsecure"] = json!(true);
    }
    if truthy(&raw["alpn"]) {
        tls["alpn"] = if raw["alpn"].is_array() { raw["alpn"].clone() } else { json!([raw["alpn"]]) };
    }
    for (key, value) in ech_fields(&raw["ech-opts"], &Value::Null).as_object().unwrap() {
        tls[key] = value.clone();
    }
    if !tls.as_object().unwrap().is_empty() {
        result["tlsSettings"] = tls;
    }
    if reality {
        let mut fields = json!({});
        for (source, key) in [("servername", "serverName"), ("client-fingerprint", "fingerprint")] {
            if truthy(&raw[source]) {
                fields[key] = raw[source].clone();
            }
        }
        for (source, key) in [("public-key", "publicKey"), ("short-id", "shortId")] {
            if truthy(&raw["reality-opts"][source]) {
                fields[key] = raw["reality-opts"][source].clone();
            }
        }
        if !fields.as_object().unwrap().is_empty() {
            result["realitySettings"] = fields;
        }
    }
    let mut settings = json!({});
    for key in ["path", "host", "mode"] {
        let value = if key == "host" {
            let host = transport_host("xhttp", raw);
            if host.is_null() { transport_host("xhttp", outer) } else { host }
        } else if raw[key].is_null() {
            &outer[key]
        } else {
            &raw[key]
        };
        if truthy(value) {
            settings[key] = value.clone();
        }
    }
    emit_extra(&mut settings, raw, true);
    if !settings.as_object().unwrap().is_empty() {
        result["xhttpSettings"] = settings;
    }
    let explicit_network = matches!(raw["network"].as_str().map(str::to_lowercase).as_deref(), Some("xhttp" | "splithttp"));
    (explicit_network || result.as_object().unwrap().len() > 1).then_some(result)
}

fn merge_unsupported(base: &mut Value, unsupported: &Value) {
    let (Some(base), Some(unsupported)) = (base.as_object_mut(), unsupported.as_object()) else {
        return;
    };
    for (key, value) in unsupported {
        if let Some(existing) = base.get_mut(key) {
            if existing.is_null() {
                *existing = value.clone();
            } else {
                merge_unsupported(existing, value);
            }
        } else {
            base.insert(key.clone(), value.clone());
        }
    }
}

pub fn uri_extra(proxy: &Value) -> Option<String> {
    let explicit = &proxy["_extra"];
    if let Some(raw) = explicit.as_str() {
        return Some(raw.to_owned());
    }
    if explicit.is_object() {
        return Some(codec::stringify(explicit));
    }
    if proxy["network"] != "xhttp" {
        return truthy(explicit).then(|| codec::js_string(explicit));
    }
    let mut extra = json!({});
    let options = &proxy["xhttp-opts"];
    emit_extra(&mut extra, options, false);
    if let Some(download) = emit_download(&options["download-settings"], options) {
        extra["downloadSettings"] = download;
    }
    merge_unsupported(&mut extra, &proxy["_extra_unsupported"]);
    (!extra.as_object().unwrap().is_empty()).then(|| codec::stringify(&extra))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn structured_edits_win_while_unknown_uri_fields_survive() {
        let mut proxy = json!({"network":"xhttp"});
        parse_extra(
            &mut proxy,
            r#"{"sessionIDLength":"2-4","xmux":{"maxConnections":"2-2","future":7},"future":{"tag":"keep"},"downloadSettings":{"security":"reality","realitySettings":{},"xhttpSettings":{"path":"/"}}}"#,
            &Value::Null,
        );
        proxy["xhttp-opts"]["session-length"] = json!("5-8");
        let extra = codec::parse(&uri_extra(&proxy).expect("native XHTTP extra")).unwrap();
        assert_eq!(extra["sessionIDLength"], "5-8");
        assert_eq!(extra["xmux"], json!({"maxConnections":"2","future":7}));
        assert_eq!(extra["future"], json!({"tag":"keep"}));
        assert_eq!(extra["downloadSettings"]["security"], "reality");
    }
}
