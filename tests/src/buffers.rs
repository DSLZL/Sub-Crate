use crate::{
    Result,
    api::{self, Api},
    json, media, reference, require,
    server::{self, Backend},
    service,
};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::{Value, json};
use std::path::Path;

struct Buffers<'a> {
    requests: media::Requests<'a>,
    phase: &'a str,
}
impl Buffers<'_> {
    fn send(
        &mut self,
        method: &str,
        path: &str,
        body: Option<Value>,
        raw: Option<&[u8]>,
        status: u16,
        error: Option<Value>,
    ) -> Result<Value> {
        let html = error.as_ref().is_some_and(|value| value == "html");
        let encoded = body
            .as_ref()
            .map(|body| json::stringify(body).into_bytes())
            .unwrap_or_default();
        let mut spec = json!({"method":method,"path":path,"status":status,"media":if raw.is_some(){"application/octet-stream"}else{"application/json"},"json_error":!html,"parser_error":html});
        if status < 400 {
            spec["json_error"] = json!(false);
        }
        let parsed = self.requests.send(raw.unwrap_or(&encoded), spec)?;
        let record = self
            .requests
            .observed
            .last_mut()
            .ok_or("Missing buffer response evidence")?;
        record["phase"] = json!(self.phase);
        record["root"] = self.requests.api.stored("root.json")?;
        if !html {
            if let Some(error) = error {
                json::check(&parsed["error"], &error, false)?;
            }
            let bytes = STANDARD.decode(
                record["body_base64"]
                    .as_str()
                    .ok_or("Missing buffer response bytes")?,
            )?;
            let length = bytes.len().to_string();
            require(
                record["headers"]["etag"] == media::etag(&bytes)
                    && record["headers"]["content-length"].as_str() == Some(length.as_str()),
                "Buffer ETag/length differs from actual bytes",
            )?;
        }
        Ok(parsed)
    }
    fn get(&mut self, path: &str) -> Result<Value> {
        self.send("GET", path, None, None, 200, None)
    }
    fn run(&mut self) -> Result {
        if self.phase == "restore" {
            let original = self.requests.api.durable()?;
            self.send(
                "POST",
                "/api/storage",
                Some(json!({"content":json::literal(&json::stringify(&original))})),
                None,
                200,
                None,
            )?;
            json::check(
                &json::parse(&json::text(
                    &self.requests.api.stored("root.json")?["sub-store"],
                )?)?,
                &original,
                false,
            )?;
        }
        for (plural, singular, label) in [
            ("subs", "sub", "Subscription"),
            ("collections", "collection", "Collection"),
        ] {
            let live = ["live", "live-before-restart"].contains(&self.phase);
            let list = format!("/api/{plural}");
            let item = format!("/api/{singular}/buffer-missing");
            if self.phase == "live-before-restart" {
                self.send(
                    "PUT",
                    &list,
                    Some(json!([{"name":"buffer-array"}])),
                    None,
                    200,
                    None,
                )?;
                json::check(
                    &self.get(&format!("/api/{singular}/buffer-array"))?["data"],
                    &json!({"name":"buffer-array"}),
                    false,
                )?;
            }
            let vectors = if self.phase == "live" {
                vec![
                    Some(b"A\0\xff".as_slice()),
                    Some(b"".as_slice()),
                    Some(b"\0A".as_slice()),
                ]
            } else if live {
                vec![Some(b"\0A".as_slice())]
            } else {
                vec![None]
            };
            for raw in vectors {
                if live {
                    self.send("PUT", &list, None, raw, 200, None)?;
                }
                let expected = if let Some(bytes) = raw {
                    json!({"type":"Buffer","data":bytes})
                } else {
                    self.requests.api.durable()?[plural].clone()
                };
                json::check(&self.get(&list)?["data"], &expected, false)?;
                let missing_get = if !live || plural == "subs" {
                    json!("html")
                } else {
                    json!({"code":"SUBSCRIPTION_NOT_FOUND","type":"ResourceNotFoundError","message":"Collection buffer-missing does not exist","details":404})
                };
                self.send("GET", &item, None, None, 500, Some(missing_get.clone()))?;
                let missing = json!({"code":"RESOURCE_NOT_FOUND","type":"ResourceNotFoundError","message":format!("{label} buffer-missing does not exist!")});
                self.send(
                    "PATCH",
                    &item,
                    Some(json!({})),
                    None,
                    if live { 404 } else { 500 },
                    Some(if live { missing.clone() } else { json!("html") }),
                )?;
                self.send("DELETE",&format!("{item}?mode=invalid"),None,None,500,Some(json!({"code":"INVALID_DELETE_MODE","type":"RequestInvalidError","message":"Unsupported delete mode: invalid"})))?;
                self.send(
                    "DELETE",
                    &format!("{item}?mode=permanent"),
                    None,
                    None,
                    500,
                    Some(if live {
                        missing
                    } else {
                        json!({"message":"e.find is not a function"})
                    }),
                )?;
                let push = json!({"message":if live{"e.push is not a function"}else{"e.find is not a function"}});
                self.send(
                    "POST",
                    &list,
                    Some(json!({"name":"buffer-new"})),
                    None,
                    500,
                    Some(push.clone()),
                )?;
                let duplicate = if live
                    && raw.is_some_and(|bytes| bytes.first().is_some_and(|byte| *byte != 0))
                {
                    json!({"code":"DUPLICATE_KEY","type":"RequestInvalidError","message":format!("{label} undefined already exists.")})
                } else {
                    push
                };
                self.send("POST", &list, Some(json!({})), None, 500, Some(duplicate))?;
                self.send(
                    "PUT",
                    &list,
                    Some(expected),
                    None,
                    500,
                    Some(json!({"message":"r.forEach is not a function"})),
                )?;
                self.send("GET", &item, None, None, 500, Some(missing_get.clone()))?;
                if live {
                    self.send(
                        "PATCH",
                        "/api/settings",
                        Some(json!({"appearanceSetting":{"createItemPosition":"top"}})),
                        None,
                        200,
                        None,
                    )?;
                    self.send(
                        "POST",
                        &list,
                        Some(json!({"name":"buffer-new"})),
                        None,
                        500,
                        Some(json!({"message":"e.unshift is not a function"})),
                    )?;
                    self.send(
                        "PATCH",
                        "/api/settings",
                        Some(json!({"appearanceSetting":{"createItemPosition":"bottom"}})),
                        None,
                        200,
                        None,
                    )?;
                    self.send("GET", &item, None, None, 500, Some(missing_get))?;
                }
            }
        }
        json::check(
            &self.get("/api/storage")?,
            &self.requests.api.durable()?,
            false,
        )
    }
}
pub fn phase(
    api: &Api,
    base: &str,
    output: &Path,
    label: &str,
    phase: &str,
    origin: Option<&str>,
) -> Result<Vec<Value>> {
    let mut test = Buffers {
        requests: media::Requests {
            api,
            base,
            output,
            label,
            origin,
            observed: Vec::new(),
        },
        phase,
    };
    test.run()?;
    json::write_new(
        output.join(format!("{label}.json")),
        &json!(test.requests.observed),
    )?;
    Ok(test.requests.observed)
}
pub fn run(output: &Path) -> Result {
    reference::verify(output)?;
    let seed = crate::edges::seed_document();
    let mut paired = Vec::new();
    let mut rust_state = None;
    for (kind, backend) in [
        ("node", Backend::Node(reference::node())),
        ("rust", service::target()?),
    ] {
        let label = format!("{kind}-buffers");
        let mut document = seed.clone();
        document["subs"] = json!([]);
        document["collections"] = json!([]);
        let mut root = crate::persistence::root_fixture();
        root["opaque"] = json!({"zero":0,"flag":false,"text":"保留🙂\r\n"});
        let (state, mut env) = service::seed(output, &label, &document, &root)?;
        server::set(&mut env, "NODE_ENV", "production");
        let api = Api::start(&backend, &env, &state, output, &label)?;
        let mut observed = phase(
            &api,
            &api.listener.url,
            output,
            &format!("{label}-live"),
            "live",
            None,
        )?;
        if kind == "rust" {
            let before = api::files(&api)?;
            let path = state.join("sub-store.json");
            let saved = state.join("saved-state.json");
            std::fs::rename(&path, &saved)?;
            std::fs::create_dir(&path)?;
            let checked = (|| -> Result {
                for plural in ["subs", "collections"] {
                    let failed = api.send(
                        &json!({"method":"PUT","path":format!("/api/{plural}"),"body":[]}),
                        500,
                    )?;
                    require(
                        failed["error"]["code"] == "STATE_WRITE_FAILED",
                        "Buffer failed write error changed",
                    )?;
                    json::check(&api.send(&json!({"method":"POST","path":format!("/api/{plural}"),"body":{"name":"buffer-new"}}),500)?,&json!({"status":"failed","error":{"message":"e.push is not a function"}}),false)?;
                }
                require(api.send(&json!({"method":"POST","path":"/api/storage","body":{"content":json::literal(&json::stringify(&seed))}}),500)?["error"]["code"]=="STATE_WRITE_FAILED","Buffer failed restore error changed")?;
                json::check(&api.send(&json!({"method":"POST","path":"/api/collections","body":{"name":"buffer-new"}}),500)?,&json!({"status":"failed","error":{"message":"e.push is not a function"}}),false)
            })();
            std::fs::remove_dir(&path)?;
            std::fs::rename(saved, path)?;
            checked?;
            require(
                api::files(&api)? == before,
                "Failed Buffer writes changed either file",
            )?;
            json::write_new(
                output.join("buffer-write-failure.json"),
                &json!({"both_files_unchanged":true,"live_identity_preserved":true}),
            )?;
        }
        observed.extend(phase(
            &api,
            &api.listener.url,
            output,
            &format!("{label}-restore"),
            "restore",
            None,
        )?);
        observed.extend(phase(
            &api,
            &api.listener.url,
            output,
            &format!("{label}-before-restart"),
            "live-before-restart",
            None,
        )?);
        drop(api);
        let api = Api::start(&backend, &env, &state, output, &format!("{label}-restart"))?;
        observed.extend(phase(
            &api,
            &api.listener.url,
            output,
            &format!("{label}-restart"),
            "stored",
            None,
        )?);
        if kind == "rust" {
            rust_state = Some(state);
        }
        paired.push(observed);
    }
    json::check(&json!(paired[0]), &json!(paired[1]), false)?;
    let state = output.join("buffers-node-rollback");
    std::fs::create_dir(&state)?;
    for file in ["sub-store.json", "root.json"] {
        std::fs::copy(
            rust_state
                .as_ref()
                .ok_or("Missing Buffer Rust state")?
                .join(file),
            state.join(file),
        )?;
    }
    let mut env = server::environment(&state);
    server::set(&mut env, "NODE_ENV", "production");
    let api = Api::start(
        &Backend::Node(reference::node()),
        &env,
        &state,
        output,
        "buffers-node-rollback",
    )?;
    let rolled = phase(
        &api,
        &api.listener.url,
        output,
        "buffers-node-rollback",
        "stored",
        None,
    )?;
    json::check(
        &json!(rolled),
        &json::read(output.join("rust-buffers-restart.json"))?,
        false,
    )
}
