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
    let mut parts = value.split(';');
    let media = parts.next()?.trim().to_ascii_lowercase();
    let token = |s: &str| {
        !s.is_empty()
            && s.bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b))
    };
    let (kind, subtype) = media.split_once('/')?;
    if !token(kind) || !token(subtype) {
        return None;
    }
    let mut charset = "utf-8".to_owned();
    for parameter in parts {
        let (key, value) = parameter.trim().split_once('=')?;
        let value = value.trim();
        let value = if value.starts_with('"') && value.ends_with('"') && value.len() >= 2 {
            &value[1..value.len() - 1]
        } else if token(value) {
            value
        } else {
            return None;
        };
        if !token(key) {
            return None;
        }
        if key.eq_ignore_ascii_case("charset") {
            charset = value.to_ascii_lowercase();
        }
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
        String::from_utf16_lossy(&words)
    };
    text.strip_prefix('\u{feff}').unwrap_or(&text).to_owned()
}

pub async fn read(request: Request<Incoming>, json_limit: usize) -> Result<Body, u16> {
    let headers = request.headers();
    let media = headers
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .and_then(content_type);
    let Some((media, charset)) = media.filter(|_| {
        headers.contains_key("content-length") || headers.contains_key("transfer-encoding")
    }) else {
        return Ok(Body::Json(json!({})));
    };
    let json = media == "application/json";
    let form = media == "application/x-www-form-urlencoded";
    if (json
        && ![
            "utf-8",
            "utf-16le",
            "utf-16",
            "utf-16be",
            "utf-7",
            "utf-7imap",
        ]
        .contains(&charset.as_str()))
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
            parse_form(&text)?
        } else if text.is_empty() {
            json!({})
        } else {
            match serde_json::from_str(&text) {
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
                finished = false;
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
        return value;
    }
    percent_decode_str(&value)
        .decode_utf8()
        .map(|decoded| decoded.into_owned())
        .unwrap_or(value)
}

fn merge(target: &mut Value, source: Value) {
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
                    merge(&mut left[index], value);
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
            merge(left, right);
        }
        (Value::Object(left), Value::Object(right)) => {
            for (key, value) in right {
                if let Some(old) = left.get_mut(&key) {
                    merge(old, value);
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
            merge(left, right);
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

fn parse_form(text: &str) -> Result<Value, u16> {
    if text.is_empty() {
        return Ok(json!({}));
    }
    let count = text.bytes().filter(|&b| b == b'&').count() + 1;
    if count > 1000 {
        return Err(413);
    }
    let mut fields = Map::new();
    for pair in text.split('&') {
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
                if depth > 32 {
                    return Err(400);
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
                if value.is_array() {
                    value
                } else {
                    json!([value])
                }
            } else if bracket
                && let Ok(index) = key.parse::<usize>()
                && index.to_string() == key
                && index < count.max(100)
            {
                let mut items = vec![Value::Null; index + 1];
                items[index] = value;
                Value::Array(items)
            } else {
                Value::Object(Map::from_iter([(key.to_owned(), value)]))
            };
        }
        // shortcut: qs overflow-array collision metadata is not modeled; extend before accepting that exotic form shape.
        merge(&mut result, value);
    }
    compact(&mut result);
    Ok(result)
}
