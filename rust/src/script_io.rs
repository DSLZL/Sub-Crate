//! Native script I/O shared by the service and retained compatibility probe.
//! Operation timeouts belong to the caller, not the old proof's eight-second guard.
use http_body_util::{BodyExt, Full};
use hyper::{
    Request, Response,
    body::{Bytes, Incoming},
};
use hyper_util::{client::legacy::Client, rt::TokioExecutor};
use rquickjs::{Ctx, Exception, JsLifetime, Promise, prelude::Opt};
use serde_json::{Value, json};
use std::{
    cell::RefCell,
    collections::{BTreeMap, HashMap},
    error::Error,
    io::{Read, Write},
    rc::Rc,
    time::Duration,
};

#[path = "script_transport.rs"]
mod transport;

type Failure = Box<dyn Error + Send + Sync>;

#[derive(Clone)]
struct Executor(tokio::runtime::Handle);

// The executor owns no JavaScript handles or context borrows.
unsafe impl<'js> JsLifetime<'js> for Executor {
    type Changed<'to> = Executor;
}

pub(crate) fn set_executor(
    ctx: &Ctx<'_>,
    executor: tokio::runtime::Handle,
) -> rquickjs::Result<()> {
    ctx.store_userdata(Executor(executor))
        .map_err(|_| Exception::throw_message(ctx, "Script I/O executor already borrowed"))?;
    Ok(())
}

#[derive(Clone, Default)]
struct Timers(Rc<RefCell<HashMap<u64, tokio::sync::oneshot::Sender<()>>>>);

// Native cancellation channels contain no JS values or engine borrows.
unsafe impl<'js> JsLifetime<'js> for Timers {
    type Changed<'to> = Timers;
}

fn timers(ctx: &Ctx<'_>) -> rquickjs::Result<Timers> {
    if let Some(timers) = ctx.userdata::<Timers>() {
        return Ok(timers.clone());
    }
    let timers = Timers::default();
    ctx.store_userdata(timers.clone())
        .map_err(|_| Exception::throw_message(ctx, "Timer state already borrowed"))?;
    Ok(timers)
}

pub(crate) fn timer(ctx: Ctx<'_>, id: u64, milliseconds: u64) -> rquickjs::Result<Promise<'_>> {
    let timers = timers(&ctx)?;
    let (cancel, cancelled) = tokio::sync::oneshot::channel();
    timers.0.borrow_mut().insert(id, cancel);
    Promise::wrap_future(&ctx, async move {
        tokio::select! {
            _ = tokio::time::sleep(Duration::from_millis(milliseconds)) => {},
            _ = cancelled => {},
        }
        timers.0.borrow_mut().remove(&id);
    })
}

pub(crate) fn cancel_timer(ctx: Ctx<'_>, id: u64) -> rquickjs::Result<()> {
    timers(&ctx)?.0.borrow_mut().remove(&id);
    Ok(())
}

fn decoded(
    status: u16,
    headers: &mut BTreeMap<String, Value>,
    mut bytes: Vec<u8>,
) -> Result<Vec<u8>, Failure> {
    if status < 200 || [204, 304].contains(&status) {
        return Ok(bytes);
    }
    let encodings = headers
        .get("content-encoding")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_ascii_lowercase();
    let encodings: Vec<_> = encodings.split(',').map(str::trim).collect();
    if encodings.len() > 5 {
        return Err(format!(
            "too many content-encodings in response: {}, maximum allowed is 5",
            encodings.len()
        )
        .into());
    }
    if encodings
        .iter()
        .any(|value| !["", "gzip", "x-gzip", "deflate", "compress", "x-compress"].contains(value))
    {
        return Ok(bytes);
    }
    if encodings.iter().all(|value| value.is_empty()) {
        return Ok(bytes);
    }
    headers.remove("content-encoding");
    headers.remove("content-length");
    for encoding in encodings
        .into_iter()
        .rev()
        .filter(|value| !value.is_empty())
    {
        let input = bytes;
        bytes = Vec::new();
        match encoding {
            "gzip" | "x-gzip" => {
                flate2::read::MultiGzDecoder::new(input.as_slice()).read_to_end(&mut bytes)?;
            }
            _ => {
                flate2::read::ZlibDecoder::new(input.as_slice()).read_to_end(&mut bytes)?;
            }
        }
    }
    Ok(bytes)
}

async fn open_response(
    method: &str,
    url: &str,
    mut headers: BTreeMap<String, String>,
    mut body: Vec<u8>,
    options: Option<&Value>,
) -> Result<Response<Incoming>, Failure> {
    // Hyper permits genuinely absent Accept; reqwest always inserts its default.
    let transport = transport::Transport::new(options)?;
    let connector = transport.connector();
    let maximum = transport::max_header_size();
    let client = Client::builder(TokioExecutor::new())
        .pool_max_idle_per_host(0)
        // Undici limits field/value bytes; the wire buffer also contains delimiters.
        .http1_max_buf_size(maximum.saturating_mul(4).max(8192))
        .build(connector);
    let mut method = method.to_owned();
    let mut url = reqwest::Url::parse(url)?;
    let mut history = Vec::new();
    for followed in 0..=3 {
        url.set_fragment(None);
        let mut request = Request::builder().method(method.as_str()).uri(url.as_str());
        let uri = url.as_str().parse()?;
        if let Some(proxy) = transport.proxy(&uri)? {
            if headers.contains_key("proxy-authorization") {
                return Err("Proxy-Authorization should be sent in ProxyAgent constructor".into());
            }
            if uri.scheme_str() == Some("http")
                && ["http", "https"].contains(&proxy.scheme())
                && let Some(auth) = transport::authorization(&proxy)?
            {
                request = request.header("proxy-authorization", auth);
            }
        }
        for (key, value) in &headers {
            request = request.header(key, value);
        }
        let response = client
            .request(request.body(Full::new(Bytes::copy_from_slice(&body)))?)
            .await
            .map_err(|error| -> Failure {
                if error
                    .source()
                    .and_then(|cause| cause.downcast_ref::<hyper::Error>())
                    .is_some_and(hyper::Error::is_parse_too_large)
                {
                    "Headers Overflow Error".into()
                } else {
                    Box::new(error)
                }
            })?;
        if response
            .headers()
            .iter()
            .map(|(key, value)| key.as_str().len() + value.as_bytes().len())
            .sum::<usize>()
            >= maximum
        {
            return Err("Headers Overflow Error".into());
        }
        // Pinned Undici rejects the third followed response, including a final 200.
        if followed == 3 {
            return Err("max redirects".into());
        }
        let status = response.status().as_u16();
        if [300, 301, 302, 303, 307, 308].contains(&status)
            && let Some(location) = response.headers().get(hyper::header::LOCATION)
        {
            let next = url.join(location.to_str()?)?;
            history.push(url.clone());
            if history.contains(&next) {
                return Err("redirect loop detected".into());
            }
            headers.remove("host");
            if next.origin() != url.origin() {
                for key in ["authorization", "cookie", "proxy-authorization"] {
                    headers.remove(key);
                }
            }
            if ([301, 302].contains(&status) && method == "POST")
                || (status == 303 && method != "HEAD")
            {
                method = "GET".to_owned();
                body.clear();
                headers.retain(|key, _| !key.starts_with("content-"));
            }
            response.into_body().collect().await?;
            url = next;
            continue;
        }
        return Ok(response);
    }
    unreachable!("redirect loop returns at the third follow")
}

pub(crate) async fn request<'js>(
    ctx: Ctx<'js>,
    method: String,
    url: String,
    headers: String,
    body: Option<String>,
    options: Opt<String>,
) -> rquickjs::Result<String> {
    async fn send(
        method: &str,
        url: &str,
        headers: &str,
        body: Option<&str>,
        options: Option<&str>,
    ) -> Result<String, Failure> {
        let options = options.map(serde_json::from_str::<Value>).transpose()?;
        let response = open_response(
            method,
            url,
            serde_json::from_str(headers)?,
            body.map(serde_json::from_str::<Vec<u8>>)
                .transpose()?
                .unwrap_or_default(),
            options.as_ref(),
        )
        .await?;
        let status = response.status().as_u16();
        let mut headers = BTreeMap::new();
        for key in response.headers().keys() {
            let values: Vec<_> = response
                .headers()
                .get_all(key)
                .iter()
                .map(|value| value.to_str().map(str::to_owned))
                .collect::<Result<_, _>>()?;
            headers.insert(
                key.as_str().to_owned(),
                if key == hyper::header::SET_COOKIE {
                    json!(values)
                } else {
                    json!(values.join(", "))
                },
            );
        }
        let bytes = response.into_body().collect().await?.to_bytes();
        let bytes = decoded(status, &mut headers, bytes.to_vec())?;
        let mut response = json!({"statusCode": status, "headers": headers});
        let binary = options
            .as_ref()
            .and_then(|options| options.get("responseBytes"))
            .and_then(Value::as_bool)
            .unwrap_or(false);
        if binary {
            response["bytes"] = json!(bytes);
        } else {
            let text = bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(&bytes);
            response["body"] = json!(String::from_utf8_lossy(text));
        }
        Ok(response.to_string())
    }
    let executor = ctx
        .userdata::<Executor>()
        .map(|executor| executor.0.clone())
        .ok_or_else(|| Exception::throw_message(&ctx, "Script I/O executor unavailable"))?;
    let mut requests = tokio::task::JoinSet::new();
    requests.spawn_on(
        async move {
            send(
                &method,
                &url,
                &headers,
                body.as_deref(),
                options.0.as_deref(),
            )
            .await
            .map_err(|error| error.to_string())
        },
        &executor,
    );
    requests
        .join_next()
        .await
        .expect("HTTP task was just spawned")
        .map_err(|error| Exception::throw_message(&ctx, &error.to_string()))?
        .map_err(|error| Exception::throw_message(&ctx, &error))
}

pub(crate) async fn download_file<'js>(
    ctx: Ctx<'js>,
    url: String,
    file: String,
) -> rquickjs::Result<String> {
    async fn save(url: &str, file: &str) -> Result<(), Failure> {
        let response = open_response("GET", url, BTreeMap::new(), Vec::new(), None).await?;
        if response.status() != 200 {
            return Err(format!("Failed to download file from {url}").into());
        }
        // Preserve the source's truncation/partial-file behavior after a successful status.
        let mut output = std::fs::File::create(file)?;
        let mut body = response.into_body();
        while let Some(frame) = body.frame().await {
            if let Some(bytes) = frame?.data_ref() {
                output.write_all(bytes)?;
            }
        }
        Ok(())
    }
    save(&url, &file)
        .await
        .map_err(|error| Exception::throw_message(&ctx, &error.to_string()))?;
    Ok(file)
}

pub(crate) fn read_file(ctx: Ctx<'_>, path: String) -> rquickjs::Result<String> {
    std::fs::read(path)
        .map(|bytes| json!(bytes).to_string())
        .map_err(|error| Exception::throw_message(&ctx, &error.to_string()))
}

pub(crate) fn random_values(ctx: Ctx<'_>, length: usize) -> rquickjs::Result<String> {
    if length > 65536 {
        return Err(Exception::throw_message(
            &ctx,
            "Random values exceed 65536 bytes",
        ));
    }
    let mut bytes = vec![0; length];
    getrandom::fill(&mut bytes)
        .map_err(|error| Exception::throw_message(&ctx, &error.to_string()))?;
    Ok(serde_json::to_string(&bytes).expect("Bytes serialize as JSON"))
}
