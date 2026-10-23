use crate::{
    Result,
    api::Api,
    contracts,
    http::Http,
    json, reference, require,
    server::{self, Backend, Environment},
    upstream::Upstream,
};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::sync::{
    Mutex,
    atomic::{AtomicUsize, Ordering},
};
use std::time::{Duration, Instant};

pub fn target() -> Result<Backend> {
    if let Some(image) = std::env::var_os("TEST_DOCKER_IMAGE") {
        return Ok(Backend::Docker(
            image.into_string().map_err(|_| "Invalid image name")?,
        ));
    }
    Ok(Backend::Rust(
        PathBuf::from(
            std::env::var_os("TEST_RUST_BINARY").ok_or("Missing delivered Rust executable")?,
        )
        .canonicalize()?,
    ))
}
pub fn seed(
    output: &Path,
    label: &str,
    document: &Value,
    root: &Value,
) -> Result<(PathBuf, Environment)> {
    let state = output.join(label);
    std::fs::create_dir(&state)?;
    json::write_new(state.join("sub-store.json"), document)?;
    json::write_new(state.join("root.json"), root)?;
    let mut environment = server::environment(&state);
    server::set(&mut environment, "SUB_STORE_BACKEND_PREFIX", "true");
    server::set(
        &mut environment,
        "SUB_STORE_FRONTEND_BACKEND_PATH",
        "/backend",
    );
    Ok((state, environment))
}
pub fn baseline(output: &Path) -> Result<(Value, Value)> {
    let state = output.join("baseline-state");
    std::fs::create_dir(&state)?;
    let mut environment = server::environment(&state);
    server::set(&mut environment, "SUB_STORE_BACKEND_PREFIX", "true");
    server::set(
        &mut environment,
        "SUB_STORE_FRONTEND_BACKEND_PATH",
        "/backend",
    );
    let api = Api::start(
        &Backend::Node(reference::node()),
        &environment,
        &state,
        output,
        "baseline",
    )?;
    contracts::check(
        &api.http,
        &api.listener.url,
        &state,
        &contracts::cases("service-state")?,
        None,
        &output.join("baseline.json"),
    )?;
    Ok((api.durable()?, api.stored("root.json")?))
}
pub fn clock(latest: bool) -> i64 {
    #[cfg(windows)]
    {
        #[repr(C)]
        #[derive(Default)]
        struct FileTime {
            low: u32,
            high: u32,
        }
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn GetSystemTimeAsFileTime(value: *mut FileTime);
            fn GetSystemTimePreciseAsFileTime(value: *mut FileTime);
        }
        let mut standard = FileTime::default();
        let mut precise = FileTime::default();
        unsafe {
            GetSystemTimeAsFileTime(&mut standard);
            GetSystemTimePreciseAsFileTime(&mut precise);
        }
        let millis = |time: FileTime| {
            (((u64::from(time.high) << 32) | u64::from(time.low)) / 10000) as i64 - 11644473600000
        };
        let samples = [millis(standard), millis(precise)];
        if latest {
            *samples.iter().max().unwrap()
        } else {
            *samples.iter().min().unwrap()
        }
    }
    #[cfg(not(windows))]
    {
        let _ = latest;
        jiff::Timestamp::now().as_millisecond()
    }
}

pub fn compare_cases(left: &[Value], right: &[Value], skip_env: bool) -> Result {
    require(left.len() == right.len(), "Paired scenario count differs")?;
    for (left, right) in left.iter().zip(right) {
        json::check(&left["id"], &right["id"], false)?;
        let left = left["steps"].as_array().ok_or("Missing scenario steps")?;
        let right = right["steps"].as_array().ok_or("Missing scenario steps")?;
        require(left.len() == right.len(), "Paired step count differs")?;
        for (left, right) in left.iter().zip(right) {
            if skip_env && left["path"] == "/api/utils/env" {
                continue;
            }
            contracts::compare_step(left, right)?;
        }
    }
    Ok(())
}
pub fn captured_clocks(cases: &[Value], lower: i64, upper: i64) -> Result {
    for case in cases {
        for step in case["steps"].as_array().ok_or("Missing scenario steps")? {
            if let Some(captures) = step.get("captured").and_then(Value::as_object) {
                for (key, value) in captures {
                    let value = if let Some(value) = value.as_i64() {
                        Some(value)
                    } else if key.ends_with("ARCHIVE_ID") {
                        Some(
                            value
                                .as_str()
                                .ok_or("Invalid archive ID")?
                                .split('-')
                                .next()
                                .ok_or("Missing archive time")?
                                .parse()?,
                        )
                    } else {
                        None
                    };
                    if let Some(value) = value {
                        require(
                            (lower..=upper).contains(&value),
                            "Captured timestamp outside actual request bounds",
                        )?;
                    }
                }
            }
        }
    }
    Ok(())
}
pub fn conversions(output: &Path) -> Result {
    reference::verify(output)?;
    let (document, root) = baseline(output)?;
    let mut cases = contracts::cases("conversion-local")?;
    contracts::isolate_age_requests(&mut cases)?;
    let parse = cases
        .iter()
        .find(|case| case["id"] == "P01-output-json")
        .ok_or("Missing parse scenario")?
        .clone();
    for (index, value) in [Value::Null, json!(false), json!(0), json!("")]
        .into_iter()
        .enumerate()
    {
        let mut step = parse["steps"][0].clone();
        step["body"]["download"] = value;
        cases.push(json!({"id":format!("parse-falsy-download-{index}"),"source":parse["source"],"steps":[step]}));
    }
    cases.extend(contracts::cases("business-local")?);
    let mut port = 0;
    let mut observed = Vec::new();
    let mut states = Vec::new();
    let mut effects = Vec::new();
    for (kind, backend) in [
        ("node", Backend::Node(reference::node())),
        ("rust", target()?),
    ] {
        let label = format!("{kind}-conversion");
        let (state, mut environment) = seed(output, &label, &document, &root)?;
        let ca = state.join("native-ca.pem");
        std::fs::write(
            &ca,
            "-----BEGIN CERTIFICATE-----\nAAECAw==\n-----END CERTIFICATE-----",
        )?;
        for (key, file) in [
            ("SUB_STORE_MMDB_COUNTRY_PATH", "GeoIP2-Country-Test.mmdb"),
            ("SUB_STORE_MMDB_ASN_PATH", "GeoLite2-ASN-Test.mmdb"),
        ] {
            server::set(
                &mut environment,
                key,
                crate::compat().join("fixtures/mmdb").join(file),
            );
        }
        let upstream = Upstream::new(port)?;
        port = reqwest::Url::parse(upstream.url())?
            .port()
            .ok_or("Missing controlled upstream port")?;
        let mut required = cases.clone();
        let helper = required
            .iter_mut()
            .find(|case| case["id"] == "PC-native-conversion-helper-contracts")
            .ok_or("Missing conversion helper scenario")?;
        helper["steps"][0]["body"]["process"][0]["args"]["arguments"]["caFile"] =
            json!(if matches!(backend, Backend::Docker(_)) {
                "/data/native-ca.pem".into()
            } else {
                ca.to_string_lossy().into_owned()
            });
        let api = Api::start(&backend, &environment, &state, output, &label)?;
        let start = clock(false);
        let mut results = contracts::check(
            &api.http,
            &api.listener.url,
            &state,
            &required,
            Some(&upstream),
            &output.join(format!("{label}.json")),
        )?;
        let end = clock(true);
        let captured = &results
            .iter()
            .find(|case| case["id"] == "T04-token-expiration-inputs-and-order")
            .ok_or("Missing token clock scenario")?["steps"]
            .as_array()
            .ok_or("Missing token steps")?
            .iter()
            .find(|step| step["captured"].get("DURATION_EXP").is_some())
            .ok_or("Missing token captures")?["captured"];
        for key in ["DURATION_CREATED", "COUNT_CREATED", "DURATION_EXP"] {
            let value = captured[key].as_i64().ok_or("Invalid token clock")?;
            let delta = if key == "DURATION_EXP" { 86400000 } else { 0 };
            require(
                (start + delta..=end + delta).contains(&value),
                "Token timestamp outside actual request clock bounds",
            )?;
        }
        json::write_new(
            output.join(format!("{label}-clock.json")),
            &json!({"before":start,"after":end,"captured":captured}),
        )?;
        effects.push(json!({"concurrency":script_concurrency(&api)?,"background":background(&api,&upstream)?,"direct":direct(&api,kind=="rust")?}));
        states.push(json!({"sub-store.json":api.durable()?,"root.json":api.stored("root.json")?}));
        drop(api);
        drop(upstream);
        let before = std::fs::read(state.join("sub-store.json"))?;
        let label = format!("{label}-restart");
        let api = Api::start(&backend, &environment, &state, output, &label)?;
        results.extend(contracts::check(
            &api.http,
            &api.listener.url,
            &state,
            &contracts::cases("conversion-local-restart")?,
            None,
            &output.join(format!("{label}.json")),
        )?);
        require(
            std::fs::read(state.join("sub-store.json"))? == before,
            "Read-only conversion changed durable state",
        )?;
        observed.push(results);
    }
    require(
        observed[0].len() == observed[1].len(),
        "Conversion scenario count differs",
    )?;
    for (left, right) in observed[0].iter().zip(&observed[1]) {
        json::check(&left["id"], &right["id"], false)?;
        let left = left["steps"].as_array().ok_or("Missing steps")?;
        let right = right["steps"].as_array().ok_or("Missing steps")?;
        require(left.len() == right.len(), "Conversion step count differs")?;
        for (left, right) in left.iter().zip(right) {
            contracts::compare_step(left, right)?;
        }
    }
    json::check(&states[0], &states[1], false)?;
    json::check(&effects[0], &effects[1], false)
}
fn direct(api: &Api, contaminate: bool) -> Result<Value> {
    let before = [
        std::fs::read(api.state.join("sub-store.json"))?,
        std::fs::read(api.state.join("root.json"))?,
    ];
    let url = api.listener.url.clone();
    let convert = |http: &Http, index: usize, client: &str| -> Result<Value> {
        let response=http.request(&url,&json!({"method":"POST","path":"/api/proxy/parse","body":{"data":format!("http://user:pass@proxy.example:8080#reuse-{index}"),"client":client,"type":"internal","download":false}}))?;
        require(
            response["status"] == if client == "missing-target" { 500 } else { 200 },
            "Direct conversion response mismatch",
        )?;
        let body = json::parse(response["body"].as_str().ok_or("Missing conversion body")?)?;
        if client == "JSON" {
            require(
                body["data"]["par_res"][0]["name"] == format!("reuse-{index}"),
                "Concurrent conversion result leaked across requests",
            )?;
        }
        Ok(json!({"status":response["status"],"body":body}))
    };
    let mut observed = vec![convert(&api.http, 0, "JSON")?];
    if contaminate {
        let dirty=api.data(&json!({"method":"POST","path":"/api/preview/sub","body":{"name":"conversion-isolation","source":"local","content":"http://proxy.example:80#script","process":[{"type":"Script Operator","args":{"content":"function operator(proxies){Object.prototype.conversionPolluted=true;globalThis.conversionMarker=true;ProxyUtils.parse=()=>[];proxies[0].pollutionObserved=({}).conversionPolluted;return proxies;}"}}]}}))?;
        require(
            dirty["processed"][0]["pollutionObserved"] == true,
            "Isolation fixture did not contaminate script globals",
        )?;
    }
    for (index, client) in ["URI", "missing-target", "JSON", "Surge", "JSON"]
        .iter()
        .enumerate()
    {
        observed.push(convert(&api.http, index + 1, client)?);
    }
    let next = AtomicUsize::new(100);
    let concurrent = Mutex::new(Vec::new());
    std::thread::scope(|scope| -> Result {
        let mut workers = Vec::new();
        for _ in 0..8 {
            let convert = &convert;
            let next = &next;
            let concurrent = &concurrent;
            workers.push(scope.spawn(move || -> Result {
                let http = Http::new(Duration::from_secs(5))?;
                loop {
                    let index = next.fetch_add(1, Ordering::Relaxed);
                    if index >= 228 {
                        return Ok(());
                    }
                    concurrent
                        .lock()
                        .unwrap()
                        .push((index, convert(&http, index, "JSON")?));
                }
            }));
        }
        for worker in workers {
            worker
                .join()
                .map_err(|_| "Concurrent conversion worker panicked")??;
        }
        Ok(())
    })?;
    let mut concurrent = concurrent.into_inner().unwrap();
    concurrent.sort_by_key(|(index, _)| *index);
    observed.extend(concurrent.into_iter().map(|(_, result)| result));
    require(
        before
            == [
                std::fs::read(api.state.join("sub-store.json"))?,
                std::fs::read(api.state.join("root.json"))?,
            ],
        "Direct conversion changed durable state",
    )?;
    Ok(json!(observed))
}
fn script_step(name: &str, content: &str, arguments: Value) -> Value {
    json!({"method":"POST","path":"/api/preview/sub","body":{"name":name,"source":"local","content":format!("ss://YWVzLTEyOC1nY206cGFzcw==@127.0.0.1:8388#{name}"),"process":[{"type":"Script Operator","args":{"content":content,"arguments":arguments}}]}})
}
fn script_concurrency(api: &Api) -> Result<Value> {
    let step = script_step(
        "concurrent",
        r#"async function operator(proxies){$substore.write({started:true},'m3-script-await');const deadline=Date.now()+3000;while(!$substore.read('settings').m3Concurrent){if(Date.now()>deadline)throw new Error('Concurrent write was blocked');await $substore.wait(10);}$substore.write({completed:true},'m3-script-await');return proxies;}"#,
        json!({}),
    );
    let url = api.listener.url.clone();
    let (patch, response) = std::thread::scope(|scope| -> Result<(Value, Value)> {
        let pending = scope.spawn(move || Http::new(Duration::from_secs(5))?.request(&url, &step));
        let deadline = Instant::now() + Duration::from_secs(4);
        loop {
            let barrier = api.send(&json!({"path":"/api/storage"}), 200)?;
            if barrier["m3-script-await"]["started"] == true {
                break;
            }
            require(
                !pending.is_finished() && Instant::now() < deadline,
                "Script did not publish the await barrier",
            )?;
            std::thread::sleep(Duration::from_millis(10));
        }
        let patch = api.response(
            &json!({"method":"PATCH","path":"/api/settings","body":{"m3Concurrent":true}}),
        )?;
        let response = pending.join().map_err(|_| "Pending script panicked")??;
        Ok((patch, response))
    })?;
    require(
        patch["status"] == 200 && response["status"] == 200,
        "Concurrent script or ordinary write failed",
    )?;
    let durable = api.durable()?;
    require(
        durable["settings"]["m3Concurrent"] == true
            && durable["m3-script-await"] == json!({"completed":true}),
        "Pending script discarded the concurrent write",
    )?;
    Ok(
        json!({"patch":{"status":patch["status"],"body":patch["body"]},"script":{"status":response["status"],"body":response["body"]}}),
    )
}
fn background(api: &Api, upstream: &Upstream) -> Result<Value> {
    let script = r#"async function operator(proxies){const OriginalDate=Date;globalThis.Date=class extends OriginalDate{static __m3Original=OriginalDate;constructor(...args){super(...(args.length?args:[2300000000000]));}static now(){return 2300000000000;}};$substore.write('old-background','#sub-store-cached-custom-native-background');const value=await ProxyUtils.download($arguments.url,'background-native');if(value!=='old-background')throw new Error('optimistic result');proxies[0].optimistic=value;return proxies;}"#;
    let offset = upstream.fixture.count();
    let checked = (|| -> Result<Value> {
        let first=api.response(&script_step("background",script,json!({"url":format!("{}/host/download/text/background#cacheKey=native-background&cacheTtl=3",upstream.url())})))?;
        require(
            first["status"] == 200
                && json::parse(
                    first["body"]
                        .as_str()
                        .ok_or("Missing optimistic response")?,
                )?["data"]["processed"][0]["optimistic"]
                    == "old-background",
            "Optimistic download did not return old value",
        )?;
        require(
            upstream
                .state
                .background_started
                .wait(Duration::from_secs(2)),
            "Refresh did not start",
        )?;
        require(
            api.stored("root.json")?["sub-store-cached-custom-native-background"]
                == "old-background",
            "Reply waited for background body",
        )?;
        require(
            !upstream.state.background_release.wait(Duration::ZERO),
            "Refresh released before response",
        )?;
        upstream.state.background_release.set();
        let second=api.response(&script_step("background",r#"async function operator(proxies){try{for(let i=0;i<150;i++){const value=$substore.read('#sub-store-cached-custom-native-background');if(value==='下载🙂\r\n\0尾\n'){proxies[0].refreshed=value;return proxies;}await $substore.wait(10);}throw new Error('background refresh did not persist after the reply');}finally{if(Date.__m3Original)globalThis.Date=Date.__m3Original;}}"#,json!({})))?;
        require(
            second["status"] == 200
                && json::parse(
                    second["body"]
                        .as_str()
                        .ok_or("Missing refreshed response")?,
                )?["data"]["processed"][0]["refreshed"]
                    == "下载🙂\r\n\0尾\n",
            "Background result did not persist after reply",
        )?;
        let requests = upstream.fixture.since(offset);
        let wire=requests.iter().map(|item|json!({"method":item["method"],"path":item["path"],"body":item["body"],"user_agent":item["headers"].get("user-agent")})).collect::<Vec<_>>();
        json::check(
            &json!(wire),
            &json!([{"method":"GET","path":"/host/download/text/background","body":"","user_agent":"background-native"}]),
            false,
        )?;
        Ok(
            json!({"returned_before_release":true,"first":{"status":first["status"],"body":first["body"]},"second":{"status":second["status"],"body":second["body"]}}),
        )
    })();
    upstream.state.background_release.set();
    checked
}
