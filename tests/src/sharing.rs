use crate::{
    Result,
    api::{self, Api},
    contracts, json, reference, require,
    server::{self, Backend},
    service, static_files,
};
use serde_json::{Value, json};
use std::path::Path;

pub fn diagnostic(
    response: &Value,
    mode: &str,
    kind: &str,
    status: u16,
    detail: &str,
    exact: bool,
    head: bool,
) -> Result<String> {
    require(response["status"] == status, "Diagnostic status mismatch")?;
    let body = response["body"].as_str().ok_or("Missing diagnostic body")?;
    let headers = &response["headers"];
    require(
        headers["content-type"] == "text/html; charset=utf-8"
            && headers["content-security-policy"] == "default-src 'none'"
            && headers["x-content-type-options"] == "nosniff",
        "Diagnostic security/type headers changed",
    )?;
    let length = headers["content-length"]
        .as_str()
        .ok_or("Missing diagnostic length")?
        .parse::<usize>()?;
    if head {
        require(
            body.is_empty() && length > 0,
            "HEAD diagnostic body must be suppressed",
        )?;
        return Ok("HEAD body suppressed".into());
    }
    require(length == body.len(), "Diagnostic content length mismatch")?;
    let prefix = "<!DOCTYPE html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n<title>Error</title>\n</head>\n<body>\n<pre>";
    let suffix = "</pre>\n</body>\n</html>\n";
    let text = body
        .strip_prefix(prefix)
        .and_then(|body| body.strip_suffix(suffix))
        .ok_or("Diagnostic HTML envelope changed")?
        .replace("<br>", "\n")
        .replace("&nbsp;", "\u{a0}")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&");
    if mode == "production" {
        let expected = match status {
            400 => "Bad Request",
            403 => "Forbidden",
            412 => "Precondition Failed",
            413 => "Payload Too Large",
            415 => "Unsupported Media Type",
            416 => "Range Not Satisfiable",
            500 => "Internal Server Error",
            _ => return Err("Unexpected diagnostic status".into()),
        };
        require(
            text == expected,
            "Production diagnostic leaked or changed error",
        )?;
        return Ok(text);
    }
    let (first, frames) = text.split_once('\n').ok_or("Native trace missing")?;
    require(
        if exact {
            first == detail
        } else {
            first.starts_with(detail)
        },
        format!("Diagnostic first line changed: {first}"),
    )?;
    require(
        regex::Regex::new(if kind == "node" {
            r"(?m)^\s+at "
        } else {
            r"(?m)^\s*\d+: "
        })?
        .is_match(frames),
        "Native trace missing",
    )?;
    Ok(if exact {
        first.to_owned()
    } else {
        format!("{detail} [parser-specific wording]")
    })
}

fn encryption(api: &Api, origin: &Value) -> Result {
    let fixture = json::read(crate::compat().join("fixtures/http-cases.json"))?;
    let case = fixture
        .as_array()
        .ok_or("Invalid HTTP fixture")?
        .iter()
        .find(|case| case["id"] == "PC-native-age-public-and-download")
        .ok_or("Missing share age fixture")?;
    let pairs = case["steps"][0]["body"]["process"][0]["args"]["arguments"]["pairs"]
        .as_array()
        .ok_or("Missing age key pairs")?;
    require(pairs.len() == 2, "Both age key types are required")?;
    let source = |pair: &Value| json!({"age-public-key":pair["public"],"process":[{"type":"Script Operator","args":{"arguments":{"resourcePublic":pair["public"]},"content":"function operator(input){if(input.$file['age-public-key']!==$arguments.resourcePublic)throw new Error('share mutated source key');return input;}"}}]});
    api.send(
        &json!({"method":"PATCH","path":"/api/file/merged-share","body":source(&pairs[0])}),
        200,
    )?;
    api.send(&json!({"method":"POST","path":"/api/token","body":{"payload":{"type":"file","name":"merged-share","token":"age-once","age-public-key":pairs[1]["public"]},"options":{"mode":"count","count":1}}}),200)?;
    let public = json!({"path":"/share/file/merged-share?token=age-once","origin":origin,"headers":{"x-sub-store-share-age-public-key":pairs[0]["public"]}});
    let first = api.response(&public)?;
    api.send(
        &json!({"method":"PATCH","path":"/api/file/merged-share","body":source(&pairs[1])}),
        200,
    )?;
    let mut backend = public.clone();
    backend["origin"] = json!(false);
    let second = api.response(&backend)?;
    let third = api.response(&json!({"path":public["path"]}))?;
    for (response, pair) in [
        (&first, &pairs[1]),
        (&second, &pairs[0]),
        (&third, &pairs[1]),
    ] {
        require(
            response["status"] == 200
                && response["body"]
                    .as_str()
                    .is_some_and(|body| body.starts_with("-----BEGIN AGE ENCRYPTED FILE-----\n"))
                && response["headers"]["content-type"] == "text/plain; charset=utf-8",
            "Shared file was not encrypted",
        )?;
        let decrypted=api.data(&json!({"method":"POST","path":"/api/preview/sub","body":{"name":"share-decrypt","source":"local","content":"ss://YWVzLTEyOC1nY206cGFzcw==@127.0.0.1:8388#fixture","process":[{"type":"Script Operator","args":{"arguments":{"armor":response["body"],"secret":pair["secret"]},"content":"async function operator(proxies){proxies[0].plain=await ProxyUtils.age.decrypt($arguments.armor,$arguments.secret);return proxies;}"}}]}}))?;
        require(
            decrypted["processed"][0]["plain"] == "share-body",
            "Shared file could not be decrypted with the selected recipient",
        )?;
    }
    api.send(
        &json!({"method":"DELETE","path":"/api/token/age-once?type=file&name=merged-share"}),
        200,
    )?;
    api.send(&json!({"method":"PATCH","path":"/api/file/merged-share","body":{"age-public-key":"","process":[]}}),200)?;
    Ok(())
}

pub fn run(output: &Path) -> Result {
    reference::verify(output)?;
    let (baseline, root) = service::baseline(output)?;
    for mode in ["merged", "standalone"] {
        let backend_port = server::free_port()?;
        let mut frontend_port = server::free_port()?;
        while frontend_port == backend_port {
            frontend_port = server::free_port()?;
        }
        let mut paired = Vec::new();
        let mut states = Vec::new();
        for (kind, backend) in [
            ("node", Backend::Node(reference::node())),
            ("rust", service::target()?),
        ] {
            let label = format!("{kind}-sharing-{mode}");
            let mut document = baseline.clone();
            for field in ["files", "subs", "collections", "tokens", "archives"] {
                document[field] = json!([]);
            }
            let (state, mut environment) = service::seed(output, &label, &document, &root)?;
            static_files::prepare(&state.join("frontend"))?;
            for (key, value) in [
                ("SUB_STORE_BACKEND_API_PORT", backend_port.to_string()),
                ("SUB_STORE_FRONTEND_PORT", frontend_port.to_string()),
                ("SUB_STORE_FRONTEND_HOST", "127.0.0.1".into()),
                (
                    "SUB_STORE_FRONTEND_PATH",
                    state.join("frontend").to_string_lossy().into_owned(),
                ),
            ] {
                server::set(&mut environment, key, value);
            }
            environment.remove(std::ffi::OsStr::new("NODE_ENV"));
            if mode == "merged" {
                server::set(&mut environment, "SUB_STORE_BACKEND_MERGE", "true");
            }
            let origin = if mode == "merged" {
                json!(true)
            } else {
                json!(format!("http://127.0.0.1:{frontend_port}"))
            };
            let mut cases = contracts::cases("sharing-core")?;
            if mode == "standalone" {
                for case in &mut cases {
                    for step in case["steps"]
                        .as_array_mut()
                        .ok_or("Missing sharing steps")?
                    {
                        if step["origin"] == true {
                            if step.get("method").and_then(Value::as_str).unwrap_or("GET") != "GET"
                            {
                                step["expect"] = json!({"status":500});
                            }
                            step["origin"] = origin.clone();
                        }
                    }
                }
            }
            eprintln!("Sharing {label}");
            let api = Api::start(&backend, &environment, &state, output, &label)?;
            if mode == "standalone" {
                require(
                    api.http.request(
                        &format!("{}/backend", api.listener.frontend),
                        &json!({"path":"/api/utils/env"}),
                    )?["status"]
                        == 200,
                    "Share frontend API readiness failed",
                )?;
            }
            let lower = service::clock(false);
            let observed = contracts::check(
                &api.http,
                &api.listener.url,
                &state,
                &cases,
                None,
                &output.join(format!("{label}.json")),
            )?;
            service::captured_clocks(&observed, lower, service::clock(true))?;
            paired.push(observed);
            api.send(&json!({"method":"POST","path":"/api/files","body":{"name":"merged-share","source":"local","content":"share-body"}}),201)?;
            let signing = json!({"method":"POST","path":"/api/token","body":{"payload":{"type":"file","name":"merged-share","token":"concurrent"},"options":{"mode":"count","count":1}}});
            let signed = api::concurrent(&api.listener.url, &[signing.clone(), signing.clone()])?;
            let mut statuses = signed
                .iter()
                .map(|response| response["status"].as_u64().ok_or("Missing signing status"))
                .collect::<std::result::Result<Vec<_>, _>>()?;
            statuses.sort();
            require(
                statuses == [200, 500],
                "Duplicate token signing was not atomic",
            )?;
            let rejected = signed
                .iter()
                .find(|response| response["status"] == 500)
                .unwrap();
            require(
                json::parse(
                    rejected["body"]
                        .as_str()
                        .ok_or("Missing duplicate response")?,
                )?["error"]["code"]
                    == "DUPLICATE_TOKEN",
                "Duplicate token error changed",
            )?;
            let public =
                json!({"path":"/share/file/merged-share?token=concurrent","origin":origin});
            let consumed = api::concurrent(&api.listener.url, &[public.clone(), public])?;
            let mut statuses = consumed
                .iter()
                .map(|response| {
                    response["status"]
                        .as_u64()
                        .ok_or("Missing consumption status")
                })
                .collect::<std::result::Result<Vec<_>, _>>()?;
            statuses.sort();
            require(
                statuses == [200, 404]
                    && consumed
                        .iter()
                        .find(|response| response["status"] == 200)
                        .unwrap()["body"]
                        == "share-body",
                "Count token consumption was not atomic",
            )?;
            let tokens = api.data(&json!({"path":"/api/tokens"}))?;
            require(
                tokens.as_array().is_some_and(|tokens| tokens.len() == 1)
                    && tokens[0]["count"] == 1
                    && tokens[0]["usedCount"] == 1,
                "Count token state differs",
            )?;
            json::write_new(
                output.join(format!("{label}-concurrency.json")),
                &json!({"signing":signed,"public":consumed,"tokens":tokens}),
            )?;
            api.send(&json!({"method":"DELETE","path":"/api/token/concurrent?type=file&name=merged-share"}),200)?;
            encryption(&api, &origin)?;
            if kind == "rust" {
                let mut signing = signing.clone();
                signing["body"]["payload"]["token"] = json!("write-failure");
                api.send(&signing, 200)?;
                api::denied_write(
                    &api,
                    &json!({"path":"/share/file/merged-share?token=write-failure","origin":origin}),
                    "/api/tokens",
                    "sub-store.json",
                    "STATE_WRITE_FAILED",
                )?;
                api.send(&json!({"method":"DELETE","path":"/api/token/write-failure?type=file&name=merged-share"}),200)?;
            }
            states.push(api.durable()?);
        }
        require(
            paired[0].len() == paired[1].len(),
            "Sharing scenario count differs",
        )?;
        for (left, right) in paired[0].iter().zip(&paired[1]) {
            json::check(&left["id"], &right["id"], false)?;
            let left = left["steps"].as_array().ok_or("Missing sharing steps")?;
            let right = right["steps"].as_array().ok_or("Missing sharing steps")?;
            require(left.len() == right.len(), "Sharing step count differs")?;
            for (left, right) in left.iter().zip(right) {
                if left["path"] == "/api/utils/env" {
                    continue;
                }
                if mode == "standalone"
                    && left["path"]
                        .as_str()
                        .is_some_and(|path| path.starts_with("/share/"))
                    && left["method"] != "GET"
                {
                    for (step, kind) in [(left, "node"), (right, "rust")] {
                        require(
                            step["response"]["headers"].get("etag").is_none(),
                            "Forwarded error must not have ETag",
                        )?;
                        diagnostic(
                            &step["response"],
                            "default",
                            kind,
                            500,
                            "Error: Method not allowed",
                            true,
                            step["method"] == "HEAD",
                        )?;
                    }
                } else {
                    contracts::compare_step(left, right)?;
                }
            }
        }
        json::check(&states[0], &states[1], false)?;
    }
    Ok(())
}
