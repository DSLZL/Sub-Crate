mod api;
mod body;
mod clash;
mod clash_output;
mod classic;
mod conversion;
mod dns_transport;
mod egern_output;
mod js_language;
mod json;
mod mmdb;
mod processors;
mod rules;
mod script;
mod script_age;
mod script_conversion;
mod script_download;
mod script_dns;
mod script_flow;
mod script_helpers;
mod script_http;
mod script_io;
mod script_loading;
mod script_mmdb;
mod script_network;
mod script_pipeline;
mod singbox_output;
mod state;
mod surge;
mod surge_output;
mod text_output;
mod xhttp;

use hyper::body::{Body, Incoming};
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{HeaderMap, Method, Request};
use hyper_util::rt::TokioIo;
use percent_encoding::percent_decode_str;
use serde_json::{Value, json};
use std::cmp::Reverse;
use std::convert::Infallible;
use std::env;
use std::io;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, UNIX_EPOCH};
use tokio::net::TcpListener;

struct Config {
    prefix: Option<String>,
    frontend: Option<PathBuf>,
    merge: bool,
    frontend_backend_path: String,
    cors: Vec<String>,
    local_origins: bool,
    body_limit: usize,
    environment: Value,
    powered_by: String,
}

fn configured(name: &str, default: &str) -> String {
    env::var(name)
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| default.to_owned())
}

fn origin(value: &str) -> String {
    let value = json::trim(value).trim_matches(['\'', '"']);
    reqwest::Url::parse(value)
        .map(|url| url.origin().ascii_serialization())
        .unwrap_or_else(|_| value.to_owned())
}

fn json_limit(raw: &str) -> io::Result<usize> {
    let raw = raw.to_ascii_lowercase();
    let scaled = ["kb", "mb", "gb", "tb", "pb"]
        .iter()
        .enumerate()
        .find_map(|(index, unit)| {
            let number = raw.strip_suffix(*unit)?.trim_end_matches(' ');
            let unsigned = number.trim_start_matches(['+', '-']);
            let mut parts = unsigned.split('.');
            let whole = parts.next().unwrap();
            let fraction = parts.next();
            let digits = |s: &str| !s.is_empty() && s.bytes().all(|c| c.is_ascii_digit());
            if !digits(whole) || !fraction.is_none_or(digits) || parts.next().is_some() {
                return None;
            }
            number
                .parse::<f64>()
                .ok()
                .map(|n| n * 1024f64.powi(index as i32 + 1))
        });
    let bytes = scaled.or_else(|| {
        let text = json::trim(&raw);
        let unsigned = text.strip_prefix(['+', '-']).unwrap_or(text);
        let end = unsigned
            .find(|c: char| !c.is_ascii_digit())
            .unwrap_or(unsigned.len());
        let number = unsigned[..end].parse::<f64>().ok()?;
        Some(if text.starts_with('-') {
            -number
        } else {
            number
        })
    });
    match bytes {
        Some(bytes) if bytes.is_finite() && bytes >= 0.0 && bytes <= usize::MAX as f64 => {
            Ok(bytes.floor() as usize)
        }
        _ => Err(io::Error::other("Invalid SUB_STORE_BODY_JSON_LIMIT")),
    }
}

impl Config {
    fn load() -> io::Result<Self> {
        let merge = !configured("SUB_STORE_BACKEND_MERGE", "").is_empty();
        let prefixed = !configured("SUB_STORE_BACKEND_PREFIX", "").is_empty();
        let backend_path = configured("SUB_STORE_FRONTEND_BACKEND_PATH", "");
        let frontend = env::var("SUB_STORE_FRONTEND_PATH")
            .ok()
            .filter(|s| !s.is_empty())
            .map(fs_root)
            .transpose()?;
        if (merge || prefixed || (frontend.is_some() && !backend_path.is_empty()))
            && !backend_path.starts_with('/')
        {
            return Err(io::Error::other(
                "SUB_STORE_FRONTEND_BACKEND_PATH must start with / when a backend mount is configured",
            ));
        }
        let allowed = configured("SUB_STORE_CORS_ALLOWED_ORIGINS", "");
        let mut cors: Vec<_> = allowed
            .split(|c: char| c == ',' || c == '，' || json::trim(&c.to_string()).is_empty())
            .filter(|s| !s.is_empty())
            .map(origin)
            .collect();
        let default_cors = cors.is_empty();
        if default_cors {
            cors = [
                "https://sub-store.vercel.app",
                "http://substore.stash",
                "https://substore.stash",
            ]
            .map(str::to_owned)
            .to_vec();
        }
        let body_limit = json_limit(&configured("SUB_STORE_BODY_JSON_LIMIT", "1mb"))?;
        let environment = Value::Object(
            env::vars()
                .filter(|(key, _)| key.starts_with("SUB_STORE_"))
                .map(|(key, value)| (json::encode(&key), json!(json::encode(&value))))
                .collect(),
        );
        Ok(Self {
            prefix: (merge || prefixed).then(|| backend_path.clone()),
            frontend,
            merge,
            frontend_backend_path: backend_path,
            cors,
            local_origins: default_cors
                && !configured("SUB_STORE_BACKEND_CUSTOM_NAME", "").is_empty(),
            body_limit,
            environment,
            powered_by: configured("SUB_STORE_X_POWERED_BY", "Sub-Store"),
        })
    }

    fn allowed(&self, raw: &str) -> bool {
        if raw.is_empty() || self.cors.iter().any(|s| s == "*") {
            return true;
        }
        let normalized = origin(raw);
        if self.cors.contains(&normalized) {
            return true;
        }
        self.local_origins
            && reqwest::Url::parse(&normalized).is_ok_and(|url| {
                url.scheme() == "http" && matches!(url.host_str(), Some("localhost" | "127.0.0.1"))
            })
    }

    fn headers(&self, response: &mut api::Reply, raw_origin: &str) {
        let headers = response.headers_mut();
        headers.insert(
            "access-control-allow-methods",
            "POST,GET,OPTIONS,PATCH,PUT,DELETE".parse().unwrap(),
        );
        headers.insert(
            "access-control-allow-headers",
            "Origin, X-Requested-With, Content-Type, Accept"
                .parse()
                .unwrap(),
        );
        if let Ok(value) = self.powered_by.parse() {
            headers.insert("x-powered-by", value);
        }
        if self.allowed(raw_origin) {
            if self.cors.iter().any(|s| s == "*") {
                headers.insert("access-control-allow-origin", "*".parse().unwrap());
            } else if !raw_origin.is_empty() {
                if let Ok(value) = origin(raw_origin).parse() {
                    headers.insert("access-control-allow-origin", value);
                }
                headers.append("vary", "Origin".parse().unwrap());
            }
        }
    }
}

fn fs_root(path: String) -> io::Result<PathBuf> {
    std::fs::canonicalize(path)
}

fn backend_route(path: &str) -> bool {
    ["/api", "/download", "/share"]
        .iter()
        .any(|prefix| path == *prefix || path.starts_with(&format!("{prefix}/")))
}

fn header<'a>(headers: &'a HeaderMap, name: &str) -> &'a str {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("")
}

fn gzip_preferred(headers: &HeaderMap) -> bool {
    let raw = header(headers, "accept-encoding");
    let mut accepted = Vec::new();
    for (index, part) in raw.split(',').enumerate() {
        let mut fields = part.trim().split(';');
        let coding = fields.next().unwrap().trim();
        if coding.is_empty() || coding.chars().any(char::is_whitespace) {
            continue;
        }
        let quality = fields
            .find_map(|field| {
                let (key, value) = field.trim().split_once('=')?;
                if key != "q" {
                    return None;
                }
                let value = value.trim();
                let quality = (0..=value.len())
                    .rev()
                    .filter(|end| value.is_char_boundary(*end))
                    .find_map(|end| value[..end].parse::<f64>().ok())
                    .unwrap_or(0.0);
                Some(if quality.is_nan() { 0.0 } else { quality })
            })
            .unwrap_or(1.0);
        accepted.push((coding, quality, index));
    }
    // The locked negotiator adds identity at the lowest nonzero quality.
    if !accepted
        .iter()
        .any(|(coding, _, _)| *coding == "*" || coding.eq_ignore_ascii_case("identity"))
    {
        let quality = accepted
            .iter()
            .map(|(_, quality, _)| if *quality == 0.0 { 1.0 } else { *quality })
            .fold(1.0, f64::min);
        accepted.push(("identity", quality, raw.split(',').count()));
    }
    let priority = |name: &str| {
        accepted
            .iter()
            .filter_map(|(coding, quality, index)| {
                if coding.eq_ignore_ascii_case(name) {
                    Some((1, *quality, *index))
                } else if *coding == "*" {
                    Some((0, *quality, *index))
                } else {
                    None
                }
            })
            .max_by(|a, b| a.partial_cmp(b).unwrap())
            .map(|(specificity, quality, index)| (quality, specificity, Reverse(index)))
            .unwrap_or((0.0, 0, Reverse(usize::MAX)))
    };
    let gzip = priority("gzip");
    gzip.0 > 0.0 && gzip >= priority("identity")
}

fn etag_match(raw: &str, etag: &str) -> bool {
    raw == "*"
        || raw.split(',').any(|part| {
            let part = part.trim_matches(' ');
            part == etag
                || part.strip_prefix("W/") == Some(etag)
                || etag.strip_prefix("W/") == Some(part)
        })
}

fn fresh(headers: &HeaderMap, etag: &str, modified: Option<std::time::SystemTime>) -> bool {
    let matched = header(headers, "if-none-match");
    let since = header(headers, "if-modified-since");
    if (matched.is_empty() && since.is_empty())
        || header(headers, "cache-control")
            .split(',')
            .any(|part| part.trim() == "no-cache")
    {
        return false;
    }
    if !matched.is_empty() && !etag_match(matched, etag) {
        return false;
    }
    since.is_empty()
        || modified
            .zip(httpdate::parse_http_date(since).ok())
            .is_some_and(|(modified, date)| modified <= date)
}

fn static_range(
    headers: &HeaderMap,
    etag: &str,
    modified: std::time::SystemTime,
    length: usize,
) -> Result<Option<(usize, usize)>, ()> {
    let Some(raw) = header(headers, "range")
        .trim_start_matches(' ')
        .strip_prefix("bytes=")
    else {
        return Ok(None);
    };
    let condition = header(headers, "if-range");
    if !condition.is_empty()
        && !(if condition.contains('"') {
            condition.contains(etag)
        } else {
            httpdate::parse_http_date(condition).is_ok_and(|date| modified <= date)
        })
    {
        return Ok(None);
    }
    // The locked range-parser uses parseInt, including numeric prefixes.
    let number = |raw: &str| {
        let raw = raw.trim_start();
        let prefix = raw
            .char_indices()
            .take_while(|(index, ch)| {
                ch.is_ascii_digit() || (*index == 0 && matches!(*ch, '+' | '-'))
            })
            .map(|(index, ch)| index + ch.len_utf8())
            .last()?;
        raw[..prefix].parse::<f64>().ok()
    };
    let last = length as f64 - 1.0;
    let mut ranges = Vec::new();
    for part in raw.split(',') {
        let mut fields = part.split('-');
        let start = number(fields.next().unwrap());
        let end = number(fields.next().unwrap_or(""));
        let (start, end) = match (start, end) {
            (Some(start), end) => (start, end.unwrap_or(last).min(last)),
            (None, Some(suffix)) => (length as f64 - suffix, last),
            _ => continue,
        };
        if start >= 0.0 && start <= end {
            ranges.push((start as usize, end as usize));
        }
    }
    ranges.sort_unstable();
    let Some(&(start, mut end)) = ranges.first() else {
        return Err(());
    };
    for &(next, last) in &ranges[1..] {
        if next > end.saturating_add(1) {
            return Ok(None);
        }
        end = end.max(last);
    }
    Ok(Some((start, end)))
}

fn static_file(root: &Path, path: &str, headers: &HeaderMap, fallback: bool) -> api::Reply {
    let relative = Path::new(path.trim_start_matches('/'));
    let safe = !path.contains(['\\', '\0'])
        && !relative.components().any(|part| match part {
            Component::Normal(name) => name.to_string_lossy().starts_with('.'),
            _ => true,
        });
    let requested = if safe {
        root.join(relative)
    } else if fallback {
        root.join("index.html")
    } else {
        return api::not_found();
    };
    let gzip = gzip_preferred(headers);
    let mut selected = None;
    for logical in std::iter::once(requested).chain(fallback.then(|| root.join("index.html"))) {
        if gzip && !logical.is_dir() {
            let compressed = logical.with_file_name(format!(
                "{}.gz",
                logical.file_name().unwrap().to_string_lossy()
            ));
            if compressed.is_file() {
                selected = Some(compressed);
                break;
            }
        }
        if logical.is_file() {
            selected = Some(logical);
            break;
        }
        if logical.is_dir() && logical.join("index.html").is_file() {
            selected = Some(logical.join("index.html"));
            break;
        }
    }
    let Some(selected) = selected else {
        return api::not_found();
    };
    let Ok(file) = selected.canonicalize() else {
        return api::not_found();
    };
    if !file.starts_with(root) {
        return api::text(403, "text/plain; charset=utf-8", "");
    }
    let Ok(metadata) = std::fs::metadata(&file) else {
        return api::not_found();
    };
    let Ok(elapsed) = metadata
        .modified()
        .and_then(|date| date.duration_since(UNIX_EPOCH).map_err(io::Error::other))
    else {
        return api::not_found();
    };
    let modified = UNIX_EPOCH + Duration::from_secs(elapsed.as_secs());
    let etag = format!(
        "W/\"{:x}-{:x}\"",
        metadata.len(),
        (elapsed.as_nanos() + 500_000) / 1_000_000
    );
    let matched = header(headers, "if-match");
    let precondition_failed = if !matched.is_empty() {
        !etag_match(matched, &etag)
    } else {
        httpdate::parse_http_date(header(headers, "if-unmodified-since"))
            .is_ok_and(|date| modified > date)
    };
    let fresh = !precondition_failed && fresh(headers, &etag, Some(modified));
    let range = if precondition_failed || fresh {
        Ok(None)
    } else {
        static_range(headers, &etag, modified, metadata.len() as usize)
    };
    let status = if precondition_failed {
        412
    } else if fresh {
        304
    } else {
        match range {
            Ok(Some(_)) => 206,
            Ok(None) => 200,
            Err(()) => 416,
        }
    };
    let bytes = if status >= 400 {
        let message = if status == 412 {
            "Precondition Failed"
        } else {
            "Range Not Satisfiable"
        };
        let kind = if status == 412 {
            "PreconditionFailedError"
        } else {
            "RangeNotSatisfiableError"
        };
        let message = api::diagnostic(status, &format!("{kind}: {message}"));
        format!("<!DOCTYPE html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n<title>Error</title>\n</head>\n<body>\n<pre>{message}</pre>\n</body>\n</html>\n").into_bytes()
    } else if fresh {
        Vec::new()
    } else if let Ok(bytes) = std::fs::read(&file) {
        if let Ok(Some((start, end))) = range {
            let Some(slice) = bytes.get(start..=end) else {
                return api::not_found();
            };
            slice.to_vec()
        } else {
            bytes
        }
    } else {
        return api::not_found();
    };
    let encoding = selected
        .extension()
        .is_some_and(|extension| extension == "gz");
    let logical = if encoding {
        selected.with_extension("")
    } else {
        selected
    };
    let content_type = match logical.extension().and_then(|s| s.to_str()).unwrap_or("") {
        "html" => "text/html; charset=utf-8",
        "js" | "mjs" => "application/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" | "map" => "application/json; charset=utf-8",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "ico" => "image/x-icon",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "txt" => "text/plain; charset=utf-8",
        "wasm" => "application/wasm",
        _ => "application/octet-stream",
    };
    let name = logical.file_name().unwrap().to_string_lossy();
    let hashed = name.match_indices('-').any(|(index, _)| {
        let Some((hash, suffix)) = name[index + 1..].split_once('.') else {
            return false;
        };
        hash.len() == 8 && hash.bytes().all(|b| b.is_ascii_hexdigit()) && !suffix.is_empty()
    });
    let length = bytes.len();
    let mut response = api::text(
        status,
        if status >= 400 {
            "text/html; charset=utf-8"
        } else {
            content_type
        },
        bytes,
    );
    response.headers_mut().insert("etag", etag.parse().unwrap());
    response.headers_mut().insert(
        "last-modified",
        httpdate::fmt_http_date(modified).parse().unwrap(),
    );
    response
        .headers_mut()
        .insert("accept-ranges", "bytes".parse().unwrap());
    response
        .headers_mut()
        .insert("vary", "Accept-Encoding".parse().unwrap());
    response.headers_mut().insert(
        "cache-control",
        if hashed {
            "public, max-age=31536000, immutable"
        } else {
            "public, max-age=0"
        }
        .parse()
        .unwrap(),
    );
    if fresh {
        response.headers_mut().remove("content-type");
    } else {
        response
            .headers_mut()
            .insert("content-length", length.into());
    }
    if encoding && status < 300 {
        response
            .headers_mut()
            .insert("content-encoding", "gzip".parse().unwrap());
    }
    if status >= 400 {
        // Development-mode Node stack paths are outside the stable wire contract.
        response.headers_mut().insert(
            "content-security-policy",
            "default-src 'none'".parse().unwrap(),
        );
        response
            .headers_mut()
            .insert("x-content-type-options", "nosniff".parse().unwrap());
    }
    let content_range = match range {
        Ok(Some((start, end))) => Some(format!("bytes {start}-{end}/{}", metadata.len())),
        Err(()) => Some(format!("bytes */{}", metadata.len())),
        _ => None,
    };
    if let Some(value) = content_range {
        response
            .headers_mut()
            .insert("content-range", value.parse().unwrap());
    }
    response
}

async fn handle(
    request: Request<Incoming>,
    config: Arc<Config>,
    store: Arc<Mutex<state::Store>>,
) -> Result<api::Reply, Infallible> {
    let raw_origin = request
        .headers()
        .get("origin")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("")
        .to_owned();
    let head = request.method() == Method::HEAD;
    let read = head || request.method() == Method::GET;
    let headers = request.headers().clone();
    let response = if !config.allowed(&raw_origin) {
        api::text(403, "text/plain;charset=UTF-8", "CORS origin not allowed")
    } else if request.method() == Method::OPTIONS && !raw_origin.is_empty() {
        api::text(200, "text/plain;charset=UTF-8", "")
    } else {
        route(request, &config, &store).await
    };
    let mut response = response;
    if response
        .extensions()
        .get::<api::TransformedResponse>()
        .is_none()
    {
        config.headers(&mut response, &raw_origin);
    }
    let etag = header(response.headers(), "etag");
    if read && response.status().is_success() && !etag.is_empty() && fresh(&headers, etag, None) {
        *response.status_mut() = hyper::StatusCode::NOT_MODIFIED;
        for name in ["content-type", "content-length", "transfer-encoding"] {
            response.headers_mut().remove(name);
        }
        *response.body_mut() = http_body_util::Full::new(hyper::body::Bytes::new());
    }
    if head {
        strip_head_body(&mut response);
    }
    Ok(response)
}

fn strip_head_body(response: &mut api::Reply) {
    if !response.status().is_informational()
        && response.status().as_u16() != 204
        && response.status().as_u16() != 304
        && let Some(length) = response.body().size_hint().exact()
    {
        response
            .headers_mut()
            .insert("content-length", length.into());
    }
    *response.body_mut() = http_body_util::Full::new(hyper::body::Bytes::new());
}

async fn frontend_handle(
    mut request: Request<Incoming>,
    config: Arc<Config>,
    store: Arc<Mutex<state::Store>>,
) -> Result<api::Reply, Infallible> {
    let path = request.uri().path().to_owned();
    let mount = config.frontend_backend_path.trim_end_matches('/');
    if !config.frontend_backend_path.is_empty() {
        for namespace in ["api", "download"] {
            let prefix = format!("{mount}/{namespace}");
            if path
                .get(..prefix.len())
                .is_some_and(|part| part.eq_ignore_ascii_case(&prefix))
                && (path.len() == prefix.len() || path.as_bytes()[prefix.len()] == b'/')
            {
                let backend_prefix = config.prefix.as_deref().unwrap_or("").trim_end_matches('/');
                let suffix = &path[prefix.len()..];
                let suffix = if suffix.is_empty() { "/" } else { suffix };
                let mut target = format!("{backend_prefix}/{namespace}{suffix}");
                // The legacy proxy joins paths with collapsed slashes, preserving the query.
                let mut slash = false;
                target.retain(|ch| {
                    let keep = ch != '/' || !slash;
                    slash = ch == '/';
                    keep
                });
                if let Some(query) = request.uri().query() {
                    target.push('?');
                    target.push_str(query);
                }
                if namespace == "api" {
                    target.push_str(if request.uri().query().is_some() {
                        "&share=true"
                    } else {
                        "?share=true"
                    });
                }
                let Ok(uri) = target.parse() else {
                    return Ok(api::text(
                        400,
                        "text/plain; charset=utf-8",
                        "Invalid proxy URL",
                    ));
                };
                *request.uri_mut() = uri;
                return handle(request, config, store).await;
            }
        }
    }
    let head = request.method() == Method::HEAD;
    let mut response = api::not_found();
    if !config.frontend_backend_path.is_empty() && path.starts_with("/share/") {
        // shortcut: public share forwarding requires M4 token enforcement; keep it unavailable until then.
        response = api::text(
            501,
            "text/plain; charset=utf-8",
            "Public share proxy is not implemented yet",
        );
    } else if (request.method() == Method::GET || head)
        && let Some(root) = &config.frontend
        && let Ok(decoded) = percent_decode_str(&path).decode_utf8()
    {
        response = static_file(root, &decoded, request.headers(), false);
        let accept = header(request.headers(), "accept");
        if response.status().as_u16() == 404
            && !accept.starts_with("application/json")
            && (accept.contains("text/html") || accept.contains("*/*"))
        {
            response = static_file(root, "/index.html", request.headers(), false);
        }
    }
    if response.status().as_u16() == 404 {
        response = api::route_not_found(request.method(), &path);
    }
    response
        .headers_mut()
        .insert("x-powered-by", "Express".parse().unwrap());
    if head {
        strip_head_body(&mut response);
    }
    Ok(response)
}

fn api_path(raw: &str) -> Result<Option<String>, ()> {
    if raw.as_bytes().iter().enumerate().any(|(index, byte)| {
        *byte == b'%'
            && !raw
                .as_bytes()
                .get(index + 1..index + 3)
                .is_some_and(|pair| pair.iter().all(u8::is_ascii_hexdigit))
    }) {
        return Err(());
    }
    percent_decode_str(raw).decode_utf8().map_err(|_| ())?;
    if raw == "/" {
        return Ok(Some(raw.to_owned()));
    }
    let parts = raw
        .strip_suffix('/')
        .unwrap_or(raw)
        .split('/')
        .collect::<Vec<_>>();
    let path = match parts.as_slice() {
        ["", api, resource] if api.eq_ignore_ascii_case("api") => {
            Some(format!("/api/{}", resource.to_ascii_lowercase()))
        }
        ["", api, resource, name] if api.eq_ignore_ascii_case("api") && !name.is_empty() => {
            let resource = resource.to_ascii_lowercase();
            match resource.as_str() {
                "sub" | "collection" => Some(format!(
                    "/api/{resource}/{}",
                    json::encode(&percent_decode_str(name).decode_utf8().map_err(|_| ())?)
                )),
                "utils" => Some(format!("/api/utils/{}", name.to_ascii_lowercase())),
                "proxy" | "rule" | "preview" => {
                    Some(format!("/api/{resource}/{}", name.to_ascii_lowercase()))
                }
                _ => None,
            }
        }
        ["", download, name] if download.eq_ignore_ascii_case("download") && !name.is_empty() => {
            Some(format!("/download/{name}"))
        }
        ["", download, collection, name]
            if download.eq_ignore_ascii_case("download")
                && collection.eq_ignore_ascii_case("collection")
                && !name.is_empty() =>
        {
            Some(format!("/download/collection/{name}"))
        }
        ["", download, collection, name, target]
            if download.eq_ignore_ascii_case("download")
                && collection.eq_ignore_ascii_case("collection")
                && !name.is_empty()
                && !target.is_empty() =>
        {
            Some(format!("/download/collection/{name}/{target}"))
        }
        ["", download, name, target]
            if download.eq_ignore_ascii_case("download")
                && !name.is_empty()
                && !target.is_empty()
                && !name.eq_ignore_ascii_case("collection") =>
        {
            Some(format!("/download/{name}/{target}"))
        }
        _ => None,
    };
    Ok(path)
}

async fn route(
    request: Request<Incoming>,
    config: &Config,
    store: &Arc<Mutex<state::Store>>,
) -> api::Reply {
    let method = if request.method() == Method::HEAD {
        Method::GET
    } else {
        request.method().clone()
    };
    let raw_path = request.uri().path().to_owned();
    let headers = request.headers().clone();
    let raw_query = request.uri().query().unwrap_or("").to_owned();
    let request_method = request.method().as_str().to_owned();
    let peer = request
        .extensions()
        .get::<std::net::SocketAddr>()
        .map(|peer| peer.ip().to_string());
    let body = match body::read(request, config.body_limit).await {
        Ok(body) => body,
        Err(error) => return api::uncaught_error(error.status, &error.detail),
    };
    let not_found = || {
        if method == Method::GET {
            api::not_found()
        } else if method == Method::OPTIONS {
            api::options("")
        } else {
            api::route_not_found(&method, &raw_path)
        }
    };
    let path = if let Some(prefix) = &config.prefix {
        let matches = if prefix == "/" {
            backend_route(&raw_path)
                && !(config.merge && config.frontend.is_some() && raw_path.starts_with("/share/"))
        } else {
            raw_path.starts_with(prefix)
        };
        if matches {
            let stripped = if prefix == "/" {
                raw_path.as_str()
            } else {
                raw_path.strip_prefix(prefix).unwrap_or("")
            };
            if stripped.is_empty() { "/" } else { stripped }
        } else {
            if let Some(root) = &config.frontend
                && config.merge
                && !backend_route(&raw_path)
            {
                if method == Method::GET {
                    return match percent_decode_str(&raw_path).decode_utf8() {
                        Ok(path) => static_file(root, &path, &headers, true),
                        Err(_) => {
                            api::text(400, "text/plain; charset=utf-8", "Invalid URL encoding")
                        }
                    };
                }
                return if raw_path == "/" {
                    api::greeting()
                } else {
                    not_found()
                };
            }
            return api::not_found();
        }
    } else {
        raw_path.as_str()
    };
    let path = match api_path(path) {
        Ok(Some(path)) => path,
        Ok(None) => return not_found(),
        Err(()) => {
            let parameter = path.rsplit('/').next().unwrap_or("");
            return api::uncaught_error(
                400,
                &format!("URIError: Failed to decode param '{parameter}'"),
            );
        }
    };
    if path == "/" && method != Method::GET {
        return api::greeting();
    }
    if method == Method::OPTIONS {
        return api::options(&path);
    }
    let query = body::query(&raw_query);
    if (path == "/" || path == "/api/utils/env") && method == Method::GET {
        return api::send(
            200,
            "application/json; charset=utf-8",
            json::stringify(&json!({
                "status":"success", "data":{
                    "backend":"Node", "version":env!("CARGO_PKG_VERSION"),
                    "feature":{"archive":false},
                    "meta":{"node":{"env":config.environment},"rust":{"version":env!("CARGO_PKG_VERSION"),"milestone":"M2","complete":false}}
                }
            })),
        );
    }
    let mut base = api::text(200, "text/plain; charset=utf-8", "");
    config.headers(&mut base, header(&headers, "origin"));
    let request = json!({
        "method":request_method,"url":if raw_query.is_empty(){raw_path.clone()}else{format!("{raw_path}?{raw_query}")},
        "path":raw_path,"query":query,"body":body.fields(),"socket":{"remoteAddress":peer},
        "headers":headers.iter().map(|(key,value)|(key.as_str().to_owned(),json!(value.to_str().unwrap_or("")))).collect::<serde_json::Map<String,Value>>(),
        "responseHeaders":base.headers().iter().map(|(key,value)|(key.as_str().to_owned(),json!(value.to_str().unwrap_or("")))).collect::<serde_json::Map<String,Value>>()
    });
    if let Some(response) =
        api::conversion_dispatch(&method, &path, &query, &body, &headers, store, &request).await
    {
        return response;
    }
    let mut store = store.lock().unwrap();
    let response = api::dispatch(&method, &path, &query, body, &mut store);
    if response.status().as_u16() == 404 && response.body().size_hint().exact() == Some(0) {
        not_found()
    } else {
        response
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = Arc::new(Config::load()?);
    let store = Arc::new(Mutex::new(state::Store::open(Path::new(&configured(
        "SUB_STORE_DATA_BASE_PATH",
        ".",
    )))?));
    let listener = TcpListener::bind((
        configured("SUB_STORE_BACKEND_API_HOST", "::"),
        configured("SUB_STORE_BACKEND_API_PORT", "3000").parse::<u16>()?,
    ))
    .await?;
    println!(
        "Sub-Store Rust {} listening on {}",
        env!("CARGO_PKG_VERSION"),
        listener.local_addr()?
    );
    if config.frontend.is_some() && !config.merge {
        let frontend = TcpListener::bind((
            configured(
                "SUB_STORE_FRONTEND_HOST",
                &configured("SUB_STORE_BACKEND_API_HOST", "::"),
            ),
            configured("SUB_STORE_FRONTEND_PORT", "3001").parse::<u16>()?,
        ))
        .await?;
        println!("Sub-Store frontend listening on {}", frontend.local_addr()?);
        tokio::try_join!(
            serve(listener, Arc::clone(&config), Arc::clone(&store), false),
            serve(frontend, config, store, true),
        )?;
    } else {
        serve(listener, config, store, false).await?;
    }
    Ok(())
}

async fn serve(
    listener: TcpListener,
    config: Arc<Config>,
    store: Arc<Mutex<state::Store>>,
    frontend: bool,
) -> io::Result<()> {
    loop {
        let (stream, peer) = listener.accept().await?;
        let config = Arc::clone(&config);
        let store = Arc::clone(&store);
        tokio::spawn(async move {
            let service = service_fn(move |mut request: Request<Incoming>| {
                request.extensions_mut().insert(peer);
                let config = Arc::clone(&config);
                let store = Arc::clone(&store);
                async move {
                    if frontend {
                        frontend_handle(request, config, store).await
                    } else {
                        handle(request, config, store).await
                    }
                }
            });
            if let Err(error) = http1::Builder::new()
                .serve_connection(TokioIo::new(stream), service)
                .await
            {
                eprintln!("HTTP connection error: {error}");
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::json_limit;

    #[test]
    fn json_limits_use_legacy_binary_units_and_integer_prefixes() {
        for (text, expected) in [
            ("0", 0),
            ("+1KB", 1024),
            ("1.5 kb", 1536),
            ("1mb", 1048576),
            ("1GB", 1073741824),
            ("1tb", 1099511627776),
            ("1pb", 1125899906842624),
            ("64junk", 64),
            ("1e3", 1),
            ("1kb ", 1),
            ("\u{feff}32b", 32),
        ] {
            assert_eq!(json_limit(text).unwrap(), expected, "{text}");
        }
        for text in ["-1kb", "unbounded", "NaN", "Infinity"] {
            assert!(json_limit(text).is_err(), "{text}");
        }
    }
}
