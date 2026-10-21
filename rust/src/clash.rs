use crate::conversion::Error;
use crate::json as codec;
use crate::state::truthy;
use rquickjs::{Array, Context, Ctx, Exception, Function, Object, Runtime, Value as JsValue};
use serde_json::Value;
use std::cell::RefCell;

pub struct Parser {
    context: Context,
    preprocessed: RefCell<Option<(String, String)>>,
}

fn yaml<'js>(ctx: &Ctx<'js>, raw: &str) -> rquickjs::Result<JsValue<'js>> {
    let utilities: Object = ctx.globals().get("SubStoreUtilities")?;
    let parse: Function = utilities.get("parseYaml")?;
    let options = ctx.json_parse(r#"{"logLevel":"error","merge":true}"#)?;
    match parse.call((raw, options.clone())) {
        Ok(value) => Ok(value),
        Err(_) => {
            ctx.catch();
            let mut parts = raw.split("!<str>");
            let mut replaced = parts.next().unwrap_or_default().to_owned();
            for part in parts {
                replaced.push_str("__SubStoreJSYAMLString__");
                replaced.push_str(part.trim_start_matches(codec::is_space));
            }
            let value: JsValue = parse.call((replaced, options.clone()))?;
            let text = ctx
                .json_stringify(value)?
                .ok_or_else(|| Exception::throw_type(ctx, "Value is not JSON serializable"))?
                .to_string()?;
            let value = ctx.json_parse(text)?;
            let dump: Function = utilities.get("stringifyYaml")?;
            let text: String = dump.call((value,))?;
            parse.call((text.replace("__SubStoreJSYAMLString__", ""), options))
        }
    }
}

fn native_yaml(raw: &str) -> Option<Value> {
    use serde_saphyr::granit_parser::{Event, Parser, ScalarStyle};
    // Let the library tokenize YAML. Retain the original resolver for schema,
    // alias/tag and non-string-key cases whose JavaScript semantics differ.
    let mut parents: Vec<Option<bool>> = Vec::new();
    for item in Parser::new_from_str(raw) {
        match item.ok()?.0 {
            Event::DocumentStart(_, Some(_)) | Event::Alias(_) => return None,
            Event::MappingStart(_, anchor, tag) | Event::SequenceStart(_, anchor, tag)
                if anchor != 0 || tag.is_some() =>
            {
                return None;
            }
            event @ (Event::MappingStart(_, _, _) | Event::SequenceStart(_, _, _)) => {
                if let Some(Some(key)) = parents.last_mut() {
                    if *key {
                        return None;
                    }
                    *key = true;
                }
                // A map alternates scalar keys and values; a sequence has no keys.
                let mapping = matches!(event, Event::MappingStart(..));
                parents.push(mapping.then_some(true));
            }
            Event::MappingEnd | Event::SequenceEnd => {
                parents.pop()?;
            }
            Event::Scalar(text, style, anchor, tag) => {
                if anchor != 0 || tag.is_some() {
                    return None;
                }
                if style == ScalarStyle::Plain {
                    let word = text.as_ref();
                    let lower = word.to_ascii_lowercase();
                    let number = word.trim_start_matches(['+', '-']);
                    let numeric = number.starts_with(|c: char| c.is_ascii_digit());
                    if matches!(lower.as_str(), "true" | "false" | "null")
                        && ![
                            "true", "True", "TRUE", "false", "False", "FALSE", "null", "Null",
                            "NULL",
                        ]
                        .contains(&word)
                        || numeric
                            && (number.contains('_')
                                || number.strip_prefix('0').is_some_and(|rest| {
                                    rest.starts_with(|c: char| {
                                        c.is_ascii_digit()
                                            || matches!(c, 'x' | 'X' | 'o' | 'O' | 'b' | 'B')
                                    })
                                }))
                        || word == "<<"
                    {
                        return None;
                    }
                    if matches!(parents.last(), Some(Some(true)))
                        && (word.is_empty()
                            || matches!(lower.as_str(), "true" | "false" | "null" | "~")
                            || word.starts_with(|c: char| {
                                c.is_ascii_digit() || matches!(c, '+' | '-' | '.')
                            }))
                    {
                        return None;
                    }
                }
                if let Some(Some(key)) = parents.last_mut() {
                    *key = !*key;
                }
            }
            _ => {}
        }
    }
    serde_saphyr::from_str_with_options(
        raw,
        serde_saphyr::options! {
            strict_booleans: true,
            reject_unsupported_tags: true,
            non_finite_float_policy: serde_saphyr::NonFiniteFloatPolicy::Reject,
            with_snippet: false,
        },
    )
    .ok()
}

fn subscription_yaml<'js>(ctx: &Ctx<'js>, raw: &str) -> rquickjs::Result<JsValue<'js>> {
    match native_yaml(raw) {
        Some(value) => ctx.json_parse(codec::stringify(&value)),
        None => yaml(ctx, raw),
    }
}

pub(crate) fn short_ids(raw: &str) -> String {
    let mut output = String::new();
    let mut remaining = raw;
    while let Some((before, tail)) = remaining.split_once("short-id:") {
        output.push_str(before);
        let end = tail.find(['#', '\n', ',', '}']).unwrap_or(tail.len());
        let value = codec::trim(&tail[..end]);
        output.push_str("short-id: ");
        if value == "null"
            || (value.len() >= 2
                && !value.contains(['\r', '\n', '\u{2028}', '\u{2029}'])
                && ((value.starts_with('"') && value.ends_with('"'))
                    || (value.starts_with('\'') && value.ends_with('\''))))
        {
            output.push_str(value);
        } else {
            output.push('"');
            output.push_str(value);
            output.push('"');
        }
        remaining = &tail[end..];
    }
    output.push_str(remaining);
    output
}

impl Parser {
    pub fn new() -> Result<Self, Error> {
        let runtime = Runtime::new().map_err(|error| Error::Legacy(error.to_string()))?;
        let context = Context::full(&runtime).map_err(|error| Error::Legacy(error.to_string()))?;
        context.with(|ctx| {
            crate::js_values::initialize(&ctx).map_err(|error| Error::Legacy(error.to_string()))?;
            ctx.eval::<(), _>(include_bytes!("../vendor/utilities.js").as_slice())
                .map_err(|error| Error::Legacy(error.to_string()))
        })?;
        Ok(Self {
            context,
            preprocessed: RefCell::new(None),
        })
    }

    pub fn dump(&self, value: &Value) -> Result<String, Error> {
        self.dump_with_options(value, false)
    }

    pub fn dump_default(&self, value: &Value) -> Result<String, Error> {
        self.dump_with_options(value, true)
    }

    pub fn set_path(&self, value: &mut Value, path: &str, item: &Value) -> Result<(), Error> {
        let parts: Vec<_> = path
            .split(['.', '[', ']'])
            .filter(|part| !part.is_empty())
            .collect();
        if parts
            .iter()
            .any(|part| ["__proto__", "constructor", "prototype"].contains(part))
        {
            return Err(Error::Legacy("Unsafe property path".to_owned()));
        }
        self.context.with(|ctx| {
            let result = (|| -> rquickjs::Result<Value> {
                let root = crate::js_values::decode(&ctx, value)?;
                let child: Function = ctx.eval("(object,key) => Object.hasOwn(object,key) && Object(object[key]) === object[key] ? object[key] : undefined")?;
                let array_key: Function = ctx.eval("key => (Math.abs(key) >> 0) === +key")?;
                let mut current = root.clone();
                for (index, key) in parts
                    .iter()
                    .take(parts.len().saturating_sub(1))
                    .enumerate()
                {
                    let mut next: JsValue = child.call((current.clone(), *key))?;
                    if !next.is_object() {
                        let is_array: bool = array_key.call((parts[index + 1],))?;
                        next = if is_array {
                            Array::new(ctx.clone())?.into_value()
                        } else {
                            Object::new(ctx.clone())?.into_value()
                        };
                        current.as_object().unwrap().set(*key, next.clone())?;
                    }
                    current = next;
                }
                if let Some(key) = parts.last() {
                    current
                        .as_object()
                        .unwrap()
                        .set(*key, crate::js_values::decode(&ctx, item)?)?;
                }
                crate::js_values::project(&ctx, root)
            })();
            *value = result.map_err(|error| {
                ctx.catch();
                Error::Legacy(error.to_string())
            })?;
            Ok(())
        })
    }

    fn dump_with_options(&self, value: &Value, defaults: bool) -> Result<String, Error> {
        self.context.with(|ctx| {
            let result = (|| -> rquickjs::Result<String> {
                let utilities: Object = ctx.globals().get("SubStoreUtilities")?;
                let dump: Function = utilities.get("stringifyYaml")?;
                let content = crate::js_values::decode(&ctx, value)?;
                let text: String = if defaults {
                    dump.call((content,))?
                } else {
                    let options = ctx.json_parse(r#"{"lineWidth":0}"#)?;
                    dump.call((content, options))?
                };
                Ok(
                    if text.contains("proxies:")
                        && text.contains("short-id:")
                        && value["proxies"]
                            .as_array()
                            .is_some_and(|proxies| !proxies.is_empty())
                    {
                        short_ids(&text)
                    } else {
                        text
                    },
                )
            })();
            result.map_err(|error| {
                ctx.catch();
                Error::Legacy(error.to_string())
            })
        })
    }

    pub fn preprocess(&self, raw: &str) -> Option<String> {
        if !raw.contains("proxies") {
            return None;
        }
        if let Some((source, output)) = self.preprocessed.borrow().as_ref()
            && source == raw
        {
            return Some(output.clone());
        }
        self.context.with(|ctx| {
            let result = (|| -> rquickjs::Result<Option<String>> {
                // Downloads serialize YAML entries as canonical JSON; retain YAML for other syntax.
                if let Some(entries) = raw
                    .strip_prefix("proxies:\n")
                    .filter(|entries| !entries.contains("short-id:"))
                {
                    let mut output = String::new();
                    let mut canonical = true;
                    for line in entries.split_terminator('\n') {
                        let Some(line) = line.strip_prefix("  - ") else {
                            canonical = false;
                            break;
                        };
                        // YAML handles Unicode escapes and line separators differently.
                        if line.contains("\\u")
                            || line.chars().any(|c| {
                                matches!(
                                    c,
                                    '\u{7f}'
                                        ..='\u{9f}'
                                            | '\u{2028}'
                                            | '\u{2029}'
                                            | '\u{fffe}'
                                            | '\u{ffff}'
                                )
                            })
                        {
                            canonical = false;
                            break;
                        }
                        let value = match ctx.json_parse(line) {
                            Ok(value) => value,
                            Err(_) => {
                                ctx.catch();
                                canonical = false;
                                break;
                            }
                        };
                        let text = ctx
                            .json_stringify(value)?
                            .map(|text| text.to_string())
                            .transpose()?;
                        if text.as_deref() != Some(line) {
                            canonical = false;
                            break;
                        }
                        output.push_str(line);
                        output.push('\n');
                    }
                    if canonical && !output.is_empty() {
                        return Ok(Some(output));
                    }
                }
                let value = subscription_yaml(&ctx, raw)?;
                let Some(root) = value.as_object() else {
                    return Ok(None);
                };
                let proxies: JsValue = root.get("proxies")?;
                let groups: JsValue = root.get("proxy-groups")?;
                if !proxies.is_array() && !groups.is_array() {
                    return Ok(None);
                }
                let value = if raw.contains("proxies:")
                    && raw.contains("short-id:")
                    && proxies.as_array().is_some_and(|array| !array.is_empty())
                {
                    subscription_yaml(&ctx, &short_ids(raw))?
                } else {
                    value
                };
                let Some(root) = value.as_object() else {
                    return Ok(None);
                };
                let proxies: JsValue = root.get("proxies")?;
                let mut output = String::new();
                if let Some(array) = proxies.as_array() {
                    for value in array.iter::<JsValue>() {
                        let text = ctx
                            .json_stringify(value?)?
                            .map(|text| text.to_string())
                            .transpose()?
                            .unwrap_or_else(|| "undefined".to_owned());
                        output.push_str(&text);
                        output.push('\n');
                    }
                }
                // Keep only the last successful generic normalization. Conversion
                // still parses fresh nodes and reads external CA files each time.
                *self.preprocessed.borrow_mut() = Some((raw.to_owned(), output.clone()));
                Ok(Some(output))
            })();
            match result {
                Ok(value) => value,
                Err(_) => {
                    ctx.catch();
                    None
                }
            }
        })
    }

    pub fn line(&self, raw: &str) -> Result<Option<Value>, Error> {
        self.context.with(|ctx| {
            let result = (|| -> rquickjs::Result<Option<Value>> {
                let value: JsValue = match ctx.json_parse(raw) {
                    Ok(value) => value,
                    Err(_) => {
                        ctx.catch();
                        let utilities: Object = ctx.globals().get("SubStoreUtilities")?;
                        let json5: Object = utilities.get("JSON5")?;
                        let parse: Function = json5.get("parse")?;
                        match parse.call((raw,)) {
                            Ok(value) => value,
                            Err(_) => {
                                ctx.catch();
                                yaml(&ctx, raw)?
                            }
                        }
                    }
                };
                let Some(object) = value.as_object() else {
                    return Ok(None);
                };
                let kind: JsValue = object.get("type")?;
                let kind = kind.as_string().and_then(|kind| kind.to_string().ok());
                if !kind.as_deref().is_some_and(|kind| {
                    [
                        "zerotier",
                        "shadowquic",
                        "gost-relay",
                        "openvpn",
                        "tailscale",
                        "easytier",
                        "trusttunnel",
                        "h2-connect",
                        "naive",
                        "anytls",
                        "mieru",
                        "masque",
                        "masque-surge",
                        "sudoku",
                        "juicity",
                        "ss",
                        "ssr",
                        "vmess",
                        "socks5",
                        "http",
                        "snell",
                        "trojan",
                        "tuic",
                        "vless",
                        "hysteria",
                        "hysteria2",
                        "wireguard",
                        "ssh",
                        "direct",
                    ]
                    .contains(&kind)
                }) {
                    return Ok(None);
                }
                crate::js_values::project(&ctx, value).map(Some)
            })();
            let Some(mut proxy) = (match result {
                Ok(value) => value,
                Err(_) => {
                    ctx.catch();
                    return Ok(None);
                }
            }) else {
                return Ok(None);
            };
            if matches!(proxy["type"].as_str(), Some("vmess" | "vless"))
                && truthy(&proxy["servername"])
            {
                proxy["sni"] = proxy["servername"].clone();
                proxy.as_object_mut().unwrap().shift_remove("servername");
            }
            for (from, to) in [
                ("server-cert-fingerprint", "tls-fingerprint"),
                ("fingerprint", "tls-fingerprint"),
                ("dialer-proxy", "underlying-proxy"),
                ("benchmark-url", "test-url"),
                ("benchmark-timeout", "test-timeout"),
            ] {
                if truthy(&proxy[from]) {
                    proxy[to] = proxy[from].clone();
                }
            }
            if proxy["type"] == "vmess" {
                proxy["cipher"] = crate::conversion::vmess_security(&proxy["cipher"]);
            }
            Ok(Some(proxy))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn legacy_line(parser: &Parser, raw: &str) -> Value {
        parser.context.with(|ctx| {
            let utilities: Object = ctx.globals().get("SubStoreUtilities").unwrap();
            let json5: Object = utilities.get("JSON5").unwrap();
            let parse: Function = json5.get("parse").unwrap();
            let value: JsValue = match parse.call((raw,)) {
                Ok(value) => value,
                Err(_) => {
                    ctx.catch();
                    yaml(&ctx, raw).unwrap()
                }
            };
            crate::js_values::project(&ctx, value).unwrap()
        })
    }

    #[test]
    fn strict_json_and_fallbacks_retain_ecmascript_values() {
        let parser = Parser::new().ok().unwrap();
        for raw in [
            r#"{"type":"ss","extra":{"n":9007199254740993,"zero":-0,"overflow":1e400,"key":null,"2":"two","1":"one","false":false,"empty":[]},"password":0}"#,
            r#"{"type":"ss","name":"\ud800\ue000\udc00","extra":{"__proto__":{"safe":true},"constructor":0,"literal":"😀"}}"#,
            r#"{type:'ss',port:0x1bb,extra:{nan:NaN,positive:Infinity,negative:-Infinity,zero:-0,},}"#,
            "{type: ss, name: yaml, extra: &loop {self: *loop}}",
        ] {
            assert_eq!(
                parser.line(raw).ok().unwrap().unwrap(),
                legacy_line(&parser, raw)
            );
        }
        for raw in ["not a node", "{", r#"{"type":"unknown"}"#, "null"] {
            assert!(parser.line(raw).ok().unwrap().is_none());
        }
        assert_eq!(
            parser.line(r#"{"type":"ss"}"#).ok().unwrap(),
            Some(json!({"type":"ss"}))
        );
    }

    #[test]
    fn yaml_preprocess_keeps_short_ids_aliases_and_unknown_fields() {
        let parser = Parser::new().ok().unwrap();
        let raw = "proxies:\n  - {type: vless, name: yaml, server: 127.0.0.1, reality-opts: {short-id: 00123}, extra: &meta {enabled: false}}\n  - {type: ss, name: alias, extra: *meta}\n";
        let lines = parser.preprocess(raw).unwrap();
        let proxies: Vec<_> = lines
            .lines()
            .map(|line| parser.line(line).ok().unwrap().unwrap())
            .collect();
        assert_eq!(proxies[0]["reality-opts"]["short-id"], "00123");
        assert_eq!(proxies[0]["extra"], json!({"enabled":false}));
        assert_eq!(proxies[1]["extra"], proxies[0]["extra"]);
        let mut changed = lines.clone();
        changed.push_str("caller mutation");
        assert_ne!(changed, lines);
        assert_eq!(parser.preprocess(raw), Some(lines));
        assert!(parser.preprocess("proxies: not-an-array").is_none());
        let literal = "proxies:\n  - {\"type\":\"ss\",\"extra\":\"short-id: raw\"}\n";
        assert!(parser.preprocess(literal).is_none());
    }

    #[test]
    fn native_subscription_yaml_matches_locked_js_schema_and_fallbacks() {
        fn render<'js>(ctx: &Ctx<'js>, result: rquickjs::Result<JsValue<'js>>) -> Option<String> {
            match result.and_then(|value| ctx.json_stringify(value)) {
                Ok(value) => value.map(|value| value.to_string().unwrap()),
                Err(_) => {
                    ctx.catch();
                    None
                }
            }
        }
        let parser = Parser::new().ok().unwrap();
        let mut documents = Vec::new();
        for scalar in [
            "0",
            "-0",
            "-0.0",
            "01",
            "0123",
            "0x1F",
            "0X1F",
            "+0x1F",
            "-0x1f",
            "0o17",
            "0b10",
            "1_000",
            "01_000",
            "1.",
            ".5",
            "+.5",
            "1E3",
            "1e999",
            ".inf",
            ".nan",
            "null",
            "Null",
            "NULL",
            "nUlL",
            "true",
            "True",
            "TRUE",
            "TrUe",
            "false",
            "FaLsE",
            "yes",
            "ON",
            "2020-01-01",
            "12:34",
            "~",
            "",
            "'literal'",
            r#""\N\L\P""#,
            r#""\ud800""#,
            "!!str 123",
            "!unknown 123",
            "[false, null, 1, \"1\", {nested: value}]",
            "{1.0: first, 0x10: second}",
            "{true: first, null: second}",
            "{\"<<\": {x: 1}}",
            "{x: 1, x: 2}",
        ] {
            documents.push(format!(
                "proxies:\n  - {{type: ss, server: example.invalid, extra: {scalar}}}\n"
            ));
        }
        documents.extend([
            "%YAML 1.1\n---\nproxies: [{type: ss, extra: ON}]\n".to_owned(),
            "proxies: [{type: ss, extra: &value {x: 1}}, {type: ss, extra: *value}]".to_owned(),
            "defaults: &value {server: example.invalid}\nproxies: [{<<: *value, type: ss}]".to_owned(),
            "proxies: [{type: ss, extra: &loop {self: *loop}}]".to_owned(),
            "proxies: [{type: ss}]\n---\nproxies: []".to_owned(),
            "proxies:\n  - type: ss\n    name: 中文😀\n    server: 127.0.0.1\n    port: 443\n    extra: |\n      original\n      multiline\n".to_owned(),
        ]);
        parser.context.with(|ctx| {
            for raw in documents {
                let expected = render(&ctx, yaml(&ctx, &raw));
                let actual = render(&ctx, subscription_yaml(&ctx, &raw));
                assert_eq!(actual, expected, "{raw}");
            }
        });
    }

    #[test]
    fn native_subscription_yaml_handles_normal_flow_and_block_documents() {
        for raw in [
            "proxies: [{type: ss, server: 127.0.0.1, port: 443, udp: true, extra: [yes, null]}]",
            "proxies:\n  - type: ss\n    name: 中文😀\n    password: 'private-fixture'\n    server: example.invalid\n    port: 443\n    udp: false\n",
        ] {
            let value = native_yaml(raw).unwrap();
            assert_eq!(value["proxies"][0]["type"], "ss");
            assert_eq!(value["proxies"][0]["port"], 443);
        }
        for extra in [
            "00123", "0b10", "1_000", "nUlL", "TrUe", "&a value", "!!str 1",
        ] {
            assert!(native_yaml(&format!("proxies: [{{extra: {extra}}}] ")).is_none());
        }
    }

    #[test]
    fn canonical_download_entries_match_yaml_and_reject_duplicate_keys() {
        let parser = Parser::new().ok().unwrap();
        for line in [
            r#"{"type":"ss","name":"json","port":443,"extra":{"__proto__":{"safe":true},"constructor":0,"<<":{"a":1},"flags":[false,null,0]}}"#.to_owned(),
            r#"{"type":"ss","name":"\ud800\ue000\udc00","extra":{"n":1e400,"zero":-0}}"#.to_owned(),
            r#"{"type":"ss","name":"first","name":"last"}"#.to_owned(),
            r#"{type: ss, name: yaml, extra: &meta {enabled: false}}"#.to_owned(),
            json!({"type":"ss","name":"unicode","extra":"\u{7f}\u{80}\u{85}\u{2028}\u{2029}\u{fffe}\u{ffff}😀"}).to_string(),
            json!({"type":"ss","extra":{"1":"one","2":"two","large":9007199254740993_u64}}).to_string(),
        ] {
            let raw = format!("proxies:\n  - {line}\n");
            let expected = parser.context.with(|ctx| match yaml(&ctx, &raw) {
                Ok(value) => ctx.json_stringify(value).unwrap().unwrap().to_string().ok(),
                Err(_) => {
                    ctx.catch();
                    None
                }
            });
            for _ in 0..2 {
                let actual = parser.preprocess(&raw).map(|lines| {
                    format!("{{\"proxies\":[{}]}}", lines.lines().collect::<Vec<_>>().join(","))
                });
                assert_eq!(actual, expected);
            }
        }
        assert!(parser.preprocess("proxies:\n").is_none());
    }

    #[test]
    #[ignore = "release profile in GitHub Actions"]
    fn profile_structured_subscription() {
        use std::{hint::black_box, time::Instant};
        let raw = r#"{"type":"ss","name":"profile","server":"127.0.0.1","port":443,"cipher":"aes-128-gcm","password":"public-fixture","udp":true,"extra":{"false":false,"nil":null}}"#;
        let began = Instant::now();
        for _ in 0..23 {
            black_box(Parser::new().ok().unwrap());
        }
        let setup = began.elapsed().as_secs_f64();
        let parser = Parser::new().ok().unwrap();
        let expected = legacy_line(&parser, raw);
        let began = Instant::now();
        for _ in 0..5524 {
            assert_eq!(black_box(legacy_line(&parser, raw)), expected);
        }
        let json5 = began.elapsed().as_secs_f64();
        let began = Instant::now();
        for _ in 0..5524 {
            assert_eq!(black_box(parser.line(raw).ok().unwrap().unwrap()), expected);
        }
        let strict = began.elapsed().as_secs_f64();
        let yaml = format!("proxies:\n{}", format!("  - {raw}\n").repeat(5524));
        let began = Instant::now();
        parser.context.with(|ctx| {
            let value = super::yaml(&ctx, &yaml).unwrap();
            let proxies: Array = value.as_object().unwrap().get("proxies").unwrap();
            assert_eq!(proxies.len(), 5524);
        });
        let generic_yaml = began.elapsed().as_secs_f64();
        let began = Instant::now();
        assert_eq!(parser.preprocess(&yaml).unwrap().lines().count(), 5524);
        let yaml = began.elapsed().as_secs_f64();
        let raw_yaml = format!("proxies:\n{}", "  - {type: ss, name: profile, server: example.invalid, port: 443, cipher: aes-128-gcm, password: public-fixture, udp: true}\n".repeat(5524));
        let began = Instant::now();
        let expected = parser.context.with(|ctx| {
            let value = super::yaml(&ctx, &raw_yaml).unwrap();
            ctx.json_stringify(value)
                .unwrap()
                .unwrap()
                .to_string()
                .unwrap()
        });
        let interpreted_raw = began.elapsed().as_secs_f64();
        let began = Instant::now();
        assert_eq!(
            native_yaml(&raw_yaml).unwrap()["proxies"]
                .as_array()
                .unwrap()
                .len(),
            5524
        );
        let actual = parser.context.with(|ctx| {
            let value = subscription_yaml(&ctx, &raw_yaml).unwrap();
            ctx.json_stringify(value)
                .unwrap()
                .unwrap()
                .to_string()
                .unwrap()
        });
        let native_raw_two_passes = began.elapsed().as_secs_f64();
        assert_eq!(actual, expected);
        println!(
            "parse profile: setup_23_s={setup:.6}, json5_5524_s={json5:.6}, strict_5524_s={strict:.6}, generic_yaml_5524_s={generic_yaml:.6}, yaml_5524_s={yaml:.6}, interpreted_raw_5524_s={interpreted_raw:.6}, native_raw_two_passes_5524_s={native_raw_two_passes:.6}"
        );
    }
}
