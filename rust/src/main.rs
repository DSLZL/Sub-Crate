mod api;
mod state;

use http_body_util::{BodyExt, Limited};
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Method, Request, body::Incoming};
use hyper_util::rt::TokioIo;
use percent_encoding::percent_decode_str;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::convert::Infallible;
use std::env;
use std::io;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex};
use tokio::net::TcpListener;

struct Config {
    prefix: Option<String>,
    frontend: Option<PathBuf>,
    cors: Vec<String>,
    local_origins: bool,
    body_limit: usize,
    environment: Value,
    powered_by: String,
}

fn configured(name: &str, default: &str) -> String {
    env::var(name).ok().filter(|s| !s.is_empty()).unwrap_or_else(|| default.to_owned())
}

fn origin(value: &str) -> String {
    let value = value.trim().trim_matches(['\'', '"']);
    reqwest::Url::parse(value).map(|url| url.origin().ascii_serialization()).unwrap_or_else(|_| value.to_owned())
}

impl Config {
    fn load() -> io::Result<Self> {
        let merge = !configured("SUB_STORE_BACKEND_MERGE", "").is_empty();
        let prefixed = !configured("SUB_STORE_BACKEND_PREFIX", "").is_empty();
        let backend_path = configured("SUB_STORE_FRONTEND_BACKEND_PATH", "");
        if (merge || prefixed) && !backend_path.starts_with('/') {
            return Err(io::Error::other("SUB_STORE_FRONTEND_BACKEND_PATH must start with / when prefix or merge is enabled"));
        }
        let frontend = if merge {
            env::var("SUB_STORE_FRONTEND_PATH").ok().filter(|s| !s.is_empty()).map(fs_root).transpose()?
        } else { None };
        let allowed = configured("SUB_STORE_CORS_ALLOWED_ORIGINS", "");
        let mut cors: Vec<_> = allowed.split(|c: char| c == ',' || c == '，' || c.is_whitespace())
            .filter(|s| !s.is_empty()).map(origin).collect();
        let default_cors = cors.is_empty();
        if default_cors {
            cors = ["https://sub-store.vercel.app", "http://substore.stash", "https://substore.stash"].map(str::to_owned).to_vec();
        }
        let raw_limit = configured("SUB_STORE_BODY_JSON_LIMIT", "1mb").to_lowercase();
        let (number, multiplier) = if let Some(number) = raw_limit.strip_suffix("mb") { (number, 1024.0 * 1024.0) }
            else if let Some(number) = raw_limit.strip_suffix("kb") { (number, 1024.0) }
            else if let Some(number) = raw_limit.strip_suffix('b') { (number, 1.0) }
            else { (raw_limit.as_str(), 1.0) };
        let bytes = number.trim().parse::<f64>().unwrap_or(f64::NAN) * multiplier;
        if !bytes.is_finite() || bytes < 1.0 || bytes > usize::MAX as f64 {
            return Err(io::Error::other("Invalid SUB_STORE_BODY_JSON_LIMIT"));
        }
        let environment = Value::Object(env::vars().filter(|(key, _)| key.starts_with("SUB_STORE_")).map(|(key, value)| (key, json!(value))).collect());
        Ok(Self {
            prefix: (merge || prefixed).then_some(backend_path),
            frontend, cors,
            local_origins: default_cors && !configured("SUB_STORE_BACKEND_CUSTOM_NAME", "").is_empty(),
            body_limit: bytes as usize, environment,
            powered_by: configured("SUB_STORE_X_POWERED_BY", "Sub-Store"),
        })
    }

    fn allowed(&self, raw: &str) -> bool {
        if raw.is_empty() || self.cors.iter().any(|s| s == "*") { return true; }
        let normalized = origin(raw);
        if self.cors.contains(&normalized) { return true; }
        self.local_origins && reqwest::Url::parse(&normalized).is_ok_and(|url| {
            url.scheme() == "http" && matches!(url.host_str(), Some("localhost" | "127.0.0.1"))
        })
    }

    fn headers(&self, response: &mut api::Reply, raw_origin: &str) {
        let headers = response.headers_mut();
        headers.insert("access-control-allow-methods", "POST,GET,OPTIONS,PATCH,PUT,DELETE".parse().unwrap());
        headers.insert("access-control-allow-headers", "Origin, X-Requested-With, Content-Type, Accept".parse().unwrap());
        if let Ok(value) = self.powered_by.parse() { headers.insert("x-powered-by", value); }
        if self.allowed(raw_origin) {
            if self.cors.iter().any(|s| s == "*") {
                headers.insert("access-control-allow-origin", "*".parse().unwrap());
            } else if !raw_origin.is_empty() {
                if let Ok(value) = origin(raw_origin).parse() { headers.insert("access-control-allow-origin", value); }
                headers.append("vary", "Origin".parse().unwrap());
            }
        }
    }
}

fn fs_root(path: String) -> io::Result<PathBuf> { std::fs::canonicalize(path) }

fn backend_route(path: &str) -> bool {
    ["/api", "/download", "/share"].iter().any(|prefix| path == *prefix || path.starts_with(&format!("{prefix}/")))
}

fn static_file(root: &Path, path: &str, gzip: bool) -> api::Reply {
    let relative = Path::new(path.trim_start_matches('/'));
    if path.contains(['\\', '\0']) || relative.components().any(|part| !matches!(part, Component::Normal(_))) {
        return api::text(403, "text/plain; charset=utf-8", "");
    }
    let requested = root.join(relative);
    let logical = if requested.is_file() { requested } else { root.join("index.html") };
    let compressed = logical.with_file_name(format!("{}.gz", logical.file_name().unwrap().to_string_lossy()));
    let (file, encoding) = if gzip && compressed.is_file() { (compressed, true) } else { (logical.clone(), false) };
    let Ok(file) = file.canonicalize() else { return api::text(404, "text/plain; charset=utf-8", "") };
    if !file.starts_with(root) { return api::text(403, "text/plain; charset=utf-8", ""); }
    let Ok(bytes) = std::fs::read(file) else { return api::text(404, "text/plain; charset=utf-8", "") };
    let content_type = match logical.extension().and_then(|s| s.to_str()).unwrap_or("") {
        "html" => "text/html; charset=utf-8", "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8", "json" | "map" => "application/json; charset=utf-8",
        "svg" => "image/svg+xml", "png" => "image/png", "jpg" | "jpeg" => "image/jpeg",
        "ico" => "image/x-icon", "woff" => "font/woff", "woff2" => "font/woff2",
        _ => "application/octet-stream",
    };
    let name = logical.file_name().unwrap().to_string_lossy();
    let hashed = name.rsplit_once('-').and_then(|(_, tail)| tail.split_once('.')).is_some_and(|(hash, _)| hash.len() == 8 && hash.bytes().all(|b| b.is_ascii_hexdigit()));
    let mut response = api::text(200, content_type, bytes);
    response.headers_mut().insert("vary", "Accept-Encoding".parse().unwrap());
    response.headers_mut().insert("cache-control", if hashed { "public, max-age=31536000, immutable" } else { "public, max-age=0" }.parse().unwrap());
    if encoding { response.headers_mut().insert("content-encoding", "gzip".parse().unwrap()); }
    response
}

async fn handle(request: Request<Incoming>, config: Arc<Config>, store: Arc<Mutex<state::Store>>) -> Result<api::Reply, Infallible> {
    let raw_origin = request.headers().get("origin").and_then(|value| value.to_str().ok()).unwrap_or("").to_owned();
    let head = request.method() == Method::HEAD;
    let response = if !config.allowed(&raw_origin) {
        api::text(403, "text/plain;charset=UTF-8", "CORS origin not allowed")
    } else if request.method() == Method::OPTIONS {
        api::text(200, "text/plain;charset=UTF-8", "")
    } else {
        route(request, &config, &store).await
    };
    let mut response = response;
    config.headers(&mut response, &raw_origin);
    if head { *response.body_mut() = http_body_util::Full::new(hyper::body::Bytes::new()); }
    Ok(response)
}

async fn route(request: Request<Incoming>, config: &Config, store: &Mutex<state::Store>) -> api::Reply {
    let method = if request.method() == Method::HEAD { Method::GET } else { request.method().clone() };
    let raw_path = request.uri().path().to_owned();
    let decoded = match percent_decode_str(&raw_path).decode_utf8() { Ok(path) => path, Err(_) => return api::text(400, "text/plain; charset=utf-8", "Invalid URL encoding") };
    let path = if let Some(prefix) = &config.prefix {
        let matches = if prefix == "/" { backend_route(&raw_path) && !(config.frontend.is_some() && raw_path.starts_with("/share/")) } else { raw_path.starts_with(prefix) };
        if matches {
            let stripped = if prefix == "/" { decoded.as_ref() } else { decoded.strip_prefix(prefix).unwrap_or("") };
            if stripped.is_empty() { "/" } else { stripped }
        } else {
            if let Some(root) = &config.frontend
                && !backend_route(&raw_path) && method == Method::GET
            {
                let gzip = request.headers().get("accept-encoding").and_then(|value| value.to_str().ok()).is_some_and(|value| value.split(',').any(|part| part.trim() == "gzip" || part.trim() == "gzip;q=1"));
                return static_file(root, &decoded, gzip);
            }
            return api::text(404, "text/plain; charset=utf-8", "");
        }
    } else { decoded.as_ref() };
    let query = reqwest::Url::parse(&format!("http://localhost/?{}", request.uri().query().unwrap_or(""))).unwrap().query_pairs().into_owned().collect::<HashMap<_, _>>();
    if (path == "/" || path == "/api/utils/env") && method == Method::GET {
        return api::text(200, "application/json;charset=UTF-8", json!({
            "status":"success", "data":{
                "backend":"Node", "version":env!("CARGO_PKG_VERSION"),
                "feature":{"archive":false},
                "meta":{"node":{"env":config.environment},"rust":{"version":env!("CARGO_PKG_VERSION"),"milestone":"M2","complete":false}}
            }
        }).to_string());
    }
    let bytes = match Limited::new(request.into_body(), config.body_limit).collect().await {
        Ok(body) => body.to_bytes(),
        Err(_) => return api::failure(413, "INVALID_BODY", "RequestInvalidError", "Request body is invalid or exceeds the configured limit"),
    };
    let body = if bytes.is_empty() { json!({}) } else {
        match serde_json::from_slice(&bytes) {
            Ok(body) => body,
            Err(_) => return api::failure(400, "INVALID_JSON", "RequestInvalidError", "Invalid JSON body"),
        }
    };
    let mut store = store.lock().unwrap();
    api::dispatch(&method, path, &query, body, &mut store)
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = Arc::new(Config::load()?);
    let store = Arc::new(Mutex::new(state::Store::open(Path::new(&configured("SUB_STORE_DATA_BASE_PATH", ".")))?));
    let listener = TcpListener::bind((configured("SUB_STORE_BACKEND_API_HOST", "::"), configured("SUB_STORE_BACKEND_API_PORT", "3000").parse::<u16>()?)).await?;
    println!("Sub-Store Rust {} listening on {}", env!("CARGO_PKG_VERSION"), listener.local_addr()?);
    loop {
        let (stream, _) = listener.accept().await?;
        let config = Arc::clone(&config);
        let store = Arc::clone(&store);
        tokio::spawn(async move {
            let service = service_fn(move |request| handle(request, Arc::clone(&config), Arc::clone(&store)));
            if let Err(error) = http1::Builder::new().serve_connection(TokioIo::new(stream), service).await {
                eprintln!("HTTP connection error: {error}");
            }
        });
    }
}
