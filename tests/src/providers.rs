use crate::{
    Result,
    fixture::{Reply, Request, quote, unquote},
    json, require,
};
use serde_json::{Map, Value, json};
use std::sync::{
    Mutex,
    atomic::{AtomicUsize, Ordering},
};
use std::time::Duration;

pub const USER_AGENT: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_4) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/81.0.4044.141 Safari/537.36";

#[derive(Default)]
pub struct ScriptGist {
    stores: Mutex<(Map<String, Value>, Map<String, Value>)>,
    active: AtomicUsize,
}

impl ScriptGist {
    pub fn restore(&self,snapshot:&Value)->Result {
        *self.stores.lock().unwrap()=(snapshot["github"].as_object().ok_or("Invalid Github fixture store")?.clone(),snapshot["gitlab"].as_object().ok_or("Invalid Gitlab fixture store")?.clone());Ok(())
    }
    pub fn snapshot(&self) -> Value {
        let stores = self.stores.lock().unwrap();
        json!({"github":stores.0,"gitlab":stores.1})
    }

    pub fn response(&self, request: &Request, origin: &str) -> Result<Reply> {
        let url = request.url(origin)?;
        let parts = url
            .path()
            .trim_start_matches("/host/gist/")
            .split('/')
            .map(unquote)
            .collect::<Result<Vec<_>>>()?;
        let provider = parts.first().ok_or("Missing fixture provider")?.as_str();
        if provider == "reset" && request.method == "POST" {
            let mut stores = self.stores.lock().unwrap();
            stores.0.clear();
            stores.1.clear();
            return Ok(Reply::json(200, &json!({})));
        }
        let valid = if provider == "gitlab" {
            request.header("PRIVATE-TOKEN") == Some("helper-token")
                && request.header("Authorization").is_none()
        } else {
            request.header("Authorization") == Some("token helper-token")
                && request.header("PRIVATE-TOKEN").is_none()
        };
        if !valid {
            return Ok(Reply::new(
                403,
                br#"{"message":"fixture credentials rejected"}"#.to_vec(),
            ));
        }
        if provider == "queue" {
            let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
            std::thread::sleep(Duration::from_millis(30));
            let name = parts.get(1).ok_or("Missing queue name")?;
            let reply = if name == "rejected" {
                Reply::json(503, &json!({"message":"queue rejected"}))
            } else {
                Reply::json(200, &json!({"name":name,"active":active}))
            };
            self.active.fetch_sub(1, Ordering::SeqCst);
            return Ok(reply);
        }
        if provider == "errors" {
            let (status, body) = match parts.get(1).map(String::as_str) {
                Some("nested") => (400, r#"{"message":{"error":"nested"},"error":"ignored"}"#),
                Some("error") => (401, r#"{"message":"ignored","error":"second"}"#),
                Some("message") => (429, r#"{"message":"third"}"#),
                Some("text") => (503, "raw fixture error"),
                Some("empty") => (404, ""),
                Some("object") => (500, r#"{"message":{"code":1}}"#),
                Some("invalid") => (200, "not json"),
                Some("delay") => {
                    std::thread::sleep(Duration::from_millis(200));
                    (200, "[]")
                }
                _ => return Err("Unknown provider error fixture".into()),
            };
            return Ok(Reply::new(status, body));
        }
        if !["github", "gitlab"].contains(&provider) {
            return Ok(Reply::json(
                404,
                &json!({"message":"unknown fixture provider"}),
            ));
        }
        let github = provider == "github";
        let mut stores = self.stores.lock().unwrap();
        let store = if github { &mut stores.0 } else { &mut stores.1 };
        let resource = if github { "gists" } else { "snippets" };
        let parts = &parts[1..];
        let metadata = |key: &str, store: &Map<String, Value>| {
            let mut item = store[key].clone();
            item["id"] = json!(key);
            item["files"] = if github {
                Value::Object(
                    store[key]["files"]
                        .as_object()
                        .unwrap()
                        .iter()
                        .map(|(name, file)| {
                            let mut file = file.clone();
                            file["filename"] = json!(name);
                            file["raw_url"] = json!(format!(
                                "{origin}/host/gist/github/raw/{key}/{}",
                                quote(name)
                            ));
                            (name.clone(), file)
                        })
                        .collect(),
                )
            } else {
                Value::Array(
                    store[key]["files"]
                        .as_object()
                        .unwrap()
                        .keys()
                        .map(|name| json!({"path":name}))
                        .collect(),
                )
            };
            item
        };
        if parts == [resource] && request.method == "GET" {
            return Ok(Reply::json(
                200,
                &Value::Array(store.keys().map(|key| metadata(key, store)).collect()),
            ));
        }
        if parts == [resource] && request.method == "POST" {
            let mut data = json::parse(std::str::from_utf8(&request.body)?)?;
            let key = format!("{}{}", if github { "g" } else { "s" }, store.len() + 1);
            if !github {
                data["files"] = Value::Object(
                    data["files"]
                        .as_array()
                        .ok_or("Invalid snippet files")?
                        .iter()
                        .map(|item| {
                            Ok((
                                item["file_path"]
                                    .as_str()
                                    .ok_or("Missing snippet path")?
                                    .to_owned(),
                                json!({"content":item["content"]}),
                            ))
                        })
                        .collect::<Result<_>>()?,
                );
            }
            store.insert(key.clone(), data);
            return Ok(Reply::json(201, &metadata(&key, store)));
        }
        if parts.len() == 2 && parts[0] == resource && store.contains_key(&parts[1]) {
            let key = &parts[1];
            if request.method == if github { "PATCH" } else { "PUT" } {
                let body = json::parse(std::str::from_utf8(&request.body)?)?;
                let files = store.get_mut(key).unwrap()["files"]
                    .as_object_mut()
                    .ok_or("Invalid fixture files")?;
                if github {
                    for (name, value) in body["files"].as_object().ok_or("Invalid fixture patch")? {
                        if value.is_null() {
                            files.remove(name);
                        } else {
                            files.insert(name.clone(), value.clone());
                        }
                    }
                } else {
                    for action in body["files"].as_array().ok_or("Invalid snippet patch")? {
                        let name = action["file_path"].as_str().ok_or("Missing snippet path")?;
                        if action["action"] == "delete" {
                            files.remove(name);
                        } else {
                            files.insert(name.to_owned(), json!({"content":action["content"]}));
                        }
                    }
                }
            }
            if ["GET", "PATCH", "PUT"].contains(&request.method.as_str()) {
                return Ok(Reply::json(200, &metadata(key, store)));
            }
        }
        if parts.len() == 3
            && parts[0] == "raw"
            && request.method == "GET"
            && let Some(content) = store
                .get(&parts[1])
                .and_then(|gist| gist["files"].get(&parts[2]))
                .and_then(|file| file["content"].as_str())
        {
            return Ok(Reply::new(200, content).header("Content-Type", "text/plain; charset=utf-8"));
        }
        Ok(Reply::json(
            404,
            &json!({"message":"fixture resource missing"}),
        ))
    }
}

pub fn expected_gist_requests() -> Vec<Value> {
    struct Wire(Vec<Value>);
    impl Wire {
        fn request(
            &mut self,
            path: &str,
            method: &str,
            body: Option<Value>,
            patch: bool,
            gitlab: bool,
            override_headers: bool,
        ) {
            let mut headers = json!({"accept":"*/*","user-agent":USER_AGENT});
            if gitlab {
                headers["private-token"] = json!("helper-token");
            } else {
                headers["authorization"] = json!("token helper-token");
            }
            if path == "reset" {
                headers = json!({"accept":"*/*"});
            }
            if patch {
                headers["accept-encoding"] = json!("gzip, deflate, br");
            }
            if gitlab && ["POST", "PUT"].contains(&method) {
                headers["content-type"] = json!("application/json");
            }
            if override_headers {
                headers["user-agent"] = json!("Gist override/1");
                headers["x-gist"] = json!("yes");
                headers["x-gist-event"] = json!("mutated");
            }
            let body_bytes = body
                .map(|value| serde_json::to_vec(&value).unwrap())
                .unwrap_or_default();
            self.0.push(json!({"method":method,"path":format!("/host/gist/{path}"),"body_bytes":body_bytes,"headers":headers}));
        }
        fn locate(&mut self, gitlab: bool) {
            self.request(
                if gitlab {
                    "gitlab/snippets"
                } else {
                    "github/gists?per_page=100&page=1"
                },
                "GET",
                None,
                false,
                gitlab,
                false,
            );
        }
        fn update(&mut self, files: Value, gitlab: bool) {
            self.locate(gitlab);
            self.request(
                if gitlab {
                    "gitlab/snippets/s1"
                } else {
                    "github/gists/g2"
                },
                if gitlab { "PUT" } else { "PATCH" },
                Some(json!({"files":files})),
                !gitlab,
                gitlab,
                false,
            );
        }
    }
    let mut wire = Wire(Vec::new());
    wire.request("reset", "POST", None, false, false, false);
    for _ in 0..3 {
        wire.locate(false);
    }
    wire.request(
        "github/gists",
        "POST",
        Some(json!({"description":"other","public":false,"files":{"other":{"content":"other"}}})),
        false,
        false,
        false,
    );
    wire.locate(false);
    wire.request("github/gists","POST",Some(json!({"description":"helper","public":false,"files":{"订阅🙂.txt":{"content":"订阅🙂\r\nsecond\n"},"remove.txt":{"content":"remove"}}})),false,false,false);
    wire.locate(false);
    wire.request("github/gists/g2", "GET", None, false, false, false);
    wire.request(
        &format!("github/raw/g2/{}", quote("订阅🙂.txt")),
        "GET",
        None,
        false,
        false,
        false,
    );
    wire.update(
        json!({"订阅🙂.txt":{"content":"changed"},"remove.txt":null,"zero":{"content":"0"}}),
        false,
    );
    wire.update(
        json!({"订阅🙂.txt":null,"zero":null,"keep":{"content":"placeholder"}}),
        false,
    );
    wire.update(json!({}), false);
    wire.update(json!({"real":{"content":"restored"},"keep":null}), false);
    for _ in 0..4 {
        wire.locate(false);
    }
    for mode in ["nested", "error", "message", "text", "empty", "object"] {
        wire.request(&format!("errors/{mode}"), "GET", None, false, false, false);
    }
    wire.request(
        "errors/invalid/gists?per_page=100&page=1",
        "GET",
        None,
        false,
        false,
        false,
    );
    wire.request("errors/delay", "GET", None, false, false, false);
    wire.locate(false);
    for method in ["get", "post", "put", "patch", "delete"] {
        wire.request(
            &format!("queue/{method}"),
            &method.to_uppercase(),
            None,
            false,
            false,
            false,
        );
    }
    for name in ["rejected", "recovery"] {
        wire.request(&format!("queue/{name}"), "GET", None, false, false, false);
    }
    wire.locate(true);
    wire.locate(true);
    wire.request("gitlab/snippets","POST",Some(json!({"title":"snippet","visibility":"private","files":[{"file_path":"alpha","content":"一🙂"},{"file_path":"gone","content":"delete"}]})),false,true,false);
    wire.update(json!([{"action":"update","file_path":"alpha","content":"changed"},{"action":"create","file_path":"beta","content":"second"},{"action":"delete","file_path":"gone"}]),true);
    for _ in 0..3 {
        wire.locate(true);
    }
    wire.update(json!([{"action":"delete","file_path":"alpha"},{"action":"delete","file_path":"beta"},{"action":"create","file_path":"keep","content":"placeholder"}]),true);
    wire.update(json!([]), true);
    wire.update(json!([{"action":"create","file_path":"real","content":"restored"},{"action":"delete","file_path":"keep"}]),true);
    wire.locate(true);
    wire.request("gitlab/gists/s1", "GET", None, false, true, false);
    wire.locate(true);
    wire.locate(false);
    wire.request("github/gists/g2", "GET", None, false, false, false);
    wire.request("github/raw/g2/real", "GET", None, false, false, false);
    wire.request(
        "github/gists?per_page=100&page=1",
        "GET",
        None,
        false,
        false,
        true,
    );
    wire.locate(false);
    wire.0
}

pub fn check_gist_requests(requests: &[Value]) -> Result {
    let keys = [
        "authorization",
        "private-token",
        "user-agent",
        "accept",
        "content-type",
        "accept-encoding",
        "x-gist",
        "x-gist-event",
    ];
    let actual:Vec<_>=requests.iter().map(|item| json!({"method":item["method"],"path":item["path"],"body_bytes":item["body_bytes"],"headers":item["headers"].as_object().unwrap().iter().filter(|(key,_)| keys.contains(&key.as_str())).map(|(key,value)| (key.clone(),value.clone())).collect::<Map<_,_>>()})).collect();
    json::check(&json!(actual), &json!(expected_gist_requests()), false)
}

pub fn check_gist_state(state: &Value) -> Result {
    json::check(
        state,
        &json!({"github":{"g1":{"description":"other","public":false,"files":{"other":{"content":"other"}}},"g2":{"description":"helper","public":false,"files":{"real":{"content":"restored"}}}},"gitlab":{"s1":{"title":"snippet","visibility":"private","files":{"real":{"content":"restored"}}}}}),
        false,
    )
}

pub fn patch_gists(store: &mut Value, request: &Request, origin: &str) -> Result<Reply> {
    if request.header("Authorization") == Some("token rejected-fixture-token") {
        return Ok(Reply::json(
            403,
            &json!({"message":"fixture credentials rejected"}),
        ));
    }
    let url = request.url(origin)?;
    let parts = url
        .path()
        .trim_matches('/')
        .split('/')
        .map(unquote)
        .collect::<Result<Vec<_>>>()?;
    let metadata = |key: &str, store: &Value| {
        let mut item = store[key].clone();
        item["id"] = json!(key);
        item["html_url"] = json!(format!("{origin}/gists/{key}"));
        item["files"] = Value::Object(
            store[key]["files"]
                .as_object()
                .unwrap()
                .iter()
                .map(|(name, file)| {
                    let mut file = file.clone();
                    file["filename"] = json!(name);
                    file["raw_url"] = json!(format!(
                        "{origin}/gist-raw/{key}/raw/revision/{}",
                        quote(name)
                    ));
                    (name.clone(), file)
                })
                .collect(),
        );
        item
    };
    if parts.first().is_some_and(|part| part == "gist-raw") && request.method == "GET" {
        if let Some(content) = parts
            .get(1)
            .and_then(|key| store.get(key))
            .and_then(|gist| parts.last().and_then(|name| gist["files"].get(name)))
            .and_then(|file| file["content"].as_str())
        {
            return Ok(Reply::new(200, content).header("Content-Type", "text/plain; charset=utf-8"));
        }
        return Ok(Reply::json(404, &json!({"message":"fixture file missing"})));
    }
    if parts == ["gists"] && request.method == "GET" {
        return Ok(Reply::json(
            200,
            &Value::Array(
                store
                    .as_object()
                    .ok_or("Invalid Gist store")?
                    .keys()
                    .map(|key| metadata(key, store))
                    .collect(),
            ),
        ));
    }
    if parts == ["gists"] && request.method == "POST" {
        let key = format!(
            "gist-{}",
            store.as_object().ok_or("Invalid Gist store")?.len() + 1
        );
        store[&key] = json::parse(std::str::from_utf8(&request.body)?)?;
        return Ok(Reply::json(201, &metadata(&key, store)));
    }
    if parts.len() == 2 && parts[0] == "gists" && store.get(&parts[1]).is_some() {
        let key = &parts[1];
        if request.method == "PATCH" {
            let patch = json::parse(std::str::from_utf8(&request.body)?)?;
            let files = store[key]["files"]
                .as_object_mut()
                .ok_or("Invalid Gist files")?;
            for (name, value) in patch["files"].as_object().ok_or("Invalid Gist patch")? {
                if value.is_null() {
                    files.remove(name);
                } else {
                    files.insert(name.clone(), value.clone());
                }
            }
        }
        if ["GET", "PATCH"].contains(&request.method.as_str()) {
            return Ok(Reply::json(200, &metadata(key, store)));
        }
    }
    Ok(Reply::json(404, &json!({"message":"fixture gist missing"})))
}

pub fn require_complete(results: &[Value]) -> Result {
    require(
        !results.is_empty() && results.iter().all(|case| case["status"] == "passed"),
        format!("Ordered contract suite failed: {}", results.iter().find(|case| case["status"] != "passed").map(crate::json::stringify).unwrap_or_else(|| "empty suite".into())),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn gist_wire_rejects_changed_auth_bytes_order_and_missing_requests() {
        crate::actions_only();
        let expected = expected_gist_requests();
        check_gist_requests(&expected).unwrap();
        for mutation in [
            "method",
            "path",
            "body",
            "auth",
            "other-provider-auth",
            "ua",
            "missing",
            "order",
            "duplicate",
        ] {
            let mut actual = expected.clone();
            match mutation {
                "method" => actual[1]["method"] = json!("POST"),
                "path" => {
                    actual[1]["path"] =
                        json!(format!("{}&extra=1", actual[1]["path"].as_str().unwrap()))
                }
                "body" => actual[4]["body_bytes"]
                    .as_array_mut()
                    .unwrap()
                    .push(json!(10)),
                "auth" => actual[1]["headers"]["authorization"] = json!("token wrong"),
                "other-provider-auth" => {
                    actual[1]["headers"]["private-token"] = json!("helper-token")
                }
                "ua" => {
                    actual[1]["headers"]
                        .as_object_mut()
                        .unwrap()
                        .remove("user-agent");
                }
                "missing" => {
                    actual.pop();
                }
                "order" => actual.swap(0, 1),
                "duplicate" => actual.push(actual.last().unwrap().clone()),
                _ => unreachable!(),
            }
            assert!(
                check_gist_requests(&actual).is_err(),
                "Mutation {mutation} was accepted"
            );
        }
    }
}
