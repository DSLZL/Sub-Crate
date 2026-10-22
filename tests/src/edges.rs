use crate::{
    Result,
    api::{self, Api, files},
    fixture::quote,
    json, process, raw_http, reference, require,
    server::{self, Backend},
    service, sharing,
};
use serde_json::{Value, json};
use std::{collections::BTreeMap, path::Path, process::Command, time::Duration};

struct Edge<'a> {
    api: &'a Api,
    observed: Vec<Value>,
}
impl Edge<'_> {
    fn send(
        &mut self,
        method: &str,
        path: &str,
        body: Option<Value>,
        status: u16,
        html: bool,
    ) -> Result<Value> {
        let mut step = json!({"method":method,"path":path});
        if let Some(body) = body {
            step["body"] = body;
        }
        let response = self.api.response(&step)?;
        require(
            response["status"] == status,
            format!("Edge response mismatch {method} {path}: {response}"),
        )?;
        let value = if html {
            sharing::diagnostic(&response, "production", "unused", status, "", true, false)?;
            json!({"status":status,"content_type":response["headers"]["content-type"],"body":response["body"]})
        } else {
            let media = if path == "/api/storage" && method == "GET" {
                "application/json; charset=utf-8"
            } else {
                "text/plain; charset=utf-8"
            };
            require(
                response["headers"]["content-type"] == media,
                "Edge Content-Type mismatch",
            )?;
            json!({"status":status,"content_type":response["headers"]["content-type"],"json":json::parse(response["body"].as_str().ok_or("Missing edge body")?)?})
        };
        self.observed
            .push(json!({"request":step,"response":value,"state":self.api.durable()?}));
        Ok(if html {
            value["body"].clone()
        } else {
            value["json"].clone()
        })
    }
    fn get(&mut self, path: &str) -> Result<Value> {
        self.send("GET", path, None, 200, false)
    }
    fn change(&mut self, method: &str, path: &str, body: Value) -> Result<Value> {
        self.send(
            method,
            path,
            Some(body),
            if method == "POST" && path != "/api/storage" {
                201
            } else {
                200
            },
            false,
        )
    }
}
pub fn settings_and_keys(output: &Path) -> Result {
    reference::verify(output)?;
    let generated = process::checked(
        Command::new(reference::node())
            .arg(crate::compat().join("fixtures/service-recipient-keys.mjs"))
            .current_dir(crate::root().join("backend")),
        output,
        "recipient-oracle",
        Duration::from_secs(30),
    )?;
    let keys = json::parse(&generated.stdout)?;
    json::write_new(
        output.join("recipient-fixture.json"),
        &json!({"sha256":process::sha256(&std::fs::read(crate::compat().join("fixtures/service-recipient-keys.mjs"))?) ,"keys":keys}),
    )?;
    let mut paired = Vec::new();
    for (kind, backend) in [
        ("node", Backend::Node(reference::node())),
        ("rust", service::target()?),
    ] {
        let label = format!("{kind}-edges");
        let state = output.join(&label);
        std::fs::create_dir(&state)?;
        let mut environment = server::environment(&state);
        server::set(&mut environment, "SUB_STORE_BACKEND_PREFIX", "true");
        server::set(
            &mut environment,
            "SUB_STORE_FRONTEND_BACKEND_PATH",
            "/backend",
        );
        environment.remove(std::ffi::OsStr::new("NODE_ENV"));
        let api = Api::start(&backend, &environment, &state, output, &label)?;
        let mut edge = Edge {
            api: &api,
            observed: Vec::new(),
        };
        let fields = [
            "defaultTimeout",
            "githubApiTimeout",
            "artifactSyncBatchSize",
            "cacheThreshold",
            "resourceCacheTtl",
            "headersCacheTtl",
            "scriptCacheTtl",
            "logsMaxCount",
            "backendRequestConcurrency",
            "backendRequestConcurrencyWaitTime",
        ];
        let values = json!([
            null,
            false,
            true,
            0,
            -1,
            0.5,
            2,
            2.5,
            "",
            " ",
            "1",
            "1.5",
            "0x20",
            "0X20",
            "0b10",
            "0B10",
            "0o10",
            "0O10",
            "+0x10",
            "-0b1",
            "0x",
            "Infinity",
            "NaN",
            "\u{feff}2\u{feff}",
            "\u{85}2",
            "\u{2003}2\u{2028}",
            [],
            [null],
            [false],
            [true],
            ["2"],
            [[["2"]]],
            ["2", "3"],
            [{}],
            {},
            ["0x20"],
            [[null]],
            [-0.0],
            [[[]]]
        ]);
        for value in values
            .as_array()
            .ok_or("Missing numeric cases")?
            .iter()
            .chain(std::iter::once(&Value::Null))
        {
            let body = Value::Object(
                fields
                    .iter()
                    .map(|key| (key.to_string(), value.clone()))
                    .collect(),
            );
            edge.change("PATCH", "/api/settings", body)?;
        }
        for field in fields {
            for value in [
                json!({"toString":null}),
                json!([{"toString":"not callable"}]),
                json!([[{"toString":false}]]),
            ] {
                let before = files(&api)?;
                let error = edge.send(
                    "PATCH",
                    "/api/settings",
                    Some(json!({field:value})),
                    500,
                    false,
                )?["error"]
                    .clone();
                json::check(
                    &error,
                    &json!({"code":"FAILED_TO_UPDATE_SETTINGS","type":"InternalServerError","message":"Failed to update settings","details":"Reason: Cannot convert object to primitive value"}),
                    false,
                )?;
                require(
                    files(&api)? == before,
                    "Rejected numeric coercion changed files",
                )?;
            }
        }
        for (plural, singular) in [("subs", "sub"), ("collections", "collection")] {
            for (index, key) in keys["valid"]
                .as_array()
                .ok_or("Missing valid age recipients")?
                .iter()
                .enumerate()
            {
                let key = key.as_str().ok_or("Invalid recipient fixture")?;
                let item = json!({"name":format!("key-{index}"),"age-public-key":format!("\u{feff} \n{key}\r "),"editorLanguage":"invalid","previewLanguage":"yaml","process":[{"type":"Script Operator","args":{"content":"// 保留🙂\r\n","editorLanguage":"bad"}}],"custom":{"null":null,"false":false,"zero":0}});
                let created = edge.change("POST", &format!("/api/{plural}"), item)?["data"].clone();
                require(
                    created["age-public-key"] == key
                        && created.get("editorLanguage").is_none()
                        && created["process"][0]["args"] == json!({"content":"// 保留🙂\r\n"}),
                    "Recipient/editor normalization changed",
                )?;
                let path = format!("/api/{singular}/key-{index}");
                for blank in [
                    Value::Null,
                    json!(""),
                    json!("\u{feff} \n"),
                    json!([]),
                    json!([null]),
                    json!([[[]]]),
                ] {
                    require(
                        edge.change("PATCH", &path, json!({"age-public-key":blank}))?["data"]
                            .get("age-public-key")
                            .is_none(),
                        "Blank recipient retained",
                    )?;
                }
                require(
                    edge.change("PATCH", &path, json!({"age-public-key":[[key]]}))?["data"]["age-public-key"]
                        == key,
                    "Nested recipient coercion changed",
                )?;
                let mut invalid = keys["invalid"]
                    .as_array()
                    .ok_or("Missing invalid recipients")?
                    .clone();
                invalid.extend([
                    json!("age1invalid"),
                    json!("age1tag1invalid"),
                    json!("age1tagpq1invalid"),
                    json!(0),
                    json!(false),
                    json!({}),
                    json!([key, ""]),
                    json!(format!("\u{85}{key}")),
                ]);
                for invalid in invalid {
                    let before = std::fs::read(state.join("sub-store.json"))?;
                    let response = api.response(
                        &json!({"method":"PATCH","path":path,"body":{"age-public-key":invalid}}),
                    )?;
                    require(
                        response["status"] == 500
                            && response["headers"]["content-type"] == "text/html; charset=utf-8"
                            && response["headers"]["content-security-policy"]
                                == "default-src 'none'"
                            && response["headers"]["x-content-type-options"] == "nosniff",
                        format!("Invalid recipient PATCH diagnostics changed ({kind}): {response}"),
                    )?;
                    require(response["body"] == "<!DOCTYPE html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n<title>Error</title>\n</head>\n<body>\n<pre>[object Object]</pre>\n</body>\n</html>\n", "Recipient object diagnostics changed")?;
                    edge.observed.push(json!({"method":"PATCH","path":path,"body":{"age-public-key":invalid},"html":response["body"],"state":api.durable()?}));
                    let rejected = edge.send(
                        "POST",
                        &format!("/api/{plural}"),
                        Some(json!({"name":"invalid","age-public-key":invalid})),
                        500,
                        false,
                    )?;
                    require(
                        rejected["error"]["code"] == "INVALID_AGE_PUBLIC_KEY"
                            && std::fs::read(state.join("sub-store.json"))? == before,
                        "Invalid recipient changed state or error",
                    )?;
                }
                require(
                    edge.send(
                        "POST",
                        &format!("/api/{plural}"),
                        Some(json!({"name":"bad/name","age-public-key":"age1invalid"})),
                        500,
                        false,
                    )?["error"]["code"]
                        == "INVALID_AGE_PUBLIC_KEY",
                    "Recipient validation order changed",
                )?;
            }
            let before = std::fs::read(state.join("sub-store.json"))?;
            require(edge.send("PUT",&format!("/api/{plural}"),Some(json!([{"name":"valid","age-public-key":keys["valid"][0]},{"name":"invalid","age-public-key":keys["invalid"][0]}])),500,false)?["error"]["code"]=="INVALID_AGE_PUBLIC_KEY"&&std::fs::read(state.join("sub-store.json"))?==before,"Rejected recipient bulk replacement changed state")?;
            let items=keys["valid"].as_array().unwrap().iter().enumerate().map(|(index,key)|json!({"name":format!("bulk-{index}"),"age-public-key":[key],"previewLanguage":"bad"})).collect::<Vec<_>>();
            edge.change("PUT", &format!("/api/{plural}"), json!(items))?;
        }
        json::write_new(output.join(format!("{label}.json")), &json!(edge.observed))?;
        paired.push(edge.observed);
    }
    json::check(&json!(paired[0]), &json!(paired[1]), false)
}
fn crud(edge: &mut Edge) -> Result {
    let api = edge.api;
    for (old, renamed) in [("1", "renamed🙂"), ("0", "zero-renamed")] {
        let patched = edge.change(
            "PATCH",
            &format!("/api/sub/{old}"),
            json!({"name":renamed,"subscriptions":["ignored"]}),
        )?["data"]
            .clone();
        require(
            patched["subscriptions"] == json!(["reserved"])
                && patched["unknown"] == json!({"zero":0,"flag":false,"text":"保留🙂\r\n"}),
            "Rename changed reserved/unknown fields",
        )?;
    }
    edge.change(
        "PATCH",
        "/api/collection/1",
        json!({"name":"collection-renamed"}),
    )?;
    let document = edge.get("/api/storage")?;
    json::check(
        &json!(
            document["subs"]
                .as_array()
                .ok_or("Missing subs")?
                .iter()
                .map(|item| item["name"].clone())
                .collect::<Vec<_>>()
        ),
        &json!(["renamed🙂", "zero-renamed"]),
        false,
    )?;
    let stored = api.durable()?;
    json::check(
        &stored["collections"][0]["subscriptions"],
        &json!(["renamed🙂", "1", "zero-renamed", "0", 1, 0]),
        false,
    )?;
    let expected = json!([
        "renamed🙂",
        "01",
        "renamed🙂",
        "renamed🙂",
        "zero-renamed",
        "zero-renamed",
        null,
        "renamed🙂",
        "renamed🙂",
        "zero-renamed",
        [],
        {}
    ]);
    for (field, kind, type_field, source) in [
        ("artifacts", "subscription", "type", "source"),
        ("files", "subscription", "sourceType", "sourceName"),
    ] {
        let actual = stored[field]
            .as_array()
            .ok_or("Missing references")?
            .iter()
            .filter(|item| item[type_field] == kind)
            .map(|item| item[source].clone())
            .collect::<Vec<_>>();
        json::check(&json!(actual), &expected, false)?;
    }
    let actual = stored["artifacts"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|item| item["type"] == "collection")
        .map(|item| item["source"].clone())
        .collect::<Vec<_>>();
    json::check(
        &json!(actual),
        &json!(["collection-renamed", 1, true, [1]]),
        false,
    )?;
    let renamed = format!("/api/sub/{}", quote("renamed🙂"));
    let before = std::fs::read(api.state.join("sub-store.json"))?;
    require(
        edge.get(&renamed)?["data"].get("subscriptions").is_none()
            && edge.get("/api/subs")?["data"][0]
                .get("subscriptions")
                .is_none()
            && std::fs::read(api.state.join("sub-store.json"))? == before,
        "GET persisted live-object cleanup",
    )?;
    edge.change("PATCH", &renamed, json!({}))?;
    require(
        api.durable()?["subs"][0].get("subscriptions").is_none(),
        "PATCH did not persist prior cleanup",
    )?;
    for path in [
        &renamed,
        "/api/sub/zero-renamed?mode=permanent",
        "/api/collection/collection-renamed?mode=",
    ] {
        edge.send("DELETE", path, None, 200, false)?;
    }
    let stored = api.durable()?;
    require(
        stored["collections"][0]["subscriptions"] == json!(["1", "0", 1, 0])
            && stored["artifacts"][0]["source"] == "renamed🙂",
        "Deletion rewrote unrelated references",
    )?;
    for (plural, singular) in [("subs", "sub"), ("collections", "collection")] {
        let path = format!("/api/{singular}/missing");
        let list = format!("/api/{plural}");
        let before = std::fs::read(api.state.join("sub-store.json"))?;
        for mode in ["", "?mode=", "?mode=permanent", "?mode=invalid"] {
            require(
                edge.send("DELETE", &format!("{path}{mode}"), None, 500, false)?["error"]["code"]
                    == if mode.ends_with("invalid") {
                        "INVALID_DELETE_MODE"
                    } else {
                        "RESOURCE_NOT_FOUND"
                    },
                "Delete error precedence changed",
            )?;
        }
        edge.send("PATCH", &path, Some(json!({})), 404, false)?;
        if plural == "subs" {
            edge.send("GET", &path, None, 500, true)?;
        } else {
            json::check(
                &edge.send("GET", &path, None, 500, false)?["error"],
                &json!({"code":"SUBSCRIPTION_NOT_FOUND","type":"ResourceNotFoundError","message":"Collection missing does not exist","details":404}),
                false,
            )?;
        }
        require(
            std::fs::read(api.state.join("sub-store.json"))? == before,
            "Missing-resource request changed state",
        )?;
        for item in [
            json!({"name":""}),
            json!({"name":null}),
            json!({}),
            json!({"name":false}),
            json!({"name":0}),
            json!({"name":1.5}),
        ] {
            edge.change("POST", &list, item.clone())?;
            let before = std::fs::read(api.state.join("sub-store.json"))?;
            require(
                edge.send("POST", &list, Some(item), 500, false)?["error"]["code"]
                    == "DUPLICATE_KEY"
                    && std::fs::read(api.state.join("sub-store.json"))? == before,
                "Duplicate primitive name changed state",
            )?;
        }
        for item in [json!({"name":["alias"]}), json!({"name":{"alias":true}})] {
            edge.change("POST", &list, item.clone())?;
            edge.change("POST", &list, item)?;
        }
        for value in [1.0, 1e-6, 1e-7, 1e20, 1e21] {
            edge.change("POST", &list, json!({"name":value}))?;
            require(
                edge.send("POST", &list, Some(json!({"name":value})), 500, false)?["error"]["code"]
                    == "DUPLICATE_KEY",
                "Numeric name duplicate changed",
            )?;
        }
        for field in ["name", "age-public-key"] {
            let before = files(api)?;
            json::check(
                &edge.send(
                    "POST",
                    &list,
                    Some(json!({field:{"toString":"not callable"}})),
                    500,
                    false,
                )?["error"],
                &json!({"message":"Cannot convert object to primitive value"}),
                false,
            )?;
            require(
                files(api)? == before,
                "Invalid primitive coercion changed files",
            )?;
        }
        let before = std::fs::read(api.state.join("sub-store.json"))?;
        for value in [json!("bad/name"), json!(["bad/name"])] {
            require(
                edge.send("POST", &list, Some(json!({"name":value})), 500, false)?["error"]["code"]
                    == "INVALID_NAME",
                "Invalid slash name accepted",
            )?;
        }
        require(
            std::fs::read(api.state.join("sub-store.json"))? == before,
            "Invalid slash name changed state",
        )?;
        for position in ["top", "bottom"] {
            edge.change(
                "PATCH",
                "/api/settings",
                json!({"appearanceSetting":{"createItemPosition":position}}),
            )?;
            edge.change(
                "POST",
                &list,
                json!({"name":position,"subscriptions":["keep-only-on-collections"]}),
            )?;
            let items = edge.get(&list)?["data"].clone();
            let items = items.as_array().ok_or("Missing resource list")?;
            require(
                if position == "top" {
                    &items[0]
                } else {
                    items.last().ok_or("Empty list")?
                }["name"]
                    == position,
                "Create item position changed",
            )?;
        }
        let process = json!([null,false,{"type":"Script Operator","args":{"content":"require('node:fs')\r\n🙂","editorLanguage":"plaintext"}}]);
        let mut dirty = process.clone();
        dirty[2]["args"]["previewLanguage"] = json!("bad");
        edge.change("PUT",&list,json!([{"name":"editable","subscriptions":["replace-keeps-this"],"unknown":{"nil":null},"editorLanguage":"yaml","previewLanguage":"bad","process":dirty},{"name":"duplicate","subscriptions":[]},{"name":"duplicate","subscriptions":[]}]))?;
        let patch_path = format!("/api/{singular}/editable");
        for value in [Value::Null, json!(false), json!(0), json!("")] {
            let changed = edge.change(
                "PATCH",
                &patch_path,
                json!({"name":value,"subscriptions":["patch-input"],"opaque":value}),
            )?["data"]
                .clone();
            require(
                changed["name"] == "editable"
                    && changed["unknown"] == json!({"nil":null})
                    && changed["subscriptions"]
                        == (if plural == "subs" {
                            json!(["replace-keeps-this"])
                        } else {
                            json!(["patch-input"])
                        })
                    && changed["process"] == process,
                "PATCH normalization/reserved fields changed",
            )?;
        }
        let before = std::fs::read(api.state.join("sub-store.json"))?;
        require(
            edge.send(
                "DELETE",
                &format!("{patch_path}?mode=invalid"),
                None,
                500,
                false,
            )?["error"]["code"]
                == "INVALID_DELETE_MODE"
                && std::fs::read(api.state.join("sub-store.json"))? == before,
            "Invalid delete changed state",
        )?;
        edge.change("PATCH", &patch_path, json!({"name":"duplicate"}))?;
        edge.send(
            "DELETE",
            &format!("/api/{singular}/duplicate?mode=permanent"),
            None,
            200,
            false,
        )?;
        json::check(
            &edge.get(&list)?["data"],
            &json!([{"name":"duplicate","subscriptions":[]},{"name":"duplicate","subscriptions":[]}]),
            false,
        )?;
    }
    for (plural, singular) in [("subs", "sub"), ("collections", "collection")] {
        let list = format!("/api/{plural}");
        let item_path = format!("/api/{singular}/typed");
        let before = std::fs::read(api.state.join("sub-store.json"))?;
        for value in [Value::Null, json!(false), json!(0), json!("scalar🙂")] {
            for (method, path) in [("POST", &list), ("PUT", &list), ("PATCH", &item_path)] {
                edge.send(method, path, Some(value.clone()), 400, true)?;
                require(
                    std::fs::read(api.state.join("sub-store.json"))? == before,
                    "Rejected scalar JSON changed state",
                )?;
            }
        }
        for value in [json!([]), json!([null,false,{"opaque":"数组🙂\r\n"}])] {
            edge.change("PUT", &list, json!([]))?;
            let created = edge.change("POST", &list, value.clone())?["data"].clone();
            let expected = if plural == "subs" {
                Value::Object(
                    value
                        .as_array()
                        .unwrap()
                        .iter()
                        .enumerate()
                        .map(|(index, value)| (index.to_string(), value.clone()))
                        .collect(),
                )
            } else {
                value
            };
            json::check(&created, &expected, false)?;
            let before = std::fs::read(api.state.join("sub-store.json"))?;
            require(
                edge.send("POST", &list, Some(json!({})), 500, false)?["error"]["code"]
                    == "DUPLICATE_KEY"
                    && std::fs::read(api.state.join("sub-store.json"))? == before,
                "Duplicate unnamed record changed state",
            )?;
        }
        let mut target = json!({"name":"typed","opaque":{"zero":0,"flag":false}});
        edge.change("PUT", &list, json!([target]))?;
        let before = files(api)?;
        json::check(
            &edge.send(
                "PUT",
                &list,
                Some(json!([target,{"age-public-key":{"toString":null}}])),
                500,
                false,
            )?["error"],
            &json!({"message":"Cannot convert object to primitive value"}),
            false,
        )?;
        edge.send(
            "PATCH",
            &item_path,
            Some(json!({"age-public-key":{"toString":null}})),
            500,
            true,
        )?;
        require(
            files(api)? == before,
            "Rejected typed coercion changed files",
        )?;
        for value in [json!([]), json!([null,false,{"opaque":"PATCH🙂\r\n"}])] {
            let changed = edge.change("PATCH", &item_path, value.clone())?["data"].clone();
            for (index, value) in value.as_array().unwrap().iter().enumerate() {
                target[index.to_string()] = value.clone();
            }
            json::check(&changed, &target, false)?;
        }
        let mut mixed = json!([false,0,"text🙂\r\n",[],[null,{"name":"nested"}],target,null]);
        edge.change("PUT", &list, mixed.clone())?;
        json::check(&edge.get(&list)?["data"], &mixed, false)?;
        json::check(&edge.get(&item_path)?["data"], &target, false)?;
        let mut patched = target.clone();
        patched["0"] = json!("after-scalars");
        json::check(
            &edge.change("PATCH", &item_path, json!(["after-scalars"]))?["data"],
            &patched,
            false,
        )?;
        let items = mixed.as_array_mut().unwrap();
        items.pop();
        items.insert(0, Value::Null);
        edge.change("PUT", &list, mixed.clone())?;
        let before = std::fs::read(api.state.join("sub-store.json"))?;
        edge.send("GET", &item_path, None, 500, true)?;
        edge.send("PATCH", &item_path, Some(json!([])), 500, true)?;
        for (method, path, body) in [
            ("DELETE", &item_path, None),
            ("POST", &list, Some(json!({"name":"new"}))),
        ] {
            json::check(
                &edge.send(method, path, body, 500, false)?["error"],
                &json!({"message":"Cannot read properties of null (reading 'name')"}),
                false,
            )?;
        }
        require(
            edge.send(
                "DELETE",
                &format!("{item_path}?mode=invalid"),
                None,
                500,
                false,
            )?["error"]["code"]
                == "INVALID_DELETE_MODE",
            "Invalid delete precedence changed",
        )?;
        json::check(&edge.get(&list)?["data"], &mixed, false)?;
        require(
            std::fs::read(api.state.join("sub-store.json"))? == before,
            "Null-record lookup changed state",
        )?;
    }
    Ok(())
}
fn reference_graphs(edge: &mut Edge, kind: &str) -> Result {
    let api = edge.api;
    let original = api.durable()?;
    let cases = [
        ("null-record", Value::Null),
        ("false-record", json!(false)),
        ("number-record", json!(3)),
        ("string-record", json!("record")),
        ("array-record", json!([])),
        ("missing", json!({})),
        ("null-names", json!({"subscriptions":null})),
        ("false-names", json!({"subscriptions":false})),
        ("number-names", json!({"subscriptions":3})),
        ("object-names", json!({"subscriptions":{}})),
        ("empty-string", json!({"subscriptions":""})),
        ("matching-string", json!({"subscriptions":"1"})),
        ("other-string", json!({"subscriptions":"other"})),
        ("valid", json!({"subscriptions":["1",1,"1",false,null]})),
        ("stored-buffer", json!({"type":"Buffer","data":[0,65]})),
        ("stored-empty-buffer", json!({"type":"Buffer","data":[]})),
        ("live-buffer", json!([0, 65])),
        ("live-empty-buffer", json!([])),
        ("artifact-coercion", json!({"subscriptions":["1"]})),
        ("file-coercion", json!({"subscriptions":["1"]})),
    ];
    for (id, collection) in cases {
        for method in ["PATCH", "DELETE"] {
            let mut document = original.clone();
            document["subs"] = json!([{"name":"1","opaque":{"keep":true}}]);
            document["collections"] = json!([{"name":"first","subscriptions":["1","1",1]},collection,{"name":"last","subscriptions":["1"]}]);
            if id.starts_with("stored-") {
                document["collections"] = collection.clone();
            } else if id.starts_with("live-") {
                document["collections"] = json!([]);
            }
            if id == "artifact-coercion" {
                document["artifacts"] = json!([{"type":"subscription","source":{"toString":null}}]);
            } else if id == "file-coercion" {
                document["files"] =
                    json!([{"sourceType":"subscription","sourceName":{"toString":null}}]);
            }
            api.send(&json!({"method":"POST","path":"/api/storage","body":{"content":json::stringify(&document)}}),200)?;
            if id.starts_with("live-") {
                let bytes = if id == "live-buffer" {
                    b"\0A".as_slice()
                } else {
                    b"".as_slice()
                };
                let setup = raw_http::request(
                    &api.listener.url,
                    "PUT",
                    "/api/collections",
                    &BTreeMap::from([("Content-Type".into(), "application/octet-stream".into())]),
                    bytes,
                    None,
                )?;
                require(setup.status == 200, format!("Live Buffer reference setup failed ({kind}/{id}/{method}): {}", setup.evidence()))?;
            }
            let before = files(api)?;
            let published_before = api.send(&json!({"path":"/api/storage"}), 200)?;
            let mut step = json!({"method":method,"path":"/api/sub/1?mode=permanent"});
            if method == "PATCH" {
                step["body"] = json!({"name":"renamed"});
            }
            let response = api.response(&step)?;
            let after = files(api)?;
            let published_after = api.send(&json!({"path":"/api/storage"}), 200)?;
            let expected = if id == "valid"
                || id == "live-empty-buffer"
                || (method == "PATCH"
                    && ["empty-string", "matching-string", "other-string"].contains(&id))
                || (method == "DELETE" && id.ends_with("-coercion"))
            {
                200
            } else {
                500
            };
            require(
                response["status"] == expected,
                format!("Reference graph {id}/{method} changed status: {response}"),
            )?;
            require(
                after[1] == before[1],
                "Reference mutation changed root backup",
            )?;
            if expected == 500 {
                if kind == "rust" {
                    require(
                        before == after,
                        "Rejected reference mutation changed Rust durable files",
                    )?;
                    json::check(&published_after, &published_before, false)?;
                } else if method == "PATCH" {
                    require(before == after, "Failed Node rename persisted early")?;
                    if !id.starts_with("stored-") && !id.starts_with("live-") {
                        json::check(
                            &published_after["collections"][0]["subscriptions"],
                            &json!(["renamed", "1", 1]),
                            false,
                        )?;
                    }
                } else {
                    json::check(&api.durable()?["subs"], &json!([]), false)?;
                }
            } else {
                let stored = api.durable()?;
                if id == "live-empty-buffer" {
                    json::check(
                        &stored["collections"],
                        &json!({"type":"Buffer","data":[]}),
                        false,
                    )?;
                } else {
                    json::check(
                        &stored["collections"][0]["subscriptions"],
                        &if method == "PATCH" {
                            json!(["renamed", "1", 1])
                        } else {
                            json!([1])
                        },
                        false,
                    )?;
                }
                json::check(
                    &stored["subs"],
                    &if method == "PATCH" {
                        json!([{"name":"renamed","opaque":{"keep":true}}])
                    } else {
                        json!([])
                    },
                    false,
                )?;
            }
            edge.observed.push(json!({"id":format!("reference-{id}-{method}"),"response":{"status":response["status"],"body":response["body"]},"headers":(["content-type","content-length","etag","content-security-policy","x-content-type-options"].iter().map(|key|(key.to_string(),response["headers"].get(*key).cloned().unwrap_or(Value::Null))).collect::<serde_json::Map<_,_>>()) }));
        }
    }
    let mut restored = original;
    for field in ["subs", "collections"] {
        restored[field] = json!([null,false,0,"恢复🙂\r\n",[],[null,{"name":"nested"}],{"0":false,"1":null,"name":"restored","opaque":{"zero":0}}]);
    }
    edge.change(
        "POST",
        "/api/storage",
        json!({"content":json::stringify(&restored)}),
    )?;
    json::check(
        &json::parse(
            api.stored("root.json")?["sub-store"]
                .as_str()
                .ok_or("Missing restored root")?,
        )?,
        &restored,
        false,
    )?;
    json::check(&edge.get("/api/storage")?, &restored, false)
}
pub fn crud_lifecycle(output: &Path) -> Result {
    reference::verify(output)?;
    let unknown = json!({"zero":0,"flag":false,"text":"保留🙂\r\n"});
    let sources = json!(["1", "01", 1, true, 0, false, null, ["1"], [1], [0], [], {}]);
    let sources = sources.as_array().unwrap();
    let mut artifacts=sources.iter().enumerate().map(|(index,value)|json!({"name":format!("sub-{index}"),"type":"subscription","source":value})).collect::<Vec<_>>();
    artifacts.extend([json!("1"),json!(1),json!(true),json!([1])].iter().enumerate().map(|(index,value)|json!({"name":format!("col-{index}"),"type":"collection","source":value})));
    let seed = json!({"schemaVersion":"2.0","settings":{},"rules":[],"modules":[],"tokens":[],"archives":[],"subs":[{"name":"1","subscriptions":["reserved"],"unknown":unknown},{"name":"0","subscriptions":["reserved"],"unknown":unknown}],"collections":[{"name":"refs","subscriptions":["1","1","0","0",1,0]},{"name":"1","subscriptions":[]}],"artifacts":artifacts,"files":sources.iter().enumerate().map(|(index,value)|json!({"name":format!("sub-{index}"),"sourceType":"subscription","sourceName":value})).collect::<Vec<_>>(),"opaque":unknown});
    let mut paired = Vec::new();
    let mut final_states = Vec::new();
    let mut rust_state = None;
    for (kind, backend) in [
        ("node", Backend::Node(reference::node())),
        ("rust", service::target()?),
    ] {
        let label = format!("{kind}-crud");
        let (state, mut environment) = service::seed(output, &label, &seed, &json!({}))?;
        server::set(&mut environment, "NODE_ENV", "production");
        server::set(&mut environment, "SUB_STORE_BODY_JSON_LIMIT", "32kb");
        let api = Api::start(&backend, &environment, &state, output, &label)?;
        if kind == "rust" {
            api::denied_write(
                &api,
                &json!({"method":"PATCH","path":"/api/sub/1","body":{"name":"must-not-commit"}}),
                "/api/storage",
                "sub-store.json",
                "STATE_WRITE_FAILED",
            )?;
        }
        let mut edge = Edge {
            api: &api,
            observed: Vec::new(),
        };
        crud(&mut edge)?;
        reference_graphs(&mut edge, kind)?;
        edge.observed.extend(crate::media::run(
            &api,
            output,
            &format!("{label}-body-http"),
            None,
        )?);
        json::write_new(output.join(format!("{label}.json")), &json!(edge.observed))?;
        paired.push(edge.observed);
        let expected = api.durable()?;
        drop(api);
        let restarted = Api::start(
            &backend,
            &environment,
            &state,
            output,
            &format!("{label}-restart"),
        )?;
        json::check(
            &restarted.send(&json!({"path":"/api/storage"}), 200)?,
            &expected,
            false,
        )?;
        final_states.push(expected);
        if kind == "rust" {
            rust_state = Some(state);
        }
    }
    json::check(&json!(paired[0]), &json!(paired[1]), false)?;
    json::check(&final_states[0], &final_states[1], false)?;
    let rollback = output.join("node-rollback");
    std::fs::create_dir(&rollback)?;
    for file in ["sub-store.json", "root.json"] {
        std::fs::copy(
            rust_state.as_ref().ok_or("Missing Rust state")?.join(file),
            rollback.join(file),
        )?;
    }
    let mut environment = server::environment(&rollback);
    server::set(&mut environment, "SUB_STORE_BACKEND_PREFIX", "true");
    server::set(
        &mut environment,
        "SUB_STORE_FRONTEND_BACKEND_PATH",
        "/backend",
    );
    let api = Api::start(
        &Backend::Node(reference::node()),
        &environment,
        &rollback,
        output,
        "rollback",
    )?;
    json::check(
        &api.send(&json!({"path":"/api/storage"}), 200)?,
        &final_states[1],
        false,
    )
}
