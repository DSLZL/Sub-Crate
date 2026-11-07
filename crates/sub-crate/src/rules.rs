use crate::conversion::Error;
use crate::json as codec;
use serde_json::{Map, Value, json};

fn line_end(c: char) -> bool {
    matches!(c, '\r' | '\n' | '\u{2028}' | '\u{2029}')
}

fn preprocess(raw: &str) -> String {
    if raw.starts_with("<!DOCTYPE html>") {
        return String::new();
    }
    if !raw.split(line_end).any(|line| line.starts_with("payload:")) {
        return raw.to_owned();
    }
    let raw = raw.replacen("payload:", "", 1);
    let mut output = String::new();
    let mut remaining = raw.as_str();
    let mut start = true;
    while !remaining.is_empty() {
        if start {
            let whitespace = remaining.trim_start_matches(codec::is_space);
            if let Some(tail) = whitespace.strip_prefix('-') {
                remaining = tail.trim_start_matches(codec::is_space);
                start = tail
                    .strip_suffix(remaining)
                    .and_then(|consumed| consumed.chars().next_back())
                    .is_some_and(line_end);
                continue;
            }
        }
        let c = remaining.chars().next().unwrap();
        output.push(c);
        remaining = &remaining[c.len_utf8()..];
        start = line_end(c);
    }
    output
}

pub fn parse(raw: Option<&Value>) -> Result<Vec<Value>, Error> {
    let Some(text) = raw.and_then(Value::as_str) else {
        let message = if raw.is_none() {
            "Cannot read properties of undefined (reading 'split')"
        } else if raw.is_some_and(Value::is_null) {
            "Cannot read properties of null (reading 'split')"
        } else {
            "e.split is not a function"
        };
        return Err(Error::Legacy(message.to_owned()));
    };
    let raw = preprocess(text);
    let mut rules = Vec::new();
    for line in raw.split('\n').map(codec::trim) {
        if line.is_empty() || line.contains('#') {
            continue;
        }
        let fields: Vec<_> = line.split(',').map(codec::trim).collect();
        let raw_type = fields[0];
        let kind = match raw_type {
            "DOMAIN" | "host" | "HOST" => "DOMAIN",
            "DOMAIN-KEYWORD" | "host-keyword" | "HOST-KEYWORD" => "DOMAIN-KEYWORD",
            "DOMAIN-SUFFIX" | "host-suffix" | "HOST-SUFFIX" => "DOMAIN-SUFFIX",
            "PROCESS-NAME" => "PROCESS-NAME",
            "DEST-PORT" | "DST-PORT" => "DST-PORT",
            "SRC-IP" | "SRC-IP-CIDR" => "SRC-IP",
            "IN-PORT" | "SRC-PORT" => "IN-PORT",
            "PROTOCOL" => "PROTOCOL",
            "IP-CIDR6" | "ip6-cidr" | "IP6-CIDR" => "IP-CIDR6",
            value if value.eq_ignore_ascii_case("USER-AGENT") => "USER-AGENT",
            value if value.eq_ignore_ascii_case("IP-CIDR") => "IP-CIDR",
            value if value.eq_ignore_ascii_case("GEOIP") => "GEOIP",
            value if value.eq_ignore_ascii_case("GEOSITE") => "GEOSITE",
            _ => continue,
        };
        let mut rule = Map::new();
        rule.insert("type".to_owned(), json!(kind));
        if let Some(value) = fields.get(1) {
            rule.insert("content".to_owned(), json!(value));
        }
        if ["IP-CIDR", "IP-CIDR6", "GEOIP"].contains(&kind) {
            rule.insert("options".to_owned(), json!(&fields[2.min(fields.len())..]));
        }
        rules.push(Value::Object(rule));
    }
    Ok(rules)
}

pub fn produce(rules: &mut [Value], target: &str) -> Result<String, Error> {
    if !["QX", "Surge", "Loon", "Clash"].contains(&target) {
        return Err(Error::Legacy(format!(
            "Target platform: {target} is not supported!"
        )));
    }
    let mut output = Vec::new();
    for rule in rules {
        let kind = crate::conversion::field_text(rule, "type");
        let unsupported: &[&str] = match target {
            "QX" => &[
                "URL-REGEX",
                "DEST-PORT",
                "SRC-IP",
                "IN-PORT",
                "PROTOCOL",
                "GEOSITE",
                "GEOIP",
            ],
            "Surge" => &["GEOSITE", "GEOIP"],
            "Loon" => &["SRC-IP", "GEOSITE", "GEOIP"],
            _ => &[],
        };
        if unsupported.contains(&kind.as_str()) {
            return Err(Error::Legacy(
                "Cannot read properties of null (reading 'length')".to_owned(),
            ));
        }
        let name = match (target, kind.as_str()) {
            ("QX", "DOMAIN") => "HOST",
            ("QX", "DOMAIN-KEYWORD") => "HOST-KEYWORD",
            ("QX", "DOMAIN-SUFFIX") => "HOST-SUFFIX",
            ("QX", "IP-CIDR6") => "IP6-CIDR",
            ("Clash", "DEST-PORT") => "DST-PORT",
            ("Clash", "SRC-IP") => "SRC-IP-CIDR",
            ("Clash", "IN-PORT") => "SRC-PORT",
            _ => &kind,
        };
        let mut line = format!("{name},{}", crate::conversion::field_text(rule, "content"));
        if target == "QX" {
            line.push_str(",SUB-STORE");
        } else if (["IP-CIDR", "IP-CIDR6"].contains(&kind.as_str())
            || (target == "Clash" && kind == "GEOIP"))
            && let Some(options) = rule["options"].as_array()
        {
            let options: Vec<_> = options
                .iter()
                .filter(|value| target == "Surge" || *value == "no-resolve")
                .cloned()
                .collect();
            if !options.is_empty() {
                line.push(',');
                line.push_str(&codec::js_string(&json!(options)));
            }
            if target != "Surge" {
                rule["options"] = json!(options);
            }
        }
        output.push(line);
    }
    if target == "Clash" {
        crate::clash::Parser::new()?.dump_default(&json!({"payload":output}))
    } else {
        Ok(output.join("\n"))
    }
}
