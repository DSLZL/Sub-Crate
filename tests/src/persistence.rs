use crate::{
    Result,
    api::{self, Api},
    fixture::{quote, unquote},
    json, reference, require,
    server::{self, Backend},
    service,
};
use base64::{
    Engine,
    engine::general_purpose::{STANDARD, URL_SAFE},
};
use serde_json::{Value, json};
use std::{collections::BTreeMap, path::Path};

const CACHES: [&str; 3] = [
    "sub-store-cached-resource",
    "sub-store-cached-headers-resource",
    "sub-store-cached-script-resource",
];

pub fn root_fixture() -> Value {
    let mut root = json!({"opaque":{"text":"根缓存🙂\r\n","zero":0,"flag":false,"nil":null},"sub-store":"previous backup"});
    for cache in CACHES {
        root[cache] = json!(json::stringify(
            &json!({"fixture":{"time":4102444800000_i64,"data":"cached"}})
        ));
    }
    root
}

fn truth(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(value) => *value,
        Value::Number(value) => value.as_f64() != Some(0.0),
        Value::String(value) => !value.is_empty(),
        _ => true,
    }
}

pub fn filename(
    response: &Value,
    prefix: &str,
    lower: i64,
    upper: i64,
    zone: &jiff::tz::TimeZone,
) -> Result {
    require(
        response["headers"]["access-control-expose-headers"] == "content-disposition",
        "Export header is not exposed",
    )?;
    let encoded = response["headers"]["content-disposition"]
        .as_str()
        .and_then(|value| value.strip_prefix("attachment; filename=\""))
        .and_then(|value| value.strip_suffix('"'))
        .ok_or("Malformed attachment filename")?;
    require(
        !encoded.contains(['"', '\r', '\n']),
        "Unsafe attachment filename",
    )?;
    let decoded = unquote(encoded)?;
    let mut roundtrip = quote(&decoded);
    for (escaped, literal) in [
        ("%21", "!"),
        ("%27", "'"),
        ("%28", "("),
        ("%29", ")"),
        ("%2A", "*"),
    ] {
        roundtrip = roundtrip.replace(escaped, literal);
    }
    require(
        roundtrip == encoded,
        "Filename is not encodeURIComponent encoded",
    )?;
    require(
        (lower.div_euclid(1000) - 1..=upper.div_euclid(1000)).any(|second| {
            jiff::Timestamp::from_second(second).is_ok_and(|time| {
                decoded
                    == format!(
                        "{prefix}_{}.json",
                        time.to_zoned(zone.clone()).strftime("%Y-%m-%d_%H-%M-%S")
                    )
            })
        }),
        "Export filename is outside actual server-local clock bounds",
    )
}

fn encodings(document: &Value) -> Result<BTreeMap<String, String>> {
    let raw = json::stringify(document).into_bytes();
    let padded = |remainder: usize| {
        let mut bytes = raw.clone();
        bytes.extend(std::iter::repeat_n(
            b' ',
            (remainder + 3 - raw.len() % 3) % 3,
        ));
        bytes
    };
    let encoded = STANDARD.encode(padded(1));
    let urlsafe = URL_SAFE.encode(padded(1));
    let split = |value: &str, size, separator: &str| {
        value
            .as_bytes()
            .chunks(size)
            .map(|part| std::str::from_utf8(part).unwrap())
            .collect::<Vec<_>>()
            .join(separator)
    };
    let mut variants = BTreeMap::from([
        ("urlsafe-padded".into(), urlsafe.clone()),
        (
            "urlsafe-unpadded".into(),
            urlsafe.trim_end_matches('=').into(),
        ),
        ("separated-urlsafe".into(), split(&urlsafe, 17, "\r\n!☃=")),
        ("interior-padding".into(), split(&encoded, 11, "==")),
        (
            "orphan-sextet".into(),
            format!("{}Z", STANDARD.encode(padded(0))),
        ),
    ]);
    let alphabet = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    for (remainder, unused) in [(1, 15), (2, 3)] {
        let mut canonical = STANDARD
            .encode(padded(remainder))
            .trim_end_matches('=')
            .to_owned();
        let last = canonical.pop().ok_or("Empty base64 fixture")? as u8;
        let changed = alphabet[alphabet
            .iter()
            .position(|byte| *byte == last)
            .ok_or("Invalid base64 fixture")?
            | unused];
        require(last != changed, "Trailing-bit fixture is canonical")?;
        canonical.push(char::from(changed));
        variants.insert(format!("trailing-bits-{remainder}"), canonical);
    }
    Ok(variants)
}

struct Backup<'a> {
    api: &'a Api,
    zone: jiff::tz::TimeZone,
    observed: Vec<Value>,
}
impl Backup<'_> {
    fn send(
        &mut self,
        method: &str,
        path: &str,
        body: Option<Value>,
        status: u16,
        export: Option<&str>,
    ) -> Result<Value> {
        let before = api::files(self.api)?;
        let lower = service::clock(false);
        let mut step = json!({"method":method,"path":path});
        if let Some(body) = body {
            step["body"] = body;
        }
        let response = self.api.response(&step)?;
        let upper = service::clock(true);
        let after = api::files(self.api)?;
        let stored = self.api.durable()?;
        let root = self.api.stored("root.json")?;
        require(
            response["status"] == status,
            format!("Backup response changed: {step} {response}"),
        )?;
        require(
            response["headers"]["content-type"]
                == if export.is_some() {
                    "application/json; charset=utf-8"
                } else {
                    "text/plain; charset=utf-8"
                },
            "Backup response media changed",
        )?;
        json::check(
            &root["opaque"],
            &json!({"text":"根缓存🙂\r\n","zero":0,"flag":false,"nil":null}),
            false,
        )?;
        if status >= 400 {
            require(before == after, "Rejected restore changed durable files")?;
        }
        if let Some(prefix) = export {
            filename(&response, prefix, lower, upper, &self.zone)?;
        }
        let parsed = json::parse(
            response["body"]
                .as_str()
                .ok_or("Missing backup response body")?,
        )?;
        let root = root
            .as_object()
            .ok_or("Invalid root")?
            .iter()
            .filter(|(key, _)| {
                ["opaque", "sub-store"].contains(&key.as_str()) || CACHES.contains(&key.as_str())
            })
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect::<serde_json::Map<_, _>>();
        self.observed.push(json!({"method":method,"path":path,"status":status,"json":parsed,"state":stored,"root":root}));
        Ok(parsed)
    }
    fn import(&mut self, content: String) -> Result<Value> {
        self.send(
            "POST",
            "/api/storage",
            Some(json!({"content":content})),
            200,
            None,
        )
    }
    fn settings(&mut self) -> Result<Value> {
        Ok(self.send("GET", "/api/settings", None, 200, None)?["data"].clone())
    }
    fn export(&mut self) -> Result<Value> {
        self.send("GET", "/api/storage", None, 200, Some("sub-store_data"))
    }
}

fn backup_edges(test: &mut Backup) -> Result {
    let resource = "导出 name!()'*🙂";
    for (plural, singular, file) in [
        ("subs", "sub", "subscription"),
        ("collections", "collection", "collection"),
    ] {
        let item = json!({"name":resource,"custom":{"zero":0,"false":false,"nil":null},"content":"保留🙂\r\nrequire('node:fs')"});
        let created =
            test.send("POST", &format!("/api/{plural}"), Some(item), 201, None)?["data"].clone();
        let exported = test.send(
            "GET",
            &format!("/api/{singular}/{}?raw=1", quote(resource)),
            None,
            200,
            Some(&format!("sub-store_{file}_{resource}")),
        )?;
        json::check(&exported, &created, false)?;
    }
    let before = api::files(test.api)?;
    json::check(&test.export()?, &test.api.durable()?, false)?;
    require(
        api::files(test.api)? == before,
        "Export changed persisted files",
    )?;
    for content in [
        "invalid backup".into(),
        json::stringify(&json!({"unknown":"missing settings"})),
        json::stringify(&json!({"settings":null})),
        "!W!==".into(),
        "!W10=".into(),
        encodings(&json!({"settings":null}))?["separated-urlsafe"].clone(),
    ] {
        require(
            test.send(
                "POST",
                "/api/storage",
                Some(json!({"content":content})),
                500,
                None,
            )?["error"]["code"]
                == "INVALID_BACKUP_DATA",
            "Invalid backup error changed",
        )?;
        require(
            api::files(test.api)? == before,
            "Invalid import changed persisted files",
        )?;
    }
    let backup = json::read(crate::compat().join("fixtures/frontend-backup.json"))?;
    let mut legacy = backup.clone();
    legacy["subs"] = json!({"10":{"name":"ten"},"2":{"name":"two","display-name":"旧名字🙂"}});
    legacy["collections"] = json!({"c":{"name":"legacy-col","subscriptions":["two"],"ua":"old"}});
    for field in ["files", "artifacts", "tokens"] {
        legacy[field] = json!({});
    }
    legacy
        .as_object_mut()
        .unwrap()
        .shift_remove("schemaVersion");
    let array_settings = json!([null,false,0,"设置🙂\r\n",[null,{"nested":true}],{"script":"require('node:fs')\r\n","opaque":{"zero":0,"nil":null}}]);
    let mut empty = backup.clone();
    empty["settings"] = json!([]);
    empty["subs"] = json!(false);
    empty["collections"] = json!(0);
    empty["artifacts"] = json!("");
    empty["rules"] = json!(false);
    empty["files"] = Value::Null;
    empty["tokens"] = json!(0);
    empty.as_object_mut().unwrap().shift_remove("schemaVersion");
    let mut array_backup = backup.clone();
    array_backup["settings"] = array_settings.clone();
    let mut empty_settings = backup.clone();
    empty_settings["settings"] = json!([]);
    for (index, original) in [
        backup.clone(),
        legacy,
        empty_settings,
        array_backup.clone(),
        empty,
    ]
    .iter()
    .enumerate()
    {
        for encoded in [false, true] {
            let raw = json::stringify(original);
            test.import(if encoded { STANDARD.encode(raw) } else { raw })?;
            json::check(
                &json::parse(
                    test.api.stored("root.json")?["sub-store"]
                        .as_str()
                        .ok_or("Missing backup root")?,
                )?,
                original,
                false,
            )?;
            let exported = test.export()?;
            json::check(&exported, &test.api.durable()?, false)?;
            require(
                exported.get("opaque").is_none() && exported.get("sub-store").is_none(),
                "Export included root cache",
            )?;
            if let Some(items) = original["settings"].as_array() {
                json::check(&test.settings()?, &original["settings"], false)?;
                let mut expected = Value::Object(
                    items
                        .iter()
                        .enumerate()
                        .map(|(index, value)| (index.to_string(), value.clone()))
                        .collect(),
                );
                expected["defaultTimeout"] = json!("1500");
                expected["appearanceSetting"] = json!({"createItemPosition":"top"});
                json::check(&test.send("PATCH","/api/settings",Some(json!({"defaultTimeout":"1500","appearanceSetting":{"createItemPosition":"top"}})),200,None)?["data"],&expected,false)?;
                expected["0"] = json!("覆盖🙂\r\n");
                expected["1"] = json!(false);
                json::check(
                    &test.send(
                        "PATCH",
                        "/api/settings",
                        Some(json!(["覆盖🙂\r\n", false])),
                        200,
                        None,
                    )?["data"],
                    &expected,
                    false,
                )?;
                json::check(&test.api.durable()?["settings"], &expected, false)?;
            }
            if index == 4 {
                for field in [
                    "subs",
                    "collections",
                    "artifacts",
                    "rules",
                    "files",
                    "tokens",
                ] {
                    require(
                        exported[field] == json!([]),
                        "Falsy legacy resource was not migrated to an empty list",
                    )?;
                }
            }
        }
    }
    for (value, encoded) in [
        json!(true),
        json!(false),
        json!(0),
        json!(1.25),
        json!(""),
        json!("设置🙂\r\n"),
    ]
    .into_iter()
    .map(|value| (value, false))
    .chain([(json!("设置🙂\r\n"), true)])
    {
        let mut original = backup.clone();
        original["settings"] = value.clone();
        let raw = json::stringify(&original);
        test.import(if encoded { STANDARD.encode(raw) } else { raw })?;
        json::check(
            &json::parse(
                test.api.stored("root.json")?["sub-store"]
                    .as_str()
                    .ok_or("Missing scalar backup root")?,
            )?,
            &original,
            false,
        )?;
        json::check(&test.export()?["settings"], &value, false)?;
        let before = api::files(test.api)?;
        let expected = if truth(&value) {
            value.clone()
        } else {
            json!({})
        };
        json::check(&test.settings()?, &expected, false)?;
        json::check(&test.api.durable()?["settings"], &expected, false)?;
        let after = api::files(test.api)?;
        require(
            after[1] == before[1],
            "Settings initialization changed root cache",
        )?;
        if truth(&value) {
            require(before[0] == after[0], "Truthy settings read rewrote data")?;
        }
        if value == true || value.as_f64() == Some(1.25) {
            let patch = json!({"defaultTimeout":"1500","opaque":{"nil":null,"flag":false,"text":"保留🙂\r\n"}});
            json::check(
                &test.send("PATCH", "/api/settings", Some(patch.clone()), 200, None)?["data"],
                &patch,
                false,
            )?;
        }
    }
    let mut codec = backup.clone();
    codec["codecText"] = json!("ÿÿÿ🙂\r\n");
    codec["codecOpaque"] = json!({"nil":null,"flag":false,"zero":0});
    let mut variants = encodings(&codec)?;
    require(
        variants["urlsafe-padded"].contains(['-', '_']),
        "URL-safe fixture lacks alternate symbols",
    )?;
    let mut lossy = codec.clone();
    lossy["codecText"] = json!("��(�");
    let mut invalid = codec.clone();
    invalid["codecText"] = json!("INVALID_UTF8");
    let raw = json::stringify(&invalid);
    let marker = "INVALID_UTF8";
    let offset = raw.find(marker).ok_or("Missing invalid UTF8 marker")?;
    let bytes = [
        &raw.as_bytes()[..offset],
        b"\xff\xc3(\xf0\x9f",
        &raw.as_bytes()[offset + marker.len()..],
    ]
    .concat();
    variants.insert("lossy-utf8".into(), STANDARD.encode(bytes));
    for (encoding, content) in variants {
        let expected = if encoding == "lossy-utf8" {
            &lossy
        } else {
            &codec
        };
        test.import(content)?;
        json::check(
            &json::parse(
                test.api.stored("root.json")?["sub-store"]
                    .as_str()
                    .ok_or("Missing codec backup root")?,
            )?,
            expected,
            false,
        )?;
        json::check(&test.export()?, expected, false)?;
        json::check(&test.settings()?, &expected["settings"], false)?;
    }
    test.import(json::stringify(&array_backup))?;
    json::check(&test.settings()?, &array_settings, false)?;
    let before = api::files(test.api)?[0].clone();
    test.send("GET", "/api/utils/refresh", None, 200, None)?;
    let root = test.api.stored("root.json")?;
    require(
        CACHES.iter().all(|key| root[*key] == "{}"),
        "No-Gist refresh did not clear every resource cache",
    )?;
    require(
        api::files(test.api)?[0] == before,
        "No-Gist refresh changed main state",
    )
}

fn settings_reads(api: &Api, expected: &Value) -> Result<Vec<Value>> {
    let mut observations = Vec::new();
    for path in ["/api/storage", "/api/settings"] {
        let before = api::files(api)?;
        let response = api.response(&json!({"path":path}))?;
        let after = api::files(api)?;
        require(
            response["status"] == 200 && before == after,
            "Initialized settings read changed files/status",
        )?;
        let stored = api.durable()?;
        let parsed = json::parse(
            response["body"]
                .as_str()
                .ok_or("Missing settings response")?,
        )?;
        json::check(&stored["settings"], expected, false)?;
        json::check(
            if path == "/api/storage" {
                &parsed
            } else {
                &parsed["data"]
            },
            if path == "/api/storage" {
                &stored
            } else {
                expected
            },
            false,
        )?;
        require(
            response["headers"]["content-type"]
                == if path == "/api/storage" {
                    "application/json; charset=utf-8"
                } else {
                    "text/plain; charset=utf-8"
                },
            "Settings read media changed",
        )?;
        observations.push(
            json!({"path":path,"json":parsed,"state":stored,"root":api.stored("root.json")?}),
        );
    }
    Ok(observations)
}

pub fn backups(output: &Path) -> Result {
    reference::verify(output)?;
    let backup = json::read(crate::compat().join("fixtures/frontend-backup.json"))?;
    let root = root_fixture();
    for (zone_name, zone) in [
        ("local", jiff::tz::TimeZone::system()),
        ("Asia/Tokyo", jiff::tz::TimeZone::get("Asia/Tokyo")?),
        ("Asia/Kathmandu", jiff::tz::TimeZone::get("Asia/Kathmandu")?),
    ] {
        let mut paired = Vec::new();
        for (kind, backend) in [
            ("node", Backend::Node(reference::node())),
            ("rust", service::target()?),
        ] {
            let label = format!("{kind}-backup-{}", zone_name.replace('/', "-"));
            let (state, mut env) = service::seed(output, &label, &backup, &root)?;
            if zone_name != "local" {
                server::set(&mut env, "TZ", zone_name);
            }
            let api = Api::start(&backend, &env, &state, output, &label)?;
            let mut test = Backup {
                api: &api,
                zone: zone.clone(),
                observed: Vec::new(),
            };
            backup_edges(&mut test)?;
            json::write_new(output.join(format!("{label}.json")), &json!(test.observed))?;
            let expected = api.durable()?;
            let expected_root = api.stored("root.json")?;
            paired.push(test.observed);
            if kind == "rust" && zone_name == "local" {
                for file in ["root.json", "sub-store.json"] {
                    let before = api::files(&api)?;
                    api::denied_write(
                        &api,
                        &json!({"method":"POST","path":"/api/storage","body":{"content":json::stringify(&backup)}}),
                        "/api/storage",
                        file,
                        "STATE_WRITE_FAILED",
                    )?;
                    require(
                        api::files(&api)? == before
                            && !state.join(".sub-store-restore.pending.json").exists(),
                        "Failed restore changed files or left a journal",
                    )?;
                }
                let mut falsy = expected.clone();
                falsy["settings"] = json!(false);
                api.send(&json!({"method":"POST","path":"/api/storage","body":{"content":json::stringify(&falsy)}}),200)?;
                let before = api::files(&api)?;
                api::denied_write(
                    &api,
                    &json!({"path":"/api/settings"}),
                    "/api/storage",
                    "sub-store.json",
                    "STATE_WRITE_FAILED",
                )?;
                require(
                    api::files(&api)? == before,
                    "Failed settings initialization changed files",
                )?;
                json::check(
                    &api.data(&json!({"path":"/api/settings"}))?,
                    &json!({}),
                    false,
                )?;
                api.send(&json!({"method":"POST","path":"/api/storage","body":{"content":json::stringify(&expected)}}),200)?;
                json::check(&api.durable()?, &expected, false)?;
            }
            drop(api);
            let restarted =
                Api::start(&backend, &env, &state, output, &format!("{label}-restart"))?;
            json::check(
                &restarted.send(&json!({"path":"/api/storage"}), 200)?,
                &expected,
                false,
            )?;
            json::check(
                &restarted.stored("root.json")?["sub-store"],
                &expected_root["sub-store"],
                false,
            )?;
            json::check(
                &restarted.data(&json!({"path":"/api/settings"}))?,
                &expected["settings"],
                false,
            )?;
        }
        json::check(&json!(paired[0]), &json!(paired[1]), false)?;
    }
    let source = output.join("rust-backup-local");
    let old_data = std::fs::read(source.join("sub-store.json"))?;
    let old_root = std::fs::read(source.join("root.json"))?;
    for phase in ["root-only", "both"] {
        let label = format!("restore-interrupted-{phase}");
        let state = output.join(&label);
        std::fs::create_dir(&state)?;
        std::fs::write(
            state.join("sub-store.json"),
            if phase == "both" {
                b"{\"settings\":{}}".as_slice()
            } else {
                old_data.as_slice()
            },
        )?;
        std::fs::write(state.join("root.json"), b"{\"partially_restored\":true}")?;
        let pending = state.join(".sub-store-restore.pending.json");
        json::write_new(
            &pending,
            &json!({"data":STANDARD.encode(&old_data),"root":STANDARD.encode(&old_root)}),
        )?;
        let api = Api::start(
            &service::target()?,
            &server::environment(&state),
            &state,
            output,
            &label,
        )?;
        json::check(
            &api.send(&json!({"path":"/api/storage"}), 200)?,
            &json::parse(std::str::from_utf8(&old_data)?)?,
            false,
        )?;
        require(
            std::fs::read(state.join("root.json"))? == old_root
                && std::fs::read(state.join("sub-store.pre-rust.json"))? == old_data
                && !pending.exists(),
            "Interrupted restore did not recover both files and snapshot",
        )?;
    }
    let state = output.join("restored-node-rollback");
    std::fs::create_dir(&state)?;
    for file in ["sub-store.json", "root.json"] {
        std::fs::copy(source.join(file), state.join(file))?;
    }
    let api = Api::start(
        &Backend::Node(reference::node()),
        &server::environment(&state),
        &state,
        output,
        "restored-node-rollback",
    )?;
    json::check(
        &api.send(&json!({"path":"/api/storage"}), 200)?,
        &json::parse(std::str::from_utf8(&old_data)?)?,
        false,
    )?;
    json::check(
        &api.stored("root.json")?["sub-store"],
        &json::parse(std::str::from_utf8(&old_root)?)?["sub-store"],
        false,
    )?;
    json::check(
        &api.data(&json!({"path":"/api/settings"}))?,
        &json::parse(std::str::from_utf8(&old_data)?)?["settings"],
        false,
    )?;
    drop(api);
    for (label, value) in [
        ("true", json!(true)),
        ("false", json!(false)),
        ("zero", json!(0)),
        ("number", json!(1.25)),
        ("empty", json!("")),
        ("string", json!("设置🙂\r\n")),
    ] {
        let expected = if truth(&value) {
            value.clone()
        } else {
            json!({})
        };
        let mut paired = Vec::new();
        for (kind, backend) in [
            ("node", Backend::Node(reference::node())),
            ("rust", service::target()?),
        ] {
            let label = format!("{kind}-settings-{label}");
            let mut document = backup.clone();
            document["settings"] = value.clone();
            let (state, env) = service::seed(output, &label, &document, &root)?;
            let original = std::fs::read(state.join("sub-store.json"))?;
            let api = Api::start(&backend, &env, &state, output, &label)?;
            let observations = settings_reads(&api, &expected)?;
            json::write_new(output.join(format!("{label}.json")), &json!(observations))?;
            if kind == "rust" {
                require(
                    std::fs::read(state.join("sub-store.pre-rust.json"))? == original,
                    "Scalar settings snapshot changed source bytes",
                )?;
            }
            paired.push(observations);
            drop(api);
            if kind == "rust" && truth(&value) {
                let rollback = output.join(format!("{label}-node-rollback"));
                std::fs::create_dir(&rollback)?;
                for file in ["sub-store.json", "root.json"] {
                    std::fs::copy(state.join(file), rollback.join(file))?;
                }
                let api = Api::start(
                    &Backend::Node(reference::node()),
                    &server::environment(&rollback),
                    &rollback,
                    output,
                    &format!("{label}-node-rollback"),
                )?;
                json::check(
                    &json!(settings_reads(&api, &value)?),
                    &json!(paired[1]),
                    false,
                )?;
            }
        }
        json::check(&json!(paired[0]), &json!(paired[1]), false)?;
    }
    Ok(())
}

pub fn migration(output: &Path) -> Result {
    reference::verify(output)?;
    let mut legacy = json!({"settings":{},"subs":{"z-last":{"name":"z-last","display-name":"旧数据🙂","process":[{"type":"Useless Filter"},{"type":"Set Property Operator","args":{"key":"udp","value":false}},{"type":"Flag Operator","args":false},{"type":"Script Operator","args":{"content":"require('node:fs')\r\n"}}]},"a-first":{"name":"a-first"},"empty-process":{"name":"empty-process","source":"local","display-name":false,"process":[]},"string-process":{"name":"string-process","process":"legacy-string"},"zero-length":{"name":"zero-length","process":{"length":0,"opaque":false}}},"collections":{"c":{"name":"c","ua":"old","subscriptions":["z-last"],"extra":false,"process":[{"type":"Flag Operator","args":"truthy"},{"type":"Script Operator","args":{"content":"// JS 保留\r\n🙂"}}]}},"unknown":{"nil":null,"zero":0,"flag":false},"builtin":{"must":"be removed"}});
    let process = legacy["subs"]["z-last"]["process"].as_array_mut().unwrap();
    for (key, value) in [
        ("tfo", json!([])),
        ("skip-cert-verify", Value::Null),
        ("aead", json!(true)),
        ("unknown", json!(false)),
    ] {
        process.push(json!({"type":"Set Property Operator","args":{"key":key,"value":value}}));
    }
    process.extend(json!([{"type":"Keyword Filter"},{"type":"Custom Keyword Operator"},{"type":"Flag Operator"},{"type":"Flag Operator","args":null},{"type":["opaque"],"args":{"content":"keep\r\n"}},{"type":["Keyword"]},{},false,0,"ignored",[]]).as_array().unwrap().clone());
    for key in ["10", "2", "01", "0", "4294967295", "4294967294"] {
        legacy["subs"][key] = json!({"name":format!("index-{key}")});
    }
    let original = encodings(&legacy)?["separated-urlsafe"]
        .clone()
        .into_bytes();
    let root =
        json!({"opaque":{"text":"cache🙂","zero":0,"flag":false},"sub-store-cached-resource":"{}"});
    json::write_new(
        output.join("migration-base64-input.json"),
        &json!({"encoded":std::str::from_utf8(&original)?,"decoded":legacy}),
    )?;
    let mut paired = Vec::new();
    for (kind, backend) in [
        ("node", Backend::Node(reference::node())),
        ("rust", service::target()?),
    ] {
        let label = format!("{kind}-migration");
        let state = output.join(&label);
        std::fs::create_dir(&state)?;
        std::fs::write(state.join("sub-store.json"), &original)?;
        json::write_new(state.join("root.json"), &root)?;
        let api = Api::start(
            &backend,
            &server::environment(&state),
            &state,
            output,
            &label,
        )?;
        let migrated = api.send(&json!({"path":"/api/storage"}), 200)?;
        require(
            migrated.get("builtin").is_none(),
            "Legacy builtin was retained",
        )?;
        if kind == "rust" {
            require(
                std::fs::read(state.join("sub-store.pre-rust.json"))? == original,
                "Noisy base64 migration snapshot changed bytes",
            )?;
            json::check(&api.stored("root.json")?, &root, false)?;
        }
        json::write_new(output.join(format!("{label}.json")), &migrated)?;
        paired.push(migrated);
    }
    for field in [
        "subs",
        "collections",
        "artifacts",
        "rules",
        "files",
        "tokens",
        "schemaVersion",
        "unknown",
    ] {
        json::check(&paired[0][field], &paired[1][field], false)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn backup_codec_fixtures_retain_noisy_urlsafe_orphan_and_noncanonical_trailing_bits() -> Result
    {
        crate::actions_only();
        let variants = encodings(&json!({"settings":{},"text":"ÿÿÿ🙂\r\n"}))?;
        require(
            variants.len() == 7
                && variants["urlsafe-padded"].contains(['-', '_'])
                && variants["separated-urlsafe"].contains("\r\n!☃=")
                && variants["interior-padding"].contains("==")
                && variants["orphan-sextet"].ends_with('Z'),
            "Backup decoder fixture coverage changed",
        )?;
        for remainder in [1, 2] {
            require(
                STANDARD
                    .decode(&variants[&format!("trailing-bits-{remainder}")])
                    .is_err(),
                "Negative fixture became canonical base64",
            )?;
        }
        Ok(())
    }
}
