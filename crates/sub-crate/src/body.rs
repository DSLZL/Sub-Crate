use crate::json as codec;
use flate2::{Decompress, FlushDecompress, Status};
use http_body_util::{BodyExt, Limited};
use hyper::{HeaderMap, Request, body::Bytes, body::Incoming};
use percent_encoding::percent_decode_str;
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;

#[derive(Clone)]
pub enum Body {
    Json(Value),
    Raw(Bytes),
}

pub struct Error {
    pub status: u16,
    pub detail: String,
}

impl Error {
    fn new(status: u16, detail: impl Into<String>) -> Self {
        Self {
            status,
            detail: detail.into(),
        }
    }
}

impl Body {
    pub fn fields(&self) -> Map<String, Value> {
        match self {
            Self::Json(Value::Object(object)) => object.clone(),
            Self::Json(Value::Array(items)) => items
                .iter()
                .enumerate()
                .map(|(index, item)| (index.to_string(), item.clone()))
                .collect(),
            Self::Json(Value::String(text)) => codec::units(text)
                .into_iter()
                .enumerate()
                .map(|(index, unit)| (index.to_string(), json!(codec::from_units(&[unit]))))
                .collect(),
            Self::Raw(bytes) => bytes
                .iter()
                .enumerate()
                .map(|(index, byte)| (index.to_string(), json!(byte)))
                .collect(),
            _ => Map::new(),
        }
    }

    pub fn into_json(self) -> Value {
        match self {
            Self::Json(value) => value,
            Self::Raw(bytes) => json!({"type": "Buffer", "data": bytes.to_vec()}),
        }
    }
}

fn media_token(c: char) -> bool {
    c.is_ascii() && (c.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&(c as u8)))
}

fn media_parameters(value: &str) -> Option<(String, BTreeMap<String, String>)> {
    let delimiter = value.find(';').unwrap_or(value.len());
    let media = value[..delimiter].trim_matches(' ').to_ascii_lowercase();
    let restricted = |s: &str, punctuation: &[u8]| {
        s.len() <= 127
            && s.as_bytes().first().is_some_and(u8::is_ascii_alphanumeric)
            && s.bytes()
                .all(|b| b.is_ascii_alphanumeric() || punctuation.contains(&b))
    };
    let (kind, subtype) = media.split_once('/')?;
    if !restricted(kind, b"!#$&^_-") || !restricted(subtype, b"!#$&^_.+-") {
        return None;
    }
    let (subtype, suffix) = subtype.rsplit_once('+').unwrap_or((subtype, ""));
    if !restricted(subtype, b"!#$&^_.-") || (!suffix.is_empty() && !restricted(suffix, b"!#$&^_-"))
    {
        return None;
    }
    let media = if suffix.is_empty() {
        format!("{kind}/{subtype}")
    } else {
        format!("{kind}/{subtype}+{suffix}")
    };
    let mut parameters = BTreeMap::new();
    let mut rest = &value[delimiter..];
    while !rest.is_empty() {
        rest = rest.strip_prefix(';')?.trim_start_matches(' ');
        let end = rest.find(|c| !media_token(c)).unwrap_or(rest.len());
        if end == 0 {
            return None;
        }
        let key = &rest[..end];
        rest = rest[end..].trim_start_matches(' ');
        rest = rest.strip_prefix('=')?.trim_start_matches(' ');
        let value = if let Some(quoted) = rest.strip_prefix('"') {
            let mut chars = quoted.char_indices();
            let mut value = String::new();
            loop {
                let (index, c) = chars.next()?;
                match c {
                    '"' => {
                        rest = &quoted[index + 1..];
                        break;
                    }
                    '\\' => {
                        let (_, escaped) = chars.next()?;
                        if !(' '..='~').contains(&escaped) {
                            return None;
                        }
                        value.push(escaped);
                    }
                    ' ' | '!' | '#'..='[' | ']'..='~' | '\u{80}'..='\u{ff}' => value.push(c),
                    _ => return None,
                }
            }
            value
        } else {
            let end = rest.find(|c| !media_token(c)).unwrap_or(rest.len());
            if end == 0 {
                return None;
            }
            let value = rest[..end].to_owned();
            rest = &rest[end..];
            value
        };
        rest = rest.trim_start_matches(' ');
        parameters.insert(key.to_ascii_lowercase(), value);
    }
    Some((media, parameters))
}

fn content_type(value: &str) -> Option<(String, String)> {
    let (media, parameters) = media_parameters(value)?;
    let charset = parameters
        .get("charset")
        .filter(|value| !value.is_empty())
        .map_or_else(|| "utf-8".to_owned(), |value| value.to_ascii_lowercase());
    Some((media, charset))
}

pub fn response_content_type(value: &str) -> Option<String> {
    let (mut media, mut parameters) = media_parameters(value)?;
    parameters.insert("charset".to_owned(), "utf-8".to_owned());
    for (key, mut value) in parameters {
        if value.is_empty() || !value.chars().all(media_token) {
            value = format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""));
        }
        media.push_str(&format!("; {key}={value}"));
    }
    Some(media)
}

fn decode(bytes: &[u8], charset: &str) -> String {
    let text = if charset == "utf-8" {
        String::from_utf8_lossy(bytes).into_owned()
    } else {
        let words: Vec<_> = bytes
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| u16::from_le_bytes(*pair))
            .collect();
        codec::utf16_json(&words)
    };
    text.strip_prefix('\u{feff}').unwrap_or(&text).to_owned()
}

pub async fn read(request: Request<Incoming>, json_limit: usize) -> Result<Body, Error> {
    let (parts, mut incoming) = request.into_parts();
    let result = read_body(&parts.headers, &mut incoming, json_limit).await;
    // Finish the request before replying: unread bytes can reset Windows TCP and hide the error.
    while let Some(frame) = incoming.frame().await {
        if frame.is_err() {
            break;
        }
    }
    result
}

async fn read_body(
    headers: &HeaderMap,
    incoming: &mut Incoming,
    json_limit: usize,
) -> Result<Body, Error> {
    let media = headers
        .get("content-type")
        .map(|v| {
            v.as_bytes()
                .iter()
                .map(|&b| char::from(b))
                .collect::<String>()
        })
        .and_then(|v| content_type(&v));
    let Some((media, charset)) = media.filter(|_| {
        headers.contains_key("content-length") || headers.contains_key("transfer-encoding")
    }) else {
        return Ok(Body::Json(json!({})));
    };
    let json = media == "application/json";
    let form = media == "application/x-www-form-urlencoded";
    let canonical = match charset.rsplit_once(':') {
        Some((name, year)) if year.len() == 4 && year.bytes().all(|b| b.is_ascii_digit()) => name,
        _ => charset.as_str(),
    };
    let canonical: String = canonical
        .bytes()
        .filter(u8::is_ascii_alphanumeric)
        .map(char::from)
        .collect();
    if (json
        && (!charset.starts_with("utf-")
            || !["utf8", "utf16le", "utf16", "utf16be", "utf7", "utf7imap"]
                .contains(&canonical.as_str())))
        || (form && charset != "utf-8")
    {
        return Err(Error::new(
            415,
            format!(
                "UnsupportedMediaTypeError: unsupported charset \"{}\"",
                charset.to_uppercase()
            ),
        ));
    }
    let encoding = headers
        .get("content-encoding")
        .and_then(|v| v.to_str().ok())
        .filter(|v| !v.is_empty())
        .unwrap_or("identity")
        .to_ascii_lowercase();
    if !["identity", "gzip", "deflate"].contains(&encoding.as_str()) {
        return Err(Error::new(
            415,
            format!("UnsupportedMediaTypeError: unsupported content encoding \"{encoding}\""),
        ));
    }
    let limit = if json { json_limit } else { 100 * 1024 };
    let bytes = if encoding == "identity" {
        Limited::new(incoming, limit)
            .collect()
            .await
            .map_err(|_| Error::new(413, "PayloadTooLargeError: request entity too large"))?
            .to_bytes()
    } else {
        inflate(incoming, encoding == "gzip", limit).await?
    };
    // The legacy rawBodySaver uses Buffer.toString, which rejects these iconv-supported charsets.
    if json && !bytes.is_empty() && !["utf-8", "utf-16le"].contains(&charset.as_str()) {
        return Err(Error::new(
            403,
            format!("TypeError [ERR_UNKNOWN_ENCODING]: Unknown encoding: {charset}"),
        ));
    }
    if json || form {
        let text = decode(&bytes, &charset);
        let value = if form {
            parse_fields(&text, false).map_err(|status| {
                Error::new(
                    status,
                    if status == 413 {
                        "PayloadTooLargeError: too many parameters"
                    } else {
                        "BadRequestError: The input exceeded the depth"
                    },
                )
            })?
        } else if text.is_empty() {
            json!({})
        } else {
            match codec::parse(&text) {
                Ok(value @ (Value::Object(_) | Value::Array(_))) => value,
                Ok(_) => {
                    return Err(Error::new(
                        400,
                        "SyntaxError: JSON body must be an object or array",
                    ));
                }
                Err(error) => return Err(Error::new(400, format!("SyntaxError: {error}"))),
            }
        };
        Ok(Body::Json(value))
    } else {
        Ok(Body::Raw(bytes))
    }
}

async fn inflate(body: &mut Incoming, gzip: bool, limit: usize) -> Result<Bytes, Error> {
    let mut decoder = if gzip {
        Decompress::new_gzip(15)
    } else {
        Decompress::new(true)
    };
    let mut output = Vec::new();
    let mut finished = false;
    let mut padding = false;
    while let Some(frame) = body.frame().await {
        let frame = frame.map_err(|error| Error::new(400, format!("Error: {error}")))?;
        let Ok(data) = frame.into_data() else {
            continue;
        };
        let mut input = data.as_ref();
        while !padding {
            if finished {
                let Some(&first) = input.first() else {
                    break;
                };
                // Node ignores bytes after a zlib stream or gzip's first padding zero.
                if !gzip || first == 0 {
                    padding = true;
                    break;
                }
                decoder = Decompress::new_gzip(15);
            }
            let mut decoded = [0u8; 8192];
            let before_in = decoder.total_in();
            let before_out = decoder.total_out();
            let status = decoder.decompress(input, &mut decoded, FlushDecompress::None);
            let consumed = (decoder.total_in() - before_in) as usize;
            let produced = (decoder.total_out() - before_out) as usize;
            if produced > limit.saturating_sub(output.len()) {
                return Err(Error::new(
                    413,
                    "PayloadTooLargeError: request entity too large",
                ));
            }
            let status = status.map_err(|error| Error::new(400, format!("Error: {error}")))?;
            output.extend_from_slice(&decoded[..produced]);
            input = &input[consumed..];
            finished = status == Status::StreamEnd;
            if consumed == 0 && produced == 0 && !finished {
                if !input.is_empty() {
                    return Err(Error::new(400, "Error: Invalid compressed request body"));
                }
                break;
            }
        }
    }
    if !finished {
        return Err(Error::new(400, "Error: Incomplete compressed request body"));
    }
    Ok(Bytes::from(output))
}

fn form_decode(value: &str) -> String {
    let value = value.replace('+', " ");
    let bytes = value.as_bytes();
    if bytes.iter().enumerate().any(|(index, &byte)| {
        byte == b'%'
            && !bytes
                .get(index + 1..index + 3)
                .is_some_and(|digits| digits.iter().all(u8::is_ascii_hexdigit))
    }) {
        return codec::encode(&value);
    }
    let decoded = percent_decode_str(&value)
        .decode_utf8()
        .map(|decoded| decoded.into_owned())
        .unwrap_or(value);
    codec::encode(&decoded)
}

// qs 6.16.1 as Express 4.22.3 parses queries and body-parser 1.20.9 parses forms.
enum Node {
    Text(String),
    /// A JavaScript array: length, present items by index, and qs's strictMerge conflict mark.
    List(usize, BTreeMap<usize, Node>, bool),
    /// A plain object with qs's overflow and conflict side channels (their highest index).
    Dict(Vec<(String, Node)>, Option<f64>, Option<f64>),
}

/// Keys qs treats as numbers: `String(parseInt(key, 10)) === key`, not negative.
fn numeric_index(key: &str) -> Option<f64> {
    key.parse::<f64>().ok().filter(|index| {
        index.is_finite()
            && !index.is_sign_negative()
            && *index < 1e21
            && index.fract() == 0.0
            && index.to_string() == key
    })
}

/// JavaScript `parseInt(key, 10)`.
fn parse_int(key: &str) -> f64 {
    let text = codec::trim(key);
    let digits = text.strip_prefix(['+', '-']).unwrap_or(text);
    let end = digits
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(digits.len());
    let value = digits[..end].parse::<f64>().unwrap_or(f64::NAN);
    if text.starts_with('-') { -value } else { value }
}

/// JavaScript objects list array-index keys first in numeric order, then the rest as inserted.
fn js_order<T>(mut fields: Vec<(String, T)>) -> Vec<(String, T)> {
    fields.sort_by_key(|(key, _)| {
        key.parse::<u32>()
            .ok()
            .filter(|index| *index < u32::MAX && index.to_string() == *key)
            .map_or(u64::MAX, u64::from)
    });
    fields
}

fn set(fields: &mut Vec<(String, Node)>, key: String, value: Node) {
    match fields.iter_mut().find(|(name, _)| *name == key) {
        Some((_, slot)) => *slot = value,
        None => fields.push((key, value)),
    }
}

fn object(items: BTreeMap<usize, Node>) -> Vec<(String, Node)> {
    items
        .into_iter()
        .map(|(index, item)| (index.to_string(), item))
        .collect()
}

/// `[].concat`: appends a value, spreading one array level with its holes.
fn append(len: &mut usize, items: &mut BTreeMap<usize, Node>, value: Node) {
    if let Node::List(more_len, more, _) = value {
        items.extend(more.into_iter().map(|(index, item)| (*len + index, item)));
        *len += more_len;
    } else {
        items.insert(*len, value);
        *len += 1;
    }
}

/// Arrays past the limit become objects that remember their last index.
fn overflowed(len: usize, items: BTreeMap<usize, Node>, conflict: bool, limit: usize) -> Node {
    if len > limit {
        Node::Dict(object(items), Some((len - 1) as f64), None)
    } else {
        Node::List(len, items, conflict)
    }
}

/// `utils.combine`. `left` is never an overflowed object here: a form's arrayLimit is
/// at least its parameter count, and a query keeps at most 1000 parameters.
fn combine(left: Node, right: Node, limit: usize) -> Node {
    let (mut len, mut items) = (0, BTreeMap::new());
    append(&mut len, &mut items, left);
    append(&mut len, &mut items, right);
    overflowed(len, items, false, limit)
}

/// `utils.merge` with the default strictMerge.
fn merge(target: Node, source: Node, limit: usize) -> Node {
    match (target, source) {
        (target, Node::Text(text)) if text.is_empty() => target,
        (Node::List(len, mut items, conflict), source @ Node::Text(_)) => {
            items.insert(len, source);
            overflowed(len + 1, items, conflict, limit)
        }
        (Node::Dict(mut fields, Some(index), conflict), source @ Node::Text(_)) => {
            set(&mut fields, (index + 1.0).to_string(), source);
            Node::Dict(fields, Some(index + 1.0), conflict)
        }
        (Node::Dict(mut fields, None, Some(index)), source @ Node::Text(_)) => {
            set(&mut fields, (index + 1.0).to_string(), source);
            Node::Dict(fields, None, Some(index + 1.0))
        }
        // Both limits are at least 100, so qs's limited pair stays an array.
        (target @ Node::Dict(..), source @ Node::Text(_)) => {
            Node::List(2, BTreeMap::from([(0, target), (1, source)]), true)
        }
        (target @ Node::Text(_), Node::Dict(fields, Some(index), _)) => {
            let mut result = vec![("0".to_owned(), target)];
            for (key, value) in js_order(fields) {
                set(&mut result, (parse_int(&key) + 1.0).to_string(), value);
            }
            Node::Dict(result, Some(index + 1.0), None)
        }
        (target @ Node::Text(_), source) => combine(target, source, limit),
        (Node::List(mut len, mut items, conflict), Node::List(_, more, _)) => {
            for (index, item) in more {
                let nested = !matches!(item, Node::Text(_));
                match items.remove(&index) {
                    Some(old) if nested && !matches!(old, Node::Text(_)) => {
                        items.insert(index, merge(old, item, limit));
                    }
                    Some(old) => {
                        items.insert(index, old);
                        items.insert(len, item);
                        len += 1;
                    }
                    None => {
                        items.insert(index, item);
                        len = len.max(index + 1);
                    }
                }
            }
            overflowed(len, items, conflict, limit)
        }
        (target, source) => {
            let (mut fields, mut overflow, mut conflict) = match target {
                Node::Dict(fields, overflow, conflict) => (fields, overflow, conflict),
                Node::List(len, items, marked) => {
                    (object(items), None, marked.then_some(len as f64 - 1.0))
                }
                Node::Text(_) => unreachable!("text targets are merged above"),
            };
            let (entries, source_overflow) = match source {
                Node::Dict(fields, overflow, _) => (js_order(fields), overflow),
                Node::List(_, items, _) => (object(items), None),
                Node::Text(_) => unreachable!("text sources are merged above"),
            };
            for (key, value) in entries {
                let index = numeric_index(&key);
                match fields.iter().position(|(name, _)| *name == key) {
                    Some(at) => {
                        let old = std::mem::replace(&mut fields[at].1, Node::Text(String::new()));
                        fields[at].1 = merge(old, value, limit);
                    }
                    None => fields.push((key, value)),
                }
                if let Some(source) = source_overflow
                    && overflow.is_none()
                {
                    let indexes = fields.iter().filter_map(|(name, _)| numeric_index(name));
                    overflow = Some(indexes.fold(source, f64::max));
                }
                for mark in [&mut overflow, &mut conflict] {
                    if let (Some(maximum), Some(index)) = (mark.as_mut(), index) {
                        *maximum = maximum.max(index);
                    }
                }
            }
            Node::Dict(fields, overflow, conflict)
        }
    }
}

/// `splitKeyIntoSegments`: the parent, then balanced bracket groups up to `depth`.
/// Returns `None` when a strict depth is exceeded.
fn segments(key: &str, depth: usize, strict: bool) -> Option<Vec<(&str, bool)>> {
    let first = key.find('[');
    let parent = &key[..first.unwrap_or(key.len())];
    let mut segments = Vec::new();
    if !parent.is_empty() {
        segments.push((parent, false));
    }
    let mut open = first;
    let mut collected = 0;
    while let Some(start) = open.filter(|_| collected < depth) {
        let mut level = 0;
        let close = key[start..].bytes().position(|byte| {
            level += match byte {
                b'[' => 1,
                b']' => -1,
                _ => 0,
            };
            level == 0
        });
        let Some(close) = close.map(|offset| start + offset) else {
            // An unterminated group stays one literal segment.
            segments.push((&key[start..], true));
            return Some(segments);
        };
        segments.push((&key[start + 1..close], true));
        collected += 1;
        open = key[close + 1..].find('[').map(|offset| close + 1 + offset);
    }
    if let Some(start) = open {
        if strict {
            return None;
        }
        segments.push((&key[start..], true));
    }
    Some(segments)
}

/// `parseObject`; bracketed segments come without their brackets.
fn parse_object(chain: &[(&str, bool)], leaf: Node, limit: usize) -> Node {
    chain.iter().rev().fold(leaf, |leaf, &(root, bracket)| {
        if bracket && root.is_empty() {
            if matches!(leaf, Node::Dict(_, Some(_), _)) {
                leaf
            } else {
                combine(Node::List(0, BTreeMap::new(), false), leaf, limit)
            }
        } else if let Some(index) = numeric_index(root).filter(|_| bracket) {
            if index < limit as f64 {
                Node::List(
                    index as usize + 1,
                    BTreeMap::from([(index as usize, leaf)]),
                    false,
                )
            } else {
                Node::Dict(vec![(root.to_owned(), leaf)], Some(index), None)
            }
        } else if root == "__proto__" {
            Node::Dict(Vec::new(), None, None)
        } else {
            Node::Dict(vec![(root.to_owned(), leaf)], None, None)
        }
    })
}

/// `utils.compact`, which also drops the side channels.
fn compact(node: Node) -> Value {
    match node {
        Node::Text(text) => Value::String(text),
        Node::List(_, items, _) => items.into_values().map(compact).collect(),
        Node::Dict(fields, ..) => Value::Object(
            js_order(fields)
                .into_iter()
                .map(|(key, value)| (key, compact(value)))
                .collect(),
        ),
    }
}

pub fn query(text: &str) -> Value {
    // Query limits truncate; only body forms reject excessive parameters/depth.
    parse_fields(text, true).unwrap()
}

fn parse_fields(text: &str, query: bool) -> Result<Value, u16> {
    if text.is_empty() {
        return Ok(json!({}));
    }
    let count = text.bytes().filter(|&b| b == b'&').count() + 1;
    if !query && count > 1000 {
        return Err(413);
    }
    // Express passes arrayLimit 1000; body-parser passes max(100, parameter count).
    let limit = if query { 1000 } else { count.max(100) };
    let mut values: Vec<(String, Node)> = Vec::new();
    for pair in text.split('&').take(1000) {
        let pair = pair
            .replace("%5B", "[")
            .replace("%5b", "[")
            .replace("%5D", "]")
            .replace("%5d", "]");
        let split = pair.find("]=").map(|i| i + 1).or_else(|| pair.find('='));
        let (key, value) = split
            .map(|i| (&pair[..i], &pair[i + 1..]))
            .unwrap_or((&pair, ""));
        let key = form_decode(key);
        let value = Node::Text(form_decode(value));
        match values.iter_mut().find(|(name, _)| *name == key) {
            Some((_, old)) => {
                *old = combine(
                    std::mem::replace(old, Node::Text(String::new())),
                    value,
                    limit,
                );
            }
            None => values.push((key, value)),
        }
    }
    let mut result = Node::Dict(Vec::new(), None, None);
    for (key, value) in js_order(values) {
        if key.is_empty() {
            continue;
        }
        let chain = segments(&key, if query { 5 } else { 32 }, !query).ok_or(400u16)?;
        result = merge(result, parse_object(&chain, value, limit), limit);
    }
    Ok(compact(result))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queries_and_forms_match_qs() {
        // Generated with qs 6.16.1 and the Express 4.22.3 and body-parser 1.20.9 options.
        let cases: Value =
            serde_json::from_str(include_str!("../../../tests/compat/fixtures/qs-cases.json"))
                .unwrap();
        for case in cases["query"].as_array().unwrap() {
            let text = case[0].as_str().unwrap();
            assert_eq!(query(text).to_string(), case[1].to_string(), "{text}");
        }
        for case in cases["form"].as_array().unwrap() {
            let text = case[0].as_str().unwrap();
            let actual = match parse_fields(text, false) {
                Ok(value) => json!({ "value": value }),
                Err(status) => json!({ "status": status }),
            };
            assert_eq!(actual.to_string(), case[1].to_string(), "{text}");
        }
    }
}
