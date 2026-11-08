use crate::conversion::Error;
use crate::json as codec;
use crate::state::truthy;
use rquickjs::{
    Array, Context, Ctx, Exception, Function, Object, Runtime, Value as JsValue,
    context::{Intrinsic, intrinsic},
};
use serde_json::Value;
use std::borrow::Cow;
use std::cell::{OnceCell, RefCell};
use std::collections::HashMap;
use std::ops::Range;

pub struct Parser {
    context: OnceCell<Context>,
    preprocessed: RefCell<Option<(String, String)>>,
}

fn utilities<'js>(ctx: &Ctx<'js>) -> rquickjs::Result<Object<'js>> {
    let globals = ctx.globals();
    let value: JsValue = globals.get("SubStoreUtilities")?;
    if let Some(utilities) = value.as_object() {
        return Ok(utilities.clone());
    }
    let _setup = crate::profile::span("parse_utilities_init");
    let regexp: JsValue = globals.get("RegExp")?;
    if regexp.is_undefined() {
        // SAFETY: This private parser context has no caller scripts. Add each
        // omitted intrinsic once, retaining the existing JSON/MapSet objects.
        unsafe {
            <(
                intrinsic::Date,
                intrinsic::RegExpCompiler,
                intrinsic::RegExp,
                intrinsic::Proxy,
                intrinsic::TypedArrays,
                intrinsic::Promise,
                intrinsic::Performance,
                intrinsic::WeakRef,
                intrinsic::DOMException,
            )>::add_intrinsic(ctx.as_raw());
        }
    }
    crate::script::bytecode::UTILITIES.eval(ctx)?;
    globals.get("SubStoreUtilities")
}

fn yaml<'js>(ctx: &Ctx<'js>, raw: &str) -> rquickjs::Result<JsValue<'js>> {
    let utilities = utilities(ctx)?;
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
                    let number = word.strip_prefix(['+', '-']).unwrap_or(word);
                    let numeric = number.starts_with(|c: char| c.is_ascii_digit());
                    let decimal = number
                        .bytes()
                        .all(|c| c.is_ascii_digit() || b"._eE+-".contains(&c));
                    let radix = ["0x", "0X", "0o", "0O", "0b", "0B"].iter().any(|prefix| {
                        number.strip_prefix(*prefix).is_some_and(|digits| {
                            !digits.is_empty()
                                && digits.bytes().all(|c| {
                                    c == b'_'
                                        || match *prefix {
                                            "0x" | "0X" => c.is_ascii_hexdigit(),
                                            "0o" | "0O" => matches!(c, b'0'..=b'7'),
                                            _ => matches!(c, b'0' | b'1'),
                                        }
                                })
                        })
                    });
                    if matches!(lower.as_str(), "true" | "false" | "null")
                        && ![
                            "true", "True", "TRUE", "false", "False", "FALSE", "null", "Null",
                            "NULL",
                        ]
                        .contains(&word)
                        || radix
                        || numeric
                            && decimal
                            && (number.contains('_')
                                || number.strip_prefix('0').is_some_and(|rest| {
                                    rest.starts_with(|c: char| c.is_ascii_digit())
                                }))
                        || word == "<<"
                    {
                        return None;
                    }
                    if matches!(parents.last(), Some(Some(true)))
                        && (word.is_empty()
                            || matches!(lower.as_str(), "true" | "false" | "null" | "~")
                            || word.parse::<f64>().is_ok())
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

/// Whether a plain YAML scalar loads as something other than a string or null.
// shortcut: %YAML 1.1 yes/no, 1_000, 1:20 and dates stay strings here because no hex short-id
// looks like them; add them if non-hex short-ids ever matter.
fn non_string(word: &str, yaml_1_1: bool) -> bool {
    fn digits(text: Option<&str>, radix: u32) -> bool {
        text.is_some_and(|text| !text.is_empty() && text.chars().all(|c| c.is_digit(radix)))
    }
    let unsigned = word.strip_prefix(['+', '-']).unwrap_or(word);
    let exponent = unsigned.strip_prefix(['e', 'E']);
    matches!(
        word,
        "true" | "True" | "TRUE" | "false" | "False" | "FALSE" | ".nan" | ".NaN" | ".NAN"
    ) || matches!(unsigned, ".inf" | ".Inf" | ".INF")
        || digits(word.strip_prefix("0x"), 16)
        || (!yaml_1_1 && digits(word.strip_prefix("0o"), 8))
        // Rust's float grammar is exactly the core schema's int, float and exponent patterns.
        || (word.bytes().all(|c| c.is_ascii_digit() || b"+-.eE".contains(&c))
            && word.parse::<f64>().is_ok())
        || (yaml_1_1
            && (digits(unsigned.strip_prefix("0b"), 2)
                || digits(exponent.map(|e| e.strip_prefix(['+', '-']).unwrap_or(e)), 10)))
}

/// A YAML node; maps list their keys and values alternately.
enum Node<'a> {
    /// The text and, when it would not load as a string, the byte range to quote.
    Scalar(Cow<'a, str>, Option<Range<usize>>),
    Alias(usize),
    Seq(Vec<usize>),
    Map(Vec<usize>),
}

/// Byte ranges of plain REALITY short-ids that a YAML load reads as numbers or booleans.
/// Like upstream, a short-id counts anywhere under `reality-opts` or `reality-config`,
/// including nodes those reach through aliases and merges.
fn unquoted_short_ids(raw: &str) -> Option<Vec<Range<usize>>> {
    use serde_saphyr::granit_parser::{Event, Parser, ScalarStyle};
    // The JS loader retries legacy `!<str>value`; `!!str ` has the same length, so spans match raw.
    let source = if raw.contains("!<str>") {
        Cow::Owned(raw.replace("!<str>", "!!str "))
    } else {
        Cow::Borrowed(raw)
    };
    let mut nodes: Vec<Node> = Vec::new();
    let mut open: Vec<usize> = Vec::new();
    let mut anchors: HashMap<usize, usize> = HashMap::new();
    let (mut documents, mut yaml_1_1) = (0, false);
    for item in Parser::new_from_str(&source) {
        let (event, span) = item.ok()?;
        let (node, anchor) = match event {
            Event::DocumentStart(_, version) => {
                documents += 1;
                yaml_1_1 = version.is_some_and(|v| (v.major, v.minor) == (1, 1));
                continue;
            }
            Event::Alias(anchor) => (Node::Alias(*anchors.get(&anchor)?), 0),
            Event::Scalar(text, style, anchor, tag) => {
                let quote = span.byte_range().filter(|_| {
                    style == ScalarStyle::Plain && tag.is_none() && non_string(&text, yaml_1_1)
                });
                (Node::Scalar(text, quote), anchor)
            }
            Event::SequenceStart(_, anchor, _) => (Node::Seq(Vec::new()), anchor),
            Event::MappingStart(_, anchor, _) => (Node::Map(Vec::new()), anchor),
            Event::SequenceEnd | Event::MappingEnd => {
                open.pop();
                continue;
            }
            _ => continue,
        };
        let index = nodes.len();
        if let Some(&parent) = open.last()
            && let Node::Seq(children) | Node::Map(children) = &mut nodes[parent]
        {
            children.push(index);
        }
        if anchor != 0 {
            anchors.insert(anchor, index);
        }
        if matches!(node, Node::Seq(_) | Node::Map(_)) {
            open.push(index);
        }
        nodes.push(node);
    }
    if documents != 1 || nodes.is_empty() {
        return None;
    }
    let target = |index: usize| match nodes[index] {
        Node::Alias(target) => target,
        _ => index,
    };
    let named = |index: usize, names: &[&str]| matches!(&nodes[target(index)], Node::Scalar(text, _) if names.contains(&&**text));
    let mut pending: Vec<usize> = nodes
        .iter()
        .filter_map(|node| match node {
            Node::Map(children) => Some(children.as_chunks::<2>().0.iter()),
            _ => None,
        })
        .flatten()
        .filter(|pair| named(pair[0], &["reality-opts", "reality-config"]))
        .map(|pair| pair[1])
        .collect();
    // Aliases and merges inside a REALITY config bring their targets into it.
    let mut reality = vec![false; nodes.len()];
    while let Some(config) = pending.pop() {
        if std::mem::replace(&mut reality[config], true) {
            continue;
        }
        let mut walk = vec![config];
        while let Some(index) = walk.pop() {
            match &nodes[index] {
                Node::Alias(target) => pending.push(*target),
                Node::Seq(children) | Node::Map(children) => walk.extend(children),
                Node::Scalar(..) => {}
            }
        }
    }
    let mut values = Vec::new();
    let mut walk = vec![(0, false)];
    while let Some((index, outer)) = walk.pop() {
        let inside = outer || reality[index];
        if let Node::Seq(children) | Node::Map(children) = &nodes[index] {
            walk.extend(children.iter().map(|&child| (child, inside)));
        }
        if inside && let Node::Map(children) = &nodes[index] {
            values.extend(
                children
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .filter(|pair| named(pair[0], &["short-id"]))
                    .map(|pair| pair[1]),
            );
        }
    }
    // A short-id list counts item by item.
    let mut seen = vec![false; nodes.len()];
    let mut ranges = Vec::new();
    while let Some(value) = values.pop() {
        let node = target(value);
        if std::mem::replace(&mut seen[node], true) {
            continue;
        }
        match &nodes[node] {
            Node::Seq(items) => values.extend(items),
            Node::Scalar(_, Some(range)) => ranges.push(range.clone()),
            _ => {}
        }
    }
    ranges.sort_unstable_by_key(|range| range.start);
    Some(ranges)
}

/// Quote REALITY short-ids such as `0088` that a later YAML load would read as numbers.
/// Returns `None` when nothing changes.
pub(crate) fn short_ids(raw: &str) -> Option<String> {
    if !raw.contains("short-id") {
        return None;
    }
    let ranges = unquoted_short_ids(raw).filter(|ranges| !ranges.is_empty())?;
    let mut output = String::with_capacity(raw.len() + 2 * ranges.len());
    let mut offset = 0;
    for range in ranges {
        // These scalars are one ASCII word, so JSON quoting only adds the quotes.
        output.push_str(&raw[offset..range.start]);
        output.push('"');
        output.push_str(&raw[range.clone()]);
        output.push('"');
        offset = range.end;
    }
    output.push_str(&raw[offset..]);
    Some(output)
}

impl Parser {
    pub(crate) fn clear_request(&self) {
        self.preprocessed.borrow_mut().take();
    }

    pub fn new() -> Result<Self, Error> {
        Ok(Self {
            context: OnceCell::new(),
            preprocessed: RefCell::new(None),
        })
    }

    fn context(&self) -> Result<&Context, Error> {
        if let Some(context) = self.context.get() {
            return Ok(context);
        }
        // Native URI paths do not need JSON5/YAML or a second JS runtime.
        let _setup = crate::profile::span("parse_context_init");
        let runtime = Runtime::new().map_err(|error| Error::Legacy(error.to_string()))?;
        let context =
            Context::custom::<(intrinsic::Eval, intrinsic::Json, intrinsic::MapSet)>(&runtime)
                .map_err(|error| Error::Legacy(error.to_string()))?;
        context.with(|ctx| {
            crate::js_values::initialize(&ctx).map_err(|error| Error::Legacy(error.to_string()))
        })?;
        Ok(self.context.get_or_init(|| context))
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
        self.context()?.with(|ctx| {
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
        self.context()?.with(|ctx| {
            let result = (|| -> rquickjs::Result<String> {
                let utilities = utilities(&ctx)?;
                let dump: Function = utilities.get("stringifyYaml")?;
                let content = crate::js_values::decode(&ctx, value)?;
                let text: String = if defaults {
                    dump.call((content,))?
                } else {
                    let options = ctx.json_parse(r#"{"lineWidth":0}"#)?;
                    dump.call((content, options))?
                };
                Ok(short_ids(&text).unwrap_or(text))
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
        self.context().ok()?.with(|ctx| {
            let result = (|| -> rquickjs::Result<Option<String>> {
                // Downloads serialize YAML entries as canonical JSON; retain YAML for other syntax.
                if let Some(entries) = raw.strip_prefix("proxies:\n").filter(|entries| {
                    // Only string short-ids can skip the YAML normalization.
                    entries
                        .match_indices("short-id")
                        .all(|(at, key)| entries[at + key.len()..].starts_with("\":\""))
                }) {
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
                let normalized = proxies
                    .as_array()
                    .is_some_and(|array| !array.is_empty())
                    .then(|| short_ids(raw))
                    .flatten();
                let value = match normalized {
                    Some(raw) => subscription_yaml(&ctx, &raw)?,
                    None => value,
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
        // JSON5 nodes require an object delimiter; a confirmed YAML string cannot be a node.
        if !raw.contains('{') {
            let _probe = crate::profile::span("parse_scalar_probe");
            if matches!(native_yaml(raw), Some(Value::String(_))) {
                return Ok(None);
            }
        }
        self.context()?.with(|ctx| {
            let result = (|| -> rquickjs::Result<Option<Value>> {
                let value: JsValue = match ctx.json_parse(raw) {
                    Ok(value) => value,
                    Err(_) => {
                        ctx.catch();
                        let utilities = utilities(&ctx)?;
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

    #[test]
    fn request_cleanup_retains_libraries_without_retaining_source_text() {
        let parser = Parser::new().ok().unwrap();
        let raw = "proxies: [{type: ss, name: request, password: secret}]";
        let expected = parser.preprocess(raw).unwrap();
        assert!(parser.preprocessed.borrow().is_some());
        parser.context().ok().unwrap().with(|ctx| {
            ctx.globals().set("retainedLibraryMarker", true).unwrap();
        });
        parser.clear_request();
        assert!(parser.preprocessed.borrow().is_none());
        parser.context().ok().unwrap().with(|ctx| {
            assert!(
                ctx.globals()
                    .get::<_, bool>("retainedLibraryMarker")
                    .unwrap()
            );
        });
        assert_eq!(parser.preprocess(raw).unwrap(), expected);
    }

    #[test]
    fn native_uri_parsing_defers_the_generic_runtime() {
        let parser = Parser::new().ok().unwrap();
        for raw in [
            "ss://none:p@127.0.0.1:443#native",
            "vless://11111111-1111-4111-8111-111111111111@v.example:443?type=ws&path=%2Fws#native",
            "trojan://secret@tr.example:443?allowInsecure=1#native",
            "hysteria2://secret@hy.example:443#native",
        ] {
            let nodes = crate::conversion::parse(&json!(raw), Some(&parser))
                .ok()
                .unwrap();
            assert_eq!(nodes.len(), 1);
            assert_eq!(nodes[0]["name"], "native");
            assert!(parser.context.get().is_none());
        }
        let strict = r#"{"type":"ss","name":"strict","extra":{"n":9007199254740993,"zero":-0}}"#;
        let proxy = parser.line(strict).ok().unwrap().unwrap();
        assert_eq!(proxy["name"], "strict");
        let canonical = "proxies:\n  - {\"type\":\"ss\",\"name\":\"canonical\",\"port\":443}\n";
        assert!(parser.preprocess(canonical).is_some());
        parser.context().ok().unwrap().with(|ctx| {
            let utilities: JsValue = ctx.globals().get("SubStoreUtilities").unwrap();
            assert!(utilities.is_undefined());
        });
        let raw = "{type:'ss',name:'fallback',extra:{nan:NaN,zero:-0}}";
        let expected = parser.line(raw).ok().unwrap();
        assert_eq!(expected.as_ref().unwrap()["name"], "fallback");
        assert!(parser.context.get().is_some());
        parser.context().ok().unwrap().with(|ctx| {
            let utilities: JsValue = ctx.globals().get("SubStoreUtilities").unwrap();
            assert!(utilities.is_object());
        });
        assert!(parser.line("{").ok().unwrap().is_none());
        assert_eq!(parser.line(raw).ok().unwrap(), expected);
        let fresh = Parser::new().ok().unwrap();
        assert!(fresh.context.get().is_none());
        assert_eq!(fresh.line(raw).ok().unwrap(), expected);
    }

    #[test]
    fn scalar_probes_preserve_generic_object_precedence_without_initializing_an_engine() {
        let parser = Parser::new().ok().unwrap();
        let oracle = Parser::new().ok().unwrap();
        for raw in [
            "Surge = ss, ss.example.com, 8388, encrypt-method=aes-128-gcm, password=secret",
            "shadowsocks=ss.example.com:8388,method=aes-128-gcm,password=secret,tag=QX",
            "shadowsocks=[2001:db8::1]:8388,method=aes-128-gcm,password=secret,tag=IPv6",
        ] {
            assert!(legacy_line(&oracle, raw).is_string());
            assert!(parser.line(raw).ok().unwrap().is_none());
            let nodes = crate::conversion::parse(&json!(raw), Some(&parser))
                .ok()
                .unwrap();
            assert_eq!(nodes.len(), 1);
            assert!(parser.context.get().is_none());
        }
        for raw in ["not a proxy", "\"quoted scalar\""] {
            assert!(legacy_line(&oracle, raw).is_string());
            assert!(parser.line(raw).ok().unwrap().is_none());
            assert!(parser.context.get().is_none());
        }
        for raw in [
            "type: ss",
            "/*prefix*/{type:'ss',name:'generic'}//=ss,ss.example,443,password=secret",
            "!!map {type: ss, name: tagged}",
        ] {
            assert_eq!(
                parser.line(raw).ok().unwrap(),
                Some(legacy_line(&oracle, raw))
            );
        }
        assert!(parser.context.get().is_some());
    }

    fn legacy_line(parser: &Parser, raw: &str) -> Value {
        parser.context().ok().unwrap().with(|ctx| {
            let utilities = utilities(&ctx).unwrap();
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
        let oracle = Parser::new().ok().unwrap();
        let runtime = Runtime::new().unwrap();
        let full = Context::full(&runtime).unwrap();
        full.with(|ctx| crate::js_values::initialize(&ctx).unwrap());
        oracle.context.set(full).ok().unwrap();
        parser.context().ok().unwrap().with(|ctx| {
            assert!(
                ctx.eval::<bool, _>(
                    r#"typeof RegExp === 'undefined' && typeof Date === 'undefined'
                        && typeof Uint8Array === 'undefined'"#,
                )
                .unwrap()
            );
            ctx.eval::<(), _>(
                "globalThis.originalJSON = JSON; globalThis.originalWeakMap = WeakMap",
            )
            .unwrap();
        });
        for raw in [
            r#"{"type":"ss","extra":{"n":9007199254740993,"zero":-0,"overflow":1e400,"key":null,"2":"two","1":"one","false":false,"empty":[]},"password":0}"#,
            r#"{"type":"ss","name":"\ud800\ue000\udc00","extra":{"__proto__":{"safe":true},"constructor":0,"literal":"😀"}}"#,
            r#"{type:'ss',port:0x1bb,extra:{nan:NaN,positive:Infinity,negative:-Infinity,zero:-0,},}"#,
            "{type: ss, name: yaml, extra: &loop {self: *loop}}",
        ] {
            assert_eq!(
                parser.line(raw).ok().unwrap().unwrap(),
                legacy_line(&oracle, raw)
            );
        }
        parser.context().ok().unwrap().with(|ctx| {
            assert!(
                ctx.eval::<bool, _>(
                    r#"JSON === originalJSON && WeakMap === originalWeakMap
                        && typeof RegExp === 'function' && typeof Date === 'function'
                        && typeof Uint8Array === 'function' && typeof Promise === 'function'"#,
                )
                .unwrap()
            );
        });
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
        // Upstream #667 reads the tree, so text that only mentions short-id stays intact.
        let literal = "proxies:\n  - {\"type\":\"ss\",\"extra\":\"short-id: raw\"}\n";
        assert_eq!(
            parser.preprocess(literal).unwrap(),
            "{\"type\":\"ss\",\"extra\":\"short-id: raw\"}\n"
        );
    }

    fn assert_quotes(input: &str, ids: &str) {
        let expected = ids.split_whitespace().fold(input.to_owned(), |text, id| {
            text.replacen(id, &format!("\"{id}\""), 1)
        });
        let output = short_ids(input).unwrap_or_else(|| input.to_owned());
        assert_eq!(output, expected, "{input}");
        assert_eq!(short_ids(&output), None, "{input}");
    }

    // Upstream 28888062, preprocessors.spec.js normalizeClashYaml.
    #[test]
    fn short_ids_are_quoted_only_inside_reality_options_through_aliases_and_merges() {
        let numeric = "11111111111 0000000000000001 9007199254740993";
        let block =
            "\n        - 11111111111\n        - 0000000000000001\n        - 9007199254740993";
        let flow = " [11111111111, 0000000000000001, 9007199254740993]";
        let listener = |ids: &str| {
            format!(
                "listeners:\n  - name: relay-proxy\n    type: vless\n    reality-config:\n      short-id:{ids}\n"
            )
        };
        let aliased = "listeners: [{reality-config: {short-id: *ids}}]\n";
        for (input, ids) in [
            ("proxies:\n  - reality-opts:\n      short-id: 08\n  - reality-opts:\n      short-id: 0088\n  - reality-opts:\n      short-id: '51'\n  - reality-opts:\n      short-id: \"\"\n  - reality-opts:\n      short-id: null\n".to_owned(), "08 0088"),
            ("metadata: {short-id: 1234}\nproxies:\n  - name: test\n    short-id: 5678\n    reality-opts: {short-id: 0088}\n".to_owned(), "0088"),
            ("opts: &opts {short-id: 0088}\nproxies:\n  - reality-opts: *opts\n  - reality-opts: *opts\n".to_owned(), "0088"),
            ("id: &id 0011\nproxies: [{reality-opts: {short-id: *id}}]\n".to_owned(), "0011"),
            ("base: &base {short-id: 0011}\nmid: &mid {<<: *base}\nopts: &opts {<<: *mid}\nproxies: [{reality-opts: *opts}]\n".to_owned(), "0011"),
            ("template: &template {reality-opts: {short-id: 0011}}\ntemplates: &templates [*template]\nnode: &node {<<: *templates}\nproxies: [*node]\n".to_owned(), "0011"),
            ("opts: &opts {short-id: 0011}\nnode: &node {reality-opts: *opts}\nnodes: &nodes [*node]\nprofile: &profile {proxies: *nodes}\ninherited: &inherited {<<: *profile}\n<<: *inherited\n".to_owned(), "0011"),
            ("profile: &profile {listeners: [{reality-config: {short-id: [0011]}}]}\nprofiles: &profiles [*profile]\n<<: *profiles\n".to_owned(), "0011"),
            ("profile: &profile {proxies: [{reality-opts: {short-id: 0011}}]}\nprofiles: &profiles [*profile]\n<<: *profiles\n".to_owned(), "0011"),
            (format!("%YAML 1.1\n---\nids: &ids [0011]\n{aliased}"), "0011"),
            (format!("ids: &ids [0088, !!int 0011, !!str 0088, null]\n{aliased}"), "0088"),
            ("ids: &ids 0011 # keep\nlisteners: [{reality-config: {short-id: [*ids]}}]\n".to_owned(), "0011"),
            (format!("ids: &ids [0011] # keep\n{aliased}"), "0011"),
            (format!("ids: &ids [0011 # keep\n]\n{aliased}"), "0011"),
            (format!("ids: &ids\n  # keep\n  - 0011\n{aliased}"), "0011"),
            (format!("ids: &ids\n  - 0011\n\n  - 0022\n{aliased}"), "0011 0022"),
            (listener(block), numeric),
            (format!("proxies: []\n{}", listener(flow)), numeric),
            (format!("proxies: [{{name: test}}]\n{}", listener(block)), numeric),
            ("ids: &ids [11111111111, 0000000000000001]\nlisteners:\n  - reality-config: {short-id: *ids}\n  - reality-config: {short-id: *ids}\n".to_owned(), "11111111111 0000000000000001"),
            ("# short-id: 0011\nproxies:\n  - {name: test, reality-opts: {short-id: 0088}}\nshort-ids: &ids [6a0f0e0ede903cad]\nlisteners:\n  - reality-config: {short-id: *ids}\ndescription: 'short-id: 0022'\nscript: |\n  short-id: 0033\nempty:\n  short-id:\n".to_owned(), "0088"),
            ("opts: &opts {short-id: 0011, \"<<\": keep}\nmetadata: *opts\nproxies: [{reality-opts: *opts}]\n".to_owned(), "0011"),
            ("proxies:\n  - name: first\n    &opts reality-opts: {&sid short-id: 0088}\n  - name: second\n    *opts : {*sid : 0011}\n".to_owned(), "0088 0011"),
            ("first: &opts {short-id: 0088}\nproxies:\n  - reality-opts: *opts\n  - reality-opts: &opts {short-id: 0011}\n  - reality-opts: *opts\n".to_owned(), "0088 0011"),
            ("first: &first {short-id: 0011}\nsecond: &second {short-id: 0022}\nproxies:\n  - reality-opts: *second\n  - reality-opts: {<<: *first}\n".to_owned(), "0011 0022"),
            ("proxies:\n  - name: !<str>Legacy\n    server: !<str>example.com\n    reality-opts:\n      short-id: 0088\n".to_owned(), "0088"),
            ("ids: &loop [*loop, 0011]\nproxies: [{reality-opts: {short-id: *loop}}]\n".to_owned(), "0011"),
            ("%YAML 1.1\n---\nproxies: [{reality-opts: {short-id: [0b10, e5, 0o17]}}]\n".to_owned(), "0b10 e5"),
            ("proxies: [{reality-opts: {short-id: [0b10, e5, 0o17, 1e5, true]}}]\n".to_owned(), "0o17 1e5 true"),
            ("proxies: [{reality-opts: {short-id: \"0088\"}}]\n".to_owned(), ""),
            ("proxies:\n  - name: broken\n    short-id: [".to_owned(), ""),
            ("proxies: [{reality-opts: {short-id: 0088}}]\n---\nproxies: []\n".to_owned(), ""),
            ("ss://YWVzLTEyOC1nY206c2VjcmV0@example.com:8388".to_owned(), ""),
        ] {
            assert_quotes(&input, ids);
        }
        for value in [
            "abcdef0123456789",
            "!!str 0088",
            "!!int 0088",
            "!custom 0088",
        ] {
            assert_quotes(
                &format!("proxies:\n  - name: test\n    reality-opts: {{short-id: {value}}}\n"),
                "",
            );
        }
        for value in [
            "\n        - 6a0f0e0ede903cad",
            " # ids\n        - 6a0f0e0ede903cad",
            " &ids\n        - 6a0f0e0ede903cad",
            " []",
            " [6a0f0e0ede903cad, \"08\"]",
            " &ids [6a0f0e0ede903cad]",
        ] {
            assert_quotes(
                &format!(
                    "proxies:\n  - name: test\n    type: vless\n    reality-opts:\n      short-id: 0088\nlisteners:\n  - type: vless\n    reality-config:\n      short-id:{value}\n"
                ),
                "0088",
            );
        }
        for word in [
            "1e3", ".5", "-1", "+0.", "0x1F", "0o17", "true", "False", ".inf", "-.INF", ".NaN",
        ] {
            assert!(non_string(word, false), "{word}");
        }
        for word in [
            "0b11", "e5", "1_000", "yes", "12:34", "6a0f", "1e", "+", ".", "-.nan", "0x", "0o8",
            "~", "",
        ] {
            assert!(!non_string(word, false), "{word}");
        }
        assert!(non_string("-0b11", true) && non_string("E-5", true) && !non_string("0o17", true));
    }

    #[test]
    fn preprocess_keeps_short_ids_through_aliased_field_names_merges_and_legacy_tags() {
        let parser = Parser::new().ok().unwrap();
        for (raw, expected) in [
            (
                "proxies:\n  - name: first\n    &opts reality-opts: {&sid short-id: 0088}\n  - name: second\n    *opts : {*sid : 0011}\n",
                json!(["0088", "0011"]),
            ),
            (
                "first: &opts {short-id: 0088}\nproxies:\n  - reality-opts: *opts\n  - reality-opts: &opts {short-id: 0011}\n  - reality-opts: *opts\n",
                json!(["0088", "0011", "0011"]),
            ),
            (
                "first: &first {short-id: 0011}\nsecond: &second {short-id: 0022}\nproxies:\n  - reality-opts: *second\n  - reality-opts: {<<: *first}\n  - reality-opts: *second\n",
                json!(["0022", "0011", "0022"]),
            ),
            (
                "proxies:\n  - name: !<str>Legacy\n    type: vless\n    server: !<str>example.com\n    reality-opts:\n      short-id: 0088\n",
                json!(["0088"]),
            ),
            (
                "proxies:\n  - {\"name\":\"canonical\",\"reality-opts\":{\"short-id\":88}}\n",
                json!(["88"]),
            ),
            (
                "proxies:\n  - {\"name\":\"canonical\",\"reality-opts\":{\"short-id\":\"88\"}}\n",
                json!(["88"]),
            ),
        ] {
            let ids: Vec<Value> = parser
                .preprocess(raw)
                .unwrap()
                .lines()
                .map(|line| {
                    serde_json::from_str::<Value>(line).unwrap()["reality-opts"]["short-id"].clone()
                })
                .collect();
            assert_eq!(json!(ids), expected, "{raw}");
        }
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
            "{000-provider: {uuid: 00112233-4455-6677-8899-aabbccddeeff, password: 0bX_private}}",
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
        parser.context().ok().unwrap().with(|ctx| {
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
            "proxies: [{type: ss, server: 127.0.0.1, port: 443, uuid: 00112233-4455-6677-8899-aabbccddeeff, password: 0bX_private, extra: {000-provider: value}}]",
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
            let expected = parser
                .context()
                .ok()
                .unwrap()
                .with(|ctx| match yaml(&ctx, &raw) {
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
            let parser = Parser::new().ok().unwrap();
            parser.context().ok().unwrap().with(|ctx| {
                black_box(utilities(&ctx).unwrap());
            });
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
        parser.context().ok().unwrap().with(|ctx| {
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
        let expected = parser.context().ok().unwrap().with(|ctx| {
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
        let actual = parser.context().ok().unwrap().with(|ctx| {
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
