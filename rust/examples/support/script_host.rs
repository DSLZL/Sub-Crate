//! Contained shared-contract host. No Node process or module loading.
use http_body_util::{BodyExt, Full};
use hyper::{
    Request, Response,
    body::{Bytes, Incoming},
};
use hyper_util::{client::legacy::Client, rt::TokioExecutor};
use rquickjs::{
    AsyncContext, AsyncRuntime, CatchResultExt, Ctx, Exception, Function, Promise,
    prelude::{Async, Func},
};
use serde_json::{Value, json};
use std::io::Write;
use std::{collections::BTreeMap, error::Error, path::Path, time::Duration};

#[path = "dns.rs"]
mod dns;
#[path = "mmdb.rs"]
mod mmdb;

async fn open_response(
    method: &str,
    url: &str,
    mut headers: BTreeMap<String, String>,
    mut body: Vec<u8>,
) -> Result<Response<Incoming>, Box<dyn Error>> {
    // Hyper permits genuinely absent Accept; reqwest always inserts its default.
    let connector = hyper_rustls::HttpsConnectorBuilder::new()
        .try_with_platform_verifier()?
        .https_or_http()
        .enable_http1()
        .build();
    let client = Client::builder(TokioExecutor::new())
        .pool_max_idle_per_host(0)
        .build(connector);
    let mut method = method.to_owned();
    let mut url = reqwest::Url::parse(url)?;
    let mut history = Vec::new();
    for followed in 0..=3 {
        url.set_fragment(None);
        let mut request = Request::builder().method(method.as_str()).uri(url.as_str());
        for (key, value) in &headers {
            request = request.header(key, value);
        }
        let response = client
            .request(request.body(Full::new(Bytes::copy_from_slice(&body)))?)
            .await?;
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

async fn request<'js>(
    ctx: Ctx<'js>,
    method: String,
    url: String,
    headers: String,
    body: Option<String>,
) -> rquickjs::Result<String> {
    async fn send(
        method: &str,
        url: &str,
        headers: &str,
        body: Option<&str>,
    ) -> Result<String, Box<dyn Error>> {
        let response = open_response(
            method,
            url,
            serde_json::from_str(headers)?,
            body.map(serde_json::from_str::<Vec<u8>>)
                .transpose()?
                .unwrap_or_default(),
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
        Ok(json!({"statusCode": status, "headers": headers, "bytes": bytes.as_ref()}).to_string())
    }
    tokio::time::timeout(
        Duration::from_secs(8),
        send(&method, &url, &headers, body.as_deref()),
    )
    .await
    .map_err(|error| Exception::throw_message(&ctx, &error.to_string()))?
    .map_err(|error| Exception::throw_message(&ctx, &error.to_string()))
}

async fn download_file<'js>(ctx: Ctx<'js>, url: String, file: String) -> rquickjs::Result<String> {
    async fn save(url: &str, file: &str) -> Result<(), Box<dyn Error>> {
        let response = open_response("GET", url, BTreeMap::new(), Vec::new()).await?;
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
    tokio::time::timeout(Duration::from_secs(8), save(&url, &file))
        .await
        .map_err(|error| Exception::throw_message(&ctx, &error.to_string()))?
        .map_err(|error| Exception::throw_message(&ctx, &error.to_string()))?;
    Ok(file)
}

fn read_file(ctx: Ctx<'_>, path: String) -> rquickjs::Result<String> {
    std::fs::read(path)
        .map(|bytes| json!(bytes).to_string())
        .map_err(|error| Exception::throw_message(&ctx, &error.to_string()))
}

#[derive(Clone, Copy)]
pub enum Suite {
    Host,
    Processors,
    Age,
    Download,
}

fn random_values(ctx: Ctx<'_>, length: usize) -> rquickjs::Result<String> {
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

async fn reenter<'js>(payload: String, callback: Function<'js>) -> rquickjs::Result<String> {
    // Use the callback's active context while its outer JS promise is pending.
    crate::wait(1).await;
    let promise: Promise = callback.call((payload,))?;
    promise.into_future::<String>().await
}

pub async fn run(
    upstream: &str,
    state: &Path,
    suite: Suite,
    resume: bool,
) -> Result<Value, Box<dyn Error>> {
    let runtime = AsyncRuntime::new()?;
    runtime.set_memory_limit(64 * 1024 * 1024).await;
    let deadline = std::time::Instant::now() + Duration::from_secs(15);
    runtime
        .set_interrupt_handler(Some(Box::new(move || {
            std::time::Instant::now() >= deadline
        })))
        .await;
    let context = AsyncContext::full(&runtime).await?;
    let helpers = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../tests/compat/.generated/script-helpers.js"),
    )?;
    let cache_path = state.join(match suite {
        Suite::Host | Suite::Age => "sub-store-probe.json",
        Suite::Processors | Suite::Download => "sub-store.json",
    });
    let root_path = state.join("root.json");
    let age_output = state.join("age-output.json");
    let age_input = if matches!(suite, Suite::Age) {
        std::fs::read_to_string(state.join("age-input.json"))?
    } else {
        "{}".to_owned()
    };
    let mmdb_root = state.to_string_lossy().to_string();
    let transient_mmdb = state.join("mmdb transient🙂.mmdb");
    let mmdb_vectors = match suite {
        Suite::Processors => std::fs::read_to_string(state.join("mmdb-vectors.json"))?,
        Suite::Host | Suite::Age | Suite::Download => "{}".to_owned(),
    };
    let cache = std::fs::read_to_string(&cache_path)?;
    let root = std::fs::read_to_string(&root_path)?;
    let state = json!({"cache": serde_json::from_str::<Value>(&cache)?, "root": serde_json::from_str::<Value>(&root)?}).to_string();
    tokio::time::timeout(
        Duration::from_secs(15),
        context.async_with(async |ctx| {
            let globals = ctx.globals();
            globals.set("probeUpstream", upstream)?;
            globals.set("ageInput", ctx.json_parse(age_input)?)?;
            globals.set("__hostRandomValues", Func::from(random_values))?;
            globals.set("probeAgeProgress", Func::from(|id: String, event: String| {
                eprintln!("COMPAT_AGE {}", json!({"id": id, "event": event}));
            }))?;
            globals.set("probeCaseProgress", Func::from(|id: String, event: String| {
                eprintln!("COMPAT_PROCESSOR {}", json!({"id": id, "event": event}));
            }))?;
            globals.set("probeDnsServers", ctx.json_parse(std::env::var("SUB_STORE_COMPAT_DNS_SERVERS").unwrap_or_else(|_| "{}".to_owned()))?)?;
            globals.set("probeMmdbRoot", mmdb_root)?;
            globals.set("probeMmdbVectors", ctx.json_parse(mmdb_vectors)?)?;
            globals.set("__hostMmdbDefaults", ctx.json_parse(json!({
                "country": std::env::var("SUB_STORE_MMDB_COUNTRY_PATH").ok(),
                "asn": std::env::var("SUB_STORE_MMDB_ASN_PATH").ok()
            }).to_string())?)?;
            globals.set("__loadHostState", Func::from(move || state.clone()))?;
            globals.set(
                "__persistHostState",
                Func::from(move |ctx: Ctx<'_>, cache: String, root: String| {
                    let persist = || -> Result<(), Box<dyn Error>> {
                        for value in [&cache, &root] {
                            if !serde_json::from_str::<Value>(value)?.is_object() {
                                return Err("Host state must be an object".into());
                            }
                        }
                        // Isolated probe files only; production atomic replacement belongs to M2.
                        std::fs::write(&cache_path, cache)?;
                        std::fs::write(&root_path, root)?;
                        Ok(())
                    };
                    persist().map_err(|error| Exception::throw_message(&ctx, &error.to_string()))
                }),
            )?;
            globals.set("__hostWait", Func::from(Async(crate::wait)))?;
            globals.set("__hostRequest", Func::from(Async(request)))?;
            globals.set("__hostDownloadFile", Func::from(Async(download_file)))?;
            globals.set("__hostReadFile", Func::from(read_file))?;
            globals.set("__hostPathIsAbsolute", Func::from(|path: String| Path::new(&path).is_absolute()))?;
            globals.set("probeDownloadProgress", Func::from(|id: String, event: String| {
                eprintln!("COMPAT_DOWNLOAD {}", json!({"id": id, "event": event}));
            }))?;
            globals.set("__hostDnsUrl", Func::from(dns::parse_url))?;
            globals.set("__hostDnsQuery", Func::from(Async(dns::query)))?;
            globals.set("__hostMmdbOpen", Func::from(mmdb::open))?;
            globals.set("probeMmdbRemoveTransient", Func::from(move |ctx: Ctx<'_>| {
                std::fs::remove_file(&transient_mmdb).map_err(|error| Exception::throw_message(&ctx, &error.to_string()))
            }))?;
            globals.set("__hostReenter", Func::from(Async(reenter)))?;
            ctx.eval::<(), _>(helpers)
                .catch(&ctx)
                .map_err(|error| format!("host helpers: {error:?}"))?;
            ctx.eval::<(), _>(include_str!(
                "../../../tests/compat/fixtures/script-host.js"
            ))
            .catch(&ctx)
            .map_err(|error| format!("host setup: {error:?}"))?;
            let script = match suite {
                Suite::Host if resume => {
                    "Promise.resolve({storage: {cache: $substore.cache, root: $substore.root}})"
                }
                Suite::Host => {
                    include_str!("../../../tests/compat/fixtures/script-host-contracts.js")
                }
                Suite::Processors | Suite::Age | Suite::Download => {
                    ctx.eval::<(), _>(include_str!(
                        "../../../tests/compat/fixtures/script-processor-clock.js"
                    ))?;
                    ctx.eval::<(), _>(include_str!(
                        "../../../tests/compat/fixtures/script-helper-contracts.js"
                    ))?;
                    ctx.eval::<(), _>(include_str!(
                        "../../../tests/compat/fixtures/script-dns-contracts.js"
                    ))?;
                    ctx.eval::<(), _>(include_str!(
                        "../../../tests/compat/fixtures/script-native-dns-contracts.js"
                    ))?;
                    ctx.eval::<(), _>(include_str!(
                        "../../../tests/compat/fixtures/script-mmdb-contracts.js"
                    ))?;
                    ctx.eval::<(), _>(include_str!(
                        "../../../tests/compat/fixtures/script-gist-contracts.js"
                    ))?;
                    ctx.eval::<(), _>(
                        "const processorDiagnostics = [];\n\
                         for (const level of ['log', 'info', 'warn', 'error']) $substore[level] = message => processorDiagnostics.push(String(message));\n\
                         const processorHost = createProbeProcessors($substore, {backend: 'Rust', version: '2.42.4', meta: {}, feature: {}});\n\
                         const probeKind = 'rust'; const probeReenter = __hostReenter;\n\
                         const probeMmdbWithDefaults = (values, callback) => { const saved = __hostMmdbDefaults; __hostMmdbDefaults = values;\n\
                           try { return callback(); } finally { __hostMmdbDefaults = saved; } };",
                    )?;
                    if matches!(suite, Suite::Download) {
                        ctx.eval::<(), _>("const probeDownloadInput = JSON.parse(Buffer.from(JSON.parse(__hostReadFile(probeMmdbRoot + '/download-input.json'))).toString('utf8'));\n\
                            const probeReadFile = path => Buffer.from(JSON.parse(__hostReadFile(path)));")?;
                        if resume {
                            include_str!("../../../tests/compat/fixtures/script-download-resume.js")
                        } else {
                            include_str!("../../../tests/compat/fixtures/script-download-contracts.js")
                        }
                    } else if matches!(suite, Suite::Age) {
                        include_str!("../../../tests/compat/fixtures/script-age-contracts.js")
                    } else if resume {
                        "Promise.resolve({storage: {cache: $substore.cache, root: $substore.root}, resourceCache: processorHost.scriptResourceCache.resourceCache, headersCache: processorHost.headersResourceCache.resourceCache, dnsCache: processorHost.resourceCache.resourceCache})"
                    } else {
                        include_str!("../../../tests/compat/fixtures/script-processor-contracts.js")
                    }
                }
            };
            let promise: Promise = ctx
                .eval(format!("Promise.resolve({script}).then(JSON.stringify)"))
                .catch(&ctx)
                .map_err(|error| format!("host assertions: {error:?}"))?;
            let result = promise
                .into_future::<String>()
                .await
                .catch(&ctx)
                .map_err(|error| format!("host execution: {error:?}"))?;
            let mut result: Value = serde_json::from_str(&result)?;
            if matches!(suite, Suite::Age) {
                let exchange = result.as_object_mut().ok_or("Age result must be an object")?
                    .remove("exchange").ok_or("Age result must include exchange")?;
                std::fs::write(&age_output, serde_json::to_vec(&exchange)?)?;
            }
            Ok::<_, Box<dyn Error>>(result)
        }),
    )
    .await?
}
