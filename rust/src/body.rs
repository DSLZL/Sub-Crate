use crate::json as codec;
use crate::state::truthy;
use flate2::{Decompress, FlushDecompress, Status};
use http_body_util::{BodyExt, Limited};
use hyper::{HeaderMap, Request, body::Bytes, body::Incoming};
use percent_encoding::percent_decode_str;
use serde_json::{Map, Value, json};

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

fn content_type(value: &str) -> Option<(String, String)> {
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
    let token = |c: char| {
        c.is_ascii() && (c.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&(c as u8)))
    };
    let mut charset = "utf-8".to_owned();
    let mut rest = &value[delimiter..];
    while !rest.is_empty() {
        rest = rest.strip_prefix(';')?.trim_start_matches(' ');
        let end = rest.find(|c| !token(c)).unwrap_or(rest.len());
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
            let end = rest.find(|c| !token(c)).unwrap_or(rest.len());
            if end == 0 {
                return None;
            }
            let value = rest[..end].to_owned();
            rest = &rest[end..];
            value
        };
        rest = rest.trim_start_matches(' ');
        if key.eq_ignore_ascii_case("charset") {
            charset = value.to_ascii_lowercase();
        }
    }
    if charset.is_empty() {
        charset = "utf-8".to_owned();
    }
    Some((media, charset))
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

// Input keys escape this marker; remove the unreachable bookkeeping key in compact.
const OVERFLOW: &str = "\u{e000}overflow";

fn overflow_index(value: &Value) -> Option<f64> {
    value.get(OVERFLOW).and_then(Value::as_f64)
}

fn numeric_index(key: &str) -> Option<f64> {
    key.parse::<f64>().ok().filter(|index| {
        index.is_finite()
            && !index.is_sign_negative()
            && *index < 1e21
            && index.fract() == 0.0
            && index.to_string() == key
    })
}

fn array_object(items: Vec<Value>, overflow: Option<f64>) -> Value {
    let mut fields: Map<_, _> = items
        .into_iter()
        .enumerate()
        .filter(|(_, value)| !value.is_null())
        .map(|(index, value)| (index.to_string(), value))
        .collect();
    if let Some(index) = overflow {
        fields.insert(OVERFLOW.to_owned(), json!(index));
    }
    Value::Object(fields)
}

fn combine(left: Value, right: Value, limit: usize) -> Value {
    if let Some(index) = overflow_index(&left) {
        let mut result = left;
        result[(index + 1.0).to_string()] = right;
        result[OVERFLOW] = json!(index + 1.0);
        return result;
    }
    let mut items = match left {
        Value::Array(items) => items,
        value => vec![value],
    };
    match right {
        Value::Array(right) => items.extend(right),
        value => items.push(value),
    }
    if items.len() > limit {
        let last = items.len() - 1;
        array_object(items, Some(last as f64))
    } else {
        Value::Array(items)
    }
}

fn merge(target: &mut Value, source: Value, query: bool, limit: usize) {
    if !truthy(&source) {
        return;
    }
    if target.is_null() {
        *target = source;
        return;
    }
    match (target, source) {
        (Value::Array(left), Value::Array(right)) => {
            for (index, value) in right.into_iter().enumerate() {
                if value.is_null() {
                    continue;
                }
                if index >= left.len() {
                    left.resize(index + 1, Value::Null);
                    left[index] = value;
                } else if left[index].is_null() {
                    left[index] = value;
                } else if (left[index].is_object() || left[index].is_array())
                    && (value.is_object() || value.is_array())
                {
                    merge(&mut left[index], value, query, limit);
                } else {
                    left.push(value);
                }
            }
        }
        (left @ Value::Array(_), right @ Value::Object(_)) => {
            let items = std::mem::take(left).as_array().unwrap().clone();
            *left = array_object(items, None);
            merge(left, right, query, limit);
        }
        (Value::Object(left), Value::Object(mut right)) => {
            let overflow = right.remove(OVERFLOW).and_then(|value| value.as_f64());
            for (key, value) in right {
                if let Some(old) = left.get_mut(&key) {
                    merge(old, value, query, limit);
                } else {
                    left.insert(key.clone(), value);
                }
                let maximum = left.get(OVERFLOW).and_then(Value::as_f64).or(overflow);
                if let Some(maximum) = maximum {
                    let index = numeric_index(&key).unwrap_or(maximum).max(maximum);
                    left.insert(OVERFLOW.to_owned(), json!(index));
                }
            }
        }
        (left @ Value::Object(_), Value::Array(right)) => {
            merge(left, array_object(right, None), query, limit);
        }
        (left @ Value::Object(_), right) if !right.is_array() && !right.is_object() => {
            if let Some(index) = overflow_index(left) {
                left[(index + 1.0).to_string()] = right;
                left[OVERFLOW] = json!(index + 1.0);
            } else if query {
                let key = right
                    .as_str()
                    .map(str::to_owned)
                    .unwrap_or_else(|| right.to_string());
                if key != "__proto__" {
                    left[key] = Value::Bool(true);
                }
            } else {
                *left = json!([std::mem::take(left), right]);
            }
        }
        (left @ Value::Array(_), right) => {
            let items = left.as_array_mut().unwrap();
            items.push(right);
            let last = items.len() - 1;
            if last > limit {
                *left = array_object(std::mem::take(items), Some(last as f64));
            }
        }
        (left, right @ Value::Array(_)) => {
            *left = combine(std::mem::take(left), right, limit);
        }
        (left, right) if overflow_index(&right).is_some() => {
            let maximum = overflow_index(&right).unwrap() + 1.0;
            let mut fields = Map::from_iter([("0".to_owned(), std::mem::take(left))]);
            for (key, value) in right.as_object().unwrap() {
                if key != OVERFLOW {
                    let text = codec::trim(key);
                    let digits = text
                        .strip_prefix('+')
                        .or_else(|| text.strip_prefix('-'))
                        .unwrap_or(text);
                    let end = digits
                        .find(|c: char| !c.is_ascii_digit())
                        .unwrap_or(digits.len());
                    let index = digits[..end].parse::<f64>().unwrap_or(f64::NAN);
                    let index = if text.starts_with('-') { -index } else { index };
                    fields.insert((index + 1.0).to_string(), value.clone());
                }
            }
            fields.insert(OVERFLOW.to_owned(), json!(maximum));
            *left = Value::Object(fields);
        }
        (left, right) => *left = json!([std::mem::take(left), right]),
    }
}

fn compact(value: &mut Value) {
    match value {
        Value::Array(items) => {
            items.retain(|v| !v.is_null());
            items.iter_mut().for_each(compact);
        }
        Value::Object(fields) => {
            fields.remove(OVERFLOW);
            fields.values_mut().for_each(compact);
        }
        _ => {}
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
    let mut fields = Map::new();
    let limit = if query { 20 } else { count.max(100) };
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
        let value = Value::String(form_decode(value));
        if let Some(old) = fields.get_mut(&key) {
            *old = combine(std::mem::take(old), value, limit);
        } else {
            fields.insert(key, value);
        }
    }
    let mut result = json!({});
    for (key, mut value) in fields {
        if key.is_empty() {
            continue;
        }
        let mut segments = Vec::new();
        let mut offset = 0;
        let mut opening = None;
        let mut depth = 0;
        for (end, character) in key.char_indices() {
            if character == '[' {
                opening = Some(end);
            } else if character == ']'
                && let Some(start) = opening.take()
            {
                depth += 1;
                if depth > if query { 5 } else { 32 } {
                    if !query {
                        return Err(400);
                    }
                    segments.push((&key[start..], true));
                    break;
                }
                if segments.is_empty() && start > 0 {
                    segments.push((&key[..start], false));
                }
                segments.push((&key[start + 1..end], true));
                offset = end + 1;
            }
        }
        if offset == 0 {
            segments.push((&key, false));
        }
        for (key, bracket) in segments.into_iter().rev() {
            value = if key == "__proto__" {
                json!({})
            } else if bracket && key.is_empty() {
                if overflow_index(&value).is_some() {
                    value
                } else {
                    combine(json!([]), value, limit)
                }
            } else if bracket && let Some(index) = numeric_index(key) {
                if index < limit as f64 {
                    let mut items = vec![Value::Null; index as usize + 1];
                    items[index as usize] = value;
                    Value::Array(items)
                } else {
                    Value::Object(Map::from_iter([
                        (key.to_owned(), value),
                        (OVERFLOW.to_owned(), json!(index)),
                    ]))
                }
            } else {
                Value::Object(Map::from_iter([(key.to_owned(), value)]))
            };
        }
        merge(&mut result, value, query, limit);
    }
    compact(&mut result);
    Ok(result)
}
