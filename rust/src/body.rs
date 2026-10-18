use crate::json as codec;
use flate2::{Decompress, FlushDecompress, Status};
use http_body_util::{BodyExt, Limited};
use hyper::{Request, body::Bytes, body::Incoming};
use percent_encoding::percent_decode_str;
use serde_json::{Map, Value, json};

pub enum Body {
    Json(Value),
    Raw(Bytes),
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

pub async fn read(request: Request<Incoming>, json_limit: usize) -> Result<Body, u16> {
    let headers = request.headers();
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
        return Err(415);
    }
    let encoding = headers
        .get("content-encoding")
        .and_then(|v| v.to_str().ok())
        .filter(|v| !v.is_empty())
        .unwrap_or("identity")
        .to_ascii_lowercase();
    if !["identity", "gzip", "deflate"].contains(&encoding.as_str()) {
        return Err(415);
    }
    let limit = if json { json_limit } else { 100 * 1024 };
    let bytes = if encoding == "identity" {
        Limited::new(request.into_body(), limit)
            .collect()
            .await
            .map_err(|_| 413u16)?
            .to_bytes()
    } else {
        inflate(request.into_body(), encoding == "gzip", limit).await?
    };
    // The legacy rawBodySaver uses Buffer.toString, which rejects these iconv-supported charsets.
    if json && !bytes.is_empty() && !["utf-8", "utf-16le"].contains(&charset.as_str()) {
        return Err(403);
    }
    if json || form {
        let text = decode(&bytes, &charset);
        let value = if form {
            parse_fields(&text, false)?
        } else if text.is_empty() {
            json!({})
        } else {
            match codec::parse(&text) {
                Ok(value @ (Value::Object(_) | Value::Array(_))) => value,
                _ => return Err(400),
            }
        };
        Ok(Body::Json(value))
    } else {
        Ok(Body::Raw(bytes))
    }
}

async fn inflate(mut body: Incoming, gzip: bool, limit: usize) -> Result<Bytes, u16> {
    let mut decoder = if gzip {
        Decompress::new_gzip(15)
    } else {
        Decompress::new(true)
    };
    let mut output = Vec::new();
    let mut finished = false;
    let mut padding = false;
    while let Some(frame) = body.frame().await {
        let frame = frame.map_err(|_| 400u16)?;
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
                return Err(413);
            }
            let status = status.map_err(|_| 400u16)?;
            output.extend_from_slice(&decoded[..produced]);
            input = &input[consumed..];
            finished = status == Status::StreamEnd;
            if consumed == 0 && produced == 0 && !finished {
                if !input.is_empty() {
                    return Err(400);
                }
                break;
            }
        }
    }
    if !finished {
        return Err(400);
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

fn merge(target: &mut Value, source: Value, query: bool) {
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
                    merge(&mut left[index], value, query);
                } else {
                    left.push(value);
                }
            }
        }
        (left @ Value::Array(_), right @ Value::Object(_)) => {
            let items = std::mem::take(left).as_array().unwrap().clone();
            *left = Value::Object(
                items
                    .into_iter()
                    .enumerate()
                    .filter(|(_, v)| !v.is_null())
                    .map(|(i, v)| (i.to_string(), v))
                    .collect(),
            );
            merge(left, right, query);
        }
        (Value::Object(left), Value::Object(right)) => {
            for (key, value) in right {
                if let Some(old) = left.get_mut(&key) {
                    merge(old, value, query);
                } else {
                    left.insert(key, value);
                }
            }
        }
        (left @ Value::Object(_), Value::Array(right)) => {
            let right = Value::Object(
                right
                    .into_iter()
                    .enumerate()
                    .filter(|(_, v)| !v.is_null())
                    .map(|(i, v)| (i.to_string(), v))
                    .collect(),
            );
            merge(left, right, query);
        }
        (Value::Object(left), Value::String(right)) if query => {
            if !right.is_empty() {
                left.insert(right, Value::Bool(true));
            }
        }
        (Value::Array(left), right) => left.push(right),
        (left, Value::Array(mut right)) => {
            right.insert(0, std::mem::take(left));
            *left = Value::Array(right);
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
        Value::Object(fields) => fields.values_mut().for_each(compact),
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
            if let Value::Array(items) = old {
                items.push(value);
            } else {
                *old = json!([std::mem::take(old), value]);
            }
        } else {
            fields.insert(key, value);
        }
    }
    if query {
        for value in fields.values_mut() {
            if let Value::Array(items) = value
                && items.len() > 20
            {
                *value = Value::Object(
                    std::mem::take(items)
                        .into_iter()
                        .enumerate()
                        .map(|(index, value)| (index.to_string(), value))
                        .collect(),
                );
            }
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
                if value.is_array() || (query && value.get("20").is_some()) {
                    value
                } else {
                    json!([value])
                }
            } else if bracket
                && let Ok(index) = key.parse::<usize>()
                && index.to_string() == key
                && index < if query { 20 } else { count.max(100) }
            {
                let mut items = vec![Value::Null; index + 1];
                items[index] = value;
                Value::Array(items)
            } else {
                Value::Object(Map::from_iter([(key.to_owned(), value)]))
            };
        }
        // shortcut: qs overflow-array collision metadata is not modeled; extend before accepting that exotic shape.
        merge(&mut result, value, query);
    }
    compact(&mut result);
    Ok(result)
}
