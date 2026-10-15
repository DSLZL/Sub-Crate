//! M1 feasibility experiment, not the production script host.
#[path = "support/script_host.rs"]
mod script_host;
use rquickjs::{
    AsyncContext, AsyncRuntime, CatchResultExt, Ctx, Exception, Function, Promise,
    prelude::{Async, Func},
};
use std::{
    cell::RefCell,
    collections::BTreeMap,
    error::Error,
    rc::Rc,
    time::{Duration, Instant},
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

async fn wait(milliseconds: u64) {
    tokio::time::sleep(Duration::from_millis(milliseconds)).await;
}

async fn http_get<'js>(ctx: Ctx<'js>, url: String, headers: String) -> rquickjs::Result<String> {
    let headers: BTreeMap<String, String> = serde_json::from_str(&headers)
        .map_err(|error| Exception::throw_message(&ctx, &error.to_string()))?;
    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(5))
        .build()
        .map_err(|error| Exception::throw_message(&ctx, &error.to_string()))?;
    let mut request = client.get(url);
    for (key, value) in headers {
        request = request.header(key, value);
    }
    let response = request
        .send()
        .await
        .map_err(|error| Exception::throw_message(&ctx, &error.to_string()))?;
    let status = response.status().as_u16();
    let headers: BTreeMap<_, _> = response
        .headers()
        .iter()
        .map(|(key, value)| {
            (
                key.as_str().to_owned(),
                value.to_str().unwrap_or("").to_owned(),
            )
        })
        .collect();
    let body = response
        .text()
        .await
        .map_err(|error| Exception::throw_message(&ctx, &error.to_string()))?;
    Ok(serde_json::json!({"status": status, "statusCode": status, "headers": headers, "body": body}).to_string())
}

async fn nested<'js>(ctx: Ctx<'js>, callback: Function<'js>) -> rquickjs::Result<String> {
    // Re-enter using the active Ctx; acquiring AsyncContext's lock again would deadlock.
    wait(1).await;
    let promise: Promise = callback.call(("nested artifact",))?;
    let result = promise.into_future::<String>().await?;
    ctx.globals().set("nestedCompleted", true)?;
    Ok(result)
}

async fn verify_limits() -> Result<serde_json::Value, Box<dyn Error>> {
    let runtime = AsyncRuntime::new()?;
    runtime.set_memory_limit(64 * 1024 * 1024).await;
    runtime.set_max_stack_size(128 * 1024).await;
    let context = AsyncContext::full(&runtime).await?;
    let mut cpu = Vec::new();
    for mode in ["eval", "promise"] {
        let started = Instant::now();
        let deadline = started + Duration::from_millis(100);
        runtime
            .set_interrupt_handler(Some(Box::new(move || Instant::now() >= deadline)))
            .await;
        let execution = if mode == "eval" {
            context
                .with(|ctx| ctx.eval::<(), _>("try { while (true) {} } catch (_) {}"))
                .await
        } else {
            context
                .async_with(async |ctx| {
                    let promise: Promise =
                        ctx.eval("Promise.resolve().then(() => { while (true) {} })")?;
                    promise.into_future::<()>().await
                })
                .await
        };
        let elapsed = started.elapsed();
        runtime.set_interrupt_handler(None).await;
        assert!(matches!(execution, Err(rquickjs::Error::Exception)));
        let diagnostic = context
            .with(|ctx| ctx.catch().as_exception().and_then(Exception::message))
            .await
            .ok_or("Interrupted execution produced no Error diagnostic")?;
        assert!(diagnostic.contains("interrupted"), "{diagnostic}");
        assert!(elapsed >= Duration::from_millis(100) && elapsed < Duration::from_secs(5));
        assert_eq!(context.with(|ctx| ctx.eval::<i32, _>("21 * 2")).await?, 42);
        cpu.push(serde_json::json!({"mode": mode, "elapsed_ms": elapsed.as_millis(), "diagnostic": diagnostic, "recovered": true}));
    }
    let mut resource_errors = BTreeMap::new();
    for (name, script, expected) in [
        (
            "memory",
            "new ArrayBuffer(128 * 1024 * 1024)",
            "out of memory",
        ),
        (
            "stack",
            "function overflow() { return overflow(); } overflow();",
            "Maximum call stack size exceeded",
        ),
    ] {
        let diagnostic = context
            .with(|ctx| {
                assert!(matches!(
                    ctx.eval::<(), _>(script),
                    Err(rquickjs::Error::Exception)
                ));
                ctx.catch().as_exception().and_then(Exception::message)
            })
            .await
            .ok_or("Resource limit produced no Error diagnostic")?;
        assert!(diagnostic.contains(expected), "{diagnostic}");
        assert_eq!(context.with(|ctx| ctx.eval::<i32, _>("21 * 2")).await?, 42);
        resource_errors.insert(name, diagnostic);
    }
    Ok(
        serde_json::json!({"cpu": cpu, "memory_limit_bytes": 64 * 1024 * 1024,
        "stack_limit_bytes": 128 * 1024, "resource_errors": resource_errors}),
    )
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn Error>> {
    let host_upstream = std::env::var("SUB_STORE_COMPAT_UPSTREAM")?;
    let host_state = std::env::var("SUB_STORE_COMPAT_STATE_DIR")?;
    if std::env::args().any(|arg| arg == "--download" || arg == "--download-resume") {
        println!(
            "{}",
            script_host::run(
                &host_upstream,
                std::path::Path::new(&host_state),
                script_host::Suite::Download,
                std::env::args().any(|arg| arg == "--download-resume")
            )
            .await?
        );
        return Ok(());
    }
    if std::env::args().any(|arg| arg == "--age") {
        println!(
            "{}",
            script_host::run(
                &host_upstream,
                std::path::Path::new(&host_state),
                script_host::Suite::Age,
                false
            )
            .await?
        );
        return Ok(());
    }
    if std::env::args().any(|arg| arg == "--host-resume") {
        println!(
            "{}",
            script_host::run(
                &host_upstream,
                std::path::Path::new(&host_state),
                script_host::Suite::Host,
                true
            )
            .await?
        );
        return Ok(());
    }
    if std::env::args().any(|arg| arg == "--processors" || arg == "--processors-resume") {
        println!(
            "{}",
            script_host::run(
                &host_upstream,
                std::path::Path::new(&host_state),
                script_host::Suite::Processors,
                std::env::args().any(|arg| arg == "--processors-resume")
            )
            .await?
        );
        return Ok(());
    }
    let limits = verify_limits().await?;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let upstream = format!("http://{}", listener.local_addr()?);
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await?;
        let mut bytes = Vec::new();
        let mut buffer = [0; 1024];
        while !bytes.windows(4).any(|part| part == b"\r\n\r\n") {
            let length = socket.read(&mut buffer).await?;
            if length == 0 || bytes.len() + length > 8192 {
                return Err(std::io::Error::other("Incomplete or oversized request"));
            }
            bytes.extend_from_slice(&buffer[..length]);
        }
        let request = String::from_utf8_lossy(&bytes);
        if !request.starts_with("GET /json/1.2.3.4?lang=zh-CN HTTP/1.1\r\n") {
            return Err(std::io::Error::other(format!(
                "Unexpected upstream request: {request}"
            )));
        }
        if !request.to_ascii_lowercase().contains("\r\nuser-agent: mozilla/5.0 (macintosh; intel mac os x 10.10; rv:78.0) gecko/20100101 firefox/78.0\r\n") {
            return Err(std::io::Error::other("Script User-Agent was not forwarded"));
        }
        let body = r#"{"status":"success","countryCode":"US"}"#;
        socket
            .write_all(
                format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes(),
            )
            .await?;
        Ok::<_, std::io::Error>(())
    });

    let runtime = AsyncRuntime::new()?;
    runtime.set_memory_limit(64 * 1024 * 1024).await;
    let deadline = Instant::now() + Duration::from_secs(15);
    runtime
        .set_interrupt_handler(Some(Box::new(move || Instant::now() >= deadline)))
        .await;
    let context = AsyncContext::full(&runtime).await?;
    let storage = Rc::new(RefCell::new(BTreeMap::<String, String>::new()));
    let helpers_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../tests/compat/.generated/script-helpers.js");
    let helpers = std::fs::read_to_string(&helpers_path).map_err(|error| {
        format!(
            "Build the pinned script helper bundle first (tests/compat/build-helpers.cjs): {error}"
        )
    })?;
    let result = tokio::time::timeout(
        Duration::from_secs(15),
        context.async_with(async |ctx| {
            let globals = ctx.globals();
            globals.set("__hostRandomValues", Func::from(script_host::random_values))?;
            globals.set("probeUpstream", upstream)?;
            globals.set("__wait", Func::from(Async(wait)))?;
            globals.set("__httpGet", Func::from(Async(http_get)))?;
            globals.set("__nested", Func::from(Async(nested)))?;
            let read_storage = Rc::clone(&storage);
            globals.set(
                "__read",
                Func::from(move |key: String| read_storage.borrow().get(&key).cloned()),
            )?;
            let write_storage = Rc::clone(&storage);
            globals.set(
                "__write",
                Func::from(move |value: String, key: String| {
                    write_storage.borrow_mut().insert(key, value);
                    true
                }),
            )?;
            ctx.eval::<(), _>(helpers)
                .catch(&ctx)
                .map_err(|error| format!("helpers: {error:?}"))?;
            ctx.eval::<(), _>(include_str!(
                "../../tests/compat/fixtures/script-host-probe.js"
            ))
            .catch(&ctx)
            .map_err(|error| format!("host setup: {error:?}"))?;
            // Preserve the supplied script bytes; redirect its known external API in the probe host.
            ctx.eval::<(), _>(include_str!("../../scripts/ip-flag-node.js"))
                .catch(&ctx)
                .map_err(|error| format!("ip-flag script: {error:?}"))?;
            let promise: Promise = ctx
                .eval(include_str!(
                    "../../tests/compat/fixtures/script-assertions.js"
                ))
                .catch(&ctx)
                .map_err(|error| format!("assertions: {error:?}"))?;
            let result = promise
                .into_future::<String>()
                .await
                .catch(&ctx)
                .map_err(|error| format!("script execution: {error:?}"))?;
            Ok::<_, Box<dyn Error>>(result)
        }),
    )
    .await??;
    server.await??;
    let mut result: serde_json::Value = serde_json::from_str(&result)?;
    let host = script_host::run(
        &host_upstream,
        std::path::Path::new(&host_state),
        script_host::Suite::Host,
        false,
    )
    .await?;
    result["passed"]
        .as_array_mut()
        .ok_or("Script assertion groups were not an array")?
        .extend([
            serde_json::json!("CPU interruption and recovery for eval and promise jobs"),
            serde_json::json!("memory limit and recovery"),
            serde_json::json!("stack limit and recovery"),
        ]);
    println!(
        "{}",
        serde_json::json!({
            "scope": "M1 script feasibility probe", "production_ready": false,
            "os": std::env::consts::OS, "arch": std::env::consts::ARCH,
            "engine": "rquickjs 0.14.0", "result": result, "limits": limits, "host": host
        })
    );
    Ok(())
}
