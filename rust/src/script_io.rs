//! Native script I/O shared by the service and retained compatibility probe.
//! Operation timeouts belong to the caller, not the old proof's eight-second guard.
use http_body_util::{BodyExt, Full};
use hyper::{
    Request, Response,
    body::{Bytes, Incoming},
};
use hyper_util::{client::legacy::Client, rt::TokioExecutor};
use rquickjs::{Ctx, Exception};
use serde_json::json;
use std::{collections::BTreeMap, error::Error, io::Write};

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

pub(crate) async fn request<'js>(
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
    send(&method, &url, &headers, body.as_deref())
        .await
        .map_err(|error| Exception::throw_message(&ctx, &error.to_string()))
}

pub(crate) async fn download_file<'js>(
    ctx: Ctx<'js>,
    url: String,
    file: String,
) -> rquickjs::Result<String> {
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
