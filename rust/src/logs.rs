//! Durable log capture shared by native business handlers and script hosts.
use crate::{
    api,
    js_language::Js,
    json as codec, resources, settings,
    state::{Store, truthy},
};
use hyper::Method;
use rquickjs::Value as JsValue;
use serde_json::{Value, json};
use std::{
    io,
    sync::{Arc, Mutex},
};

pub(crate) const KEY: &str = "sub-store-logs";

fn maximum(settings: &Value) -> usize {
    let raw = &settings["logsMaxCount"];
    let number = codec::to_number(raw);
    if !raw.is_null() && raw != "" && number.is_finite() && number >= 0.0 {
        number.floor().min(10000.0) as usize
    } else {
        0
    }
}

fn read(root: &Value) -> Vec<Value> {
    let raw = &root[KEY];
    let parsed = raw.as_str().and_then(|text| codec::parse(text).ok());
    parsed
        .as_ref()
        .unwrap_or(raw)
        .as_array()
        .into_iter()
        .flatten()
        .filter(|entry| {
            entry["id"].is_string()
                && entry["time"].is_number()
                && entry["level"].is_string()
                && entry["message"].is_string()
        })
        .cloned()
        .collect()
}

fn persist(store: &mut Store, root: &Value, entries: &[Value]) -> io::Result<Value> {
    let raw = json!(codec::stringify(&json!(entries)));
    let mut next = root.clone();
    next[KEY] = raw.clone();
    store.script_commit(true, root, &next)?;
    Ok(raw)
}

fn scope(text: &str) -> Option<(&str, &str)> {
    let (name, rest) = text.strip_prefix('[')?.split_once(']')?;
    if name.is_empty() || name.contains(['\r', '\n']) {
        return None;
    }
    Some((name, rest))
}

fn prefix(text: &str) -> Option<&str> {
    let (_, rest) = scope(text)?;
    if !rest.starts_with(codec::is_space) {
        return None;
    }
    let rest = rest.trim_start_matches(codec::is_space);
    let (level, content) = rest.split_once(':')?;
    ["LOG", "INFO", "WARN", "ERROR", "DEBUG"]
        .iter()
        .any(|candidate| level.eq_ignore_ascii_case(candidate))
        .then(|| content.trim_start_matches(codec::is_space))
}

fn message(text: &str, level: &str) -> String {
    if prefix(text).is_some() {
        return text.to_owned();
    }
    let (name, content) = scope(text).map_or(("unknown", text), |(name, rest)| {
        let name = codec::trim(name);
        (
            if name.is_empty() { "unknown" } else { name },
            rest.trim_start_matches(codec::is_space),
        )
    });
    format!("[{name}] {}: {content}", level.to_ascii_uppercase())
}

fn banner(text: &str) -> bool {
    let Some(rest) = text.strip_prefix("Sub-Store") else {
        return false;
    };
    if !rest.starts_with(codec::is_space) {
        return false;
    }
    let Some(rest) = rest.trim_start_matches(codec::is_space).strip_prefix("--") else {
        return false;
    };
    if !rest.starts_with(codec::is_space) {
        return false;
    }
    let Some(version) = rest.trim_start_matches(codec::is_space).strip_prefix('v') else {
        return false;
    };
    let parts: Vec<_> = version.split('.').collect();
    (1..=4).contains(&parts.len())
        && parts
            .iter()
            .all(|part| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit()))
}

fn ignored(text: &str) -> bool {
    let mut lines: Vec<_> = text
        .split('\n')
        .map(codec::trim)
        .filter(|line| !line.is_empty())
        .collect();
    if let Some(first) = lines.first_mut()
        && let Some(content) = prefix(first)
    {
        *first = codec::trim(content);
    }
    if lines.first().is_some_and(|line| line.is_empty()) {
        lines.remove(0);
    }
    if lines.len() == 1 {
        let line = lines[0];
        if banner(line) {
            return true;
        }
        let line = line
            .strip_prefix("[sub-store]")
            .and_then(|rest| {
                ["LOG", "INFO", "WARN", "ERROR", "DEBUG"]
                    .iter()
                    .find_map(|level| {
                        if !rest.starts_with(codec::is_space) {
                            return None;
                        }
                        rest.trim_start_matches(codec::is_space)
                            .strip_prefix(&format!("{level}:"))
                            .map(|content| content.trim_start_matches(codec::is_space))
                    })
            })
            .unwrap_or(line);
        let parser = line
            .strip_suffix('!')
            .unwrap_or(line)
            .strip_suffix(" is activated");
        let parser = parser.is_some_and(|name| {
            name.strip_suffix(" Parser")
                .is_some_and(|name| !name.is_empty())
                || name
                    .strip_prefix("Rule parser [")
                    .and_then(|name| name.strip_suffix(']'))
                    .is_some_and(|name| !name.is_empty())
        });
        let pre = line
            .strip_suffix('!')
            .unwrap_or(line)
            .strip_suffix(" activated")
            .and_then(|name| name.strip_prefix("Pre-processor ["))
            .and_then(|name| name.strip_suffix(']'))
            .is_some_and(|name| !name.is_empty());
        return parser
            || pre
            || line
                == "Fallback Base64 Pre-processor error: decoded line does not start with protocol"
            || line
                .strip_prefix("[CORS] allowed origins: ")
                .is_some_and(|rest| !rest.is_empty());
    }
    let separator = |line: &str| line.chars().count() >= 8 && line.chars().all(|c| c == '\u{2505}');
    lines.len() >= 3
        && separator(lines[0])
        && separator(lines[lines.len() - 1])
        && lines[1..lines.len() - 1].iter().any(|line| banner(line))
}

fn cached_count(store: &mut Store) -> usize {
    let now = resources::now();
    match store.logs_max_cache {
        Some((expires, count)) if now < expires => count,
        _ => {
            let count = maximum(&store.data["settings"]);
            store.logs_max_cache = Some((now.saturating_add(1000), count));
            count
        }
    }
}

pub(crate) fn enabled(store: &mut Store) -> bool {
    cached_count(store) != 0
}

pub(crate) fn append(store: &mut Store, level: &str, text: &str) -> Option<Value> {
    let count = cached_count(store);
    if count == 0 || ignored(text) {
        return None;
    }
    let now = resources::now();
    let result = (|| {
        let root = store.script_root()?;
        let mut entries = read(&root);
        let mut random = [0; 8];
        getrandom::fill(&mut random).map_err(|error| io::Error::other(error.to_string()))?;
        let suffix: String = random
            .into_iter()
            .map(|byte| char::from_digit((byte % 36) as u32, 36).unwrap())
            .collect();
        let level = if ["log", "info", "warn", "error", "debug"].contains(&level) {
            level
        } else {
            "log"
        };
        entries.push(json!({"id":format!("{now}-{suffix}"),"time":resources::now(),"level":level,"message":message(text,level)}));
        if entries.len() > count {
            entries.drain(..entries.len() - count);
        }
        persist(store, &root, &entries)
    })();
    result.ok()
}

pub(crate) fn info(store: &mut Store, text: &str) {
    eprintln!("[sub-store] INFO: {text}");
    let _ = append(store, "log", &format!("[sub-store] INFO: {text}"));
}

pub(crate) fn error(store: &mut Store, text: &str) {
    eprintln!("[sub-store] ERROR: {text}");
    let _ = append(store, "log", &format!("[sub-store] ERROR: {text}"));
}

fn boolean(value: &Value) -> bool {
    value == true || value == 1 || value == "1" || value == "true" || value == "on"
}

fn regex_reason(pattern: &str, reason: String) -> String {
    // Keep the established API wording for unmatched character classes.
    let (mut class, mut escaped) = (false, false);
    for c in pattern.chars() {
        if escaped {
            escaped = false;
            continue;
        }
        match c {
            '\\' => escaped = true,
            '[' if !class => class = true,
            ']' if class => class = false,
            _ => (),
        }
    }
    if class {
        format!("Invalid regular expression: /{pattern}/: Unterminated character class")
    } else {
        reason
    }
}

pub(crate) fn query<'js>(
    js: &Js<'js>,
    query: JsValue<'js>,
    store: &Arc<Mutex<Store>>,
) -> rquickjs::Result<JsValue<'js>> {
    let query = crate::js_values::project(&js.ctx, query)?;
    let count = maximum(&store.lock().unwrap().data["settings"]);
    let result = crate::script_pipeline::object(js)?;
    if count == 0 {
        js.set(
            &result,
            "data",
            crate::js_values::decode(&js.ctx, &json!({"logs":[],"total":0,"maxCount":0}))?,
        )?;
        return Ok(result);
    }
    let limit = codec::to_number(&query["limit"]);
    let limit = if limit.is_finite() && limit > 0.0 {
        limit.floor().min(10000.0) as usize
    } else {
        count
    };
    let keyword = if truthy(&query["keyword"]) {
        codec::js_string(&query["keyword"])
    } else {
        String::new()
    };
    let keyword = codec::trim(&keyword);
    let ignore_case = boolean(&query["ignoreCase"]);
    let regex = if boolean(&query["regex"]) && !keyword.is_empty() {
        match js.call::<JsValue, _>("regex", (keyword, if ignore_case { "i" } else { "" })) {
            Ok(regex) => Some(regex),
            Err(error) => {
                let reason = regex_reason(keyword, crate::script::message(&js.ctx, error));
                let error = json!({"code":"INVALID_LOG_KEYWORD_REGEX","type":"RequestInvalidError","message":"Invalid log keyword regular expression","details":format!("Reason: {reason}")});
                js.set(&result, "error", crate::js_values::decode(&js.ctx, &error)?)?;
                js.set(&result, "status", js.value(400)?)?;
                return Ok(result);
            }
        }
    } else {
        None
    };
    let mut entries = {
        let mut store = store.lock().unwrap();
        let root = store.script_root().unwrap_or_else(|_| json!({}));
        let entries = read(&root);
        let mut sanitized: Vec<_> = entries
            .iter()
            .filter(|entry| !ignored(entry["message"].as_str().unwrap()))
            .cloned()
            .collect();
        if sanitized.len() > count {
            sanitized.drain(..sanitized.len() - count);
        }
        if sanitized.len() != entries.len() {
            persist(&mut store, &root, &sanitized).map_err(|error| js.error(&error.to_string()))?;
        }
        sanitized
    };
    if !keyword.is_empty() {
        let mut needle = crate::js_values::decode(&js.ctx, &json!(keyword))?;
        if ignore_case {
            needle = js.method(needle, "toLowerCase", vec![])?;
        }
        let mut filtered = Vec::new();
        for entry in entries {
            let text = format!(
                "{}\n{}",
                entry["level"].as_str().unwrap(),
                entry["message"].as_str().unwrap()
            );
            let mut text = crate::js_values::decode(&js.ctx, &json!(text))?;
            let matches = if let Some(regex) = &regex {
                js.truthy(&js.method(regex.clone(), "test", vec![text])?)?
            } else {
                if ignore_case {
                    text = js.method(text, "toLowerCase", vec![])?;
                }
                js.truthy(&js.method(text, "includes", vec![needle.clone()])?)?
            };
            if matches {
                filtered.push(entry);
            }
        }
        entries = filtered;
    }
    let total = entries.len();
    if limit != 0 && entries.len() > limit {
        entries.drain(..entries.len() - limit);
    }
    entries.reverse();
    js.set(
        &result,
        "data",
        crate::js_values::decode(
            &js.ctx,
            &json!({"logs":entries,"total":total,"maxCount":count}),
        )?,
    )?;
    Ok(result)
}

pub async fn dispatch(
    method: &Method,
    path: &str,
    query: &Value,
    store: &Arc<Mutex<Store>>,
) -> Option<api::Reply> {
    if path != "/api/logs" {
        return None;
    }
    if method == Method::DELETE {
        let result = {
            let mut store = store.lock().unwrap();
            store
                .script_root()
                .and_then(|root| persist(&mut store, &root, &[]))
        };
        return Some(result.map_or_else(
            |error| {
                resources::Error::internal("FAILED_TO_CLEAR_LOGS", "Failed to clear logs", error)
                    .reply(500)
            },
            |_| api::success(None, 200),
        ));
    }
    if method != Method::GET {
        return None;
    }
    Some(
        match settings::job(store, "logs-query", query.clone()).await {
            Ok(result) if result.get("error").is_some() => api::send(
                result["status"].as_u64().unwrap_or(500) as u16,
                "text/plain; charset=utf-8",
                codec::stringify(&json!({"status":"failed","error":result["error"]})),
            ),
            Ok(result) => api::success(result.get("data").cloned(), 200),
            Err(error) => resources::Error::internal(
                "FAILED_TO_GET_LOGS",
                "Failed to get logs",
                error.message,
            )
            .reply(500),
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn normalize_and_ignore_source_noise() {
        assert_eq!(
            message("[ my scope ] value", "warn"),
            "[my scope] WARN: value"
        );
        assert_eq!(
            message("[scope] iNfO: value", "warn"),
            "[scope] iNfO: value"
        );
        assert_eq!(message("plain🙂", "debug"), "[unknown] DEBUG: plain🙂");
        for text in [
            "Sub-Store -- v2.42.4",
            "[scope] INFO: URI Parser is activated!",
            "Pre-processor [A] activated",
            "[sub-store] INFO: [CORS] allowed origins: *",
            "┅┅┅┅┅┅┅┅\nSub-Store -- v1.0\n┅┅┅┅┅┅┅┅",
        ] {
            assert!(ignored(text), "{text}");
        }
        for text in [
            "",
            "Parser is activated",
            "Sub-Store -- v1.2.3.4.5",
            "URI Parser is activated!\nmeaningful",
            "[unknown] WARN: real failure",
        ] {
            assert!(!ignored(text), "{text}");
        }
    }
}
