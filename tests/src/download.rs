use crate::{
    Result,
    fixture::{Fixture, Reply},
    json, probe, process, reference, require, server,
};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::io::Write;
use std::net::Shutdown;
use std::path::Path;
use std::process::Command;
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

const TEXT: &str = "下载🙂\r\n\0尾\n";
const SMALL: &[u8] = &[0, 255, 128, 13, 10, 65];
fn pattern() -> Vec<u8> {
    (0..=255)
        .cycle()
        .take(256 * 1024)
        .chain([0, 1, 2])
        .collect()
}
fn expected_files() -> BTreeMap<&'static str, Vec<u8>> {
    BTreeMap::from([
        ("local🙂.txt", TEXT.as_bytes().to_vec()),
        ("empty.txt", vec![]),
        ("invalid-utf8.txt", b"a\xf0(\x8c(z".to_vec()),
        ("script🙂.bin", SMALL.to_vec()),
        ("binary🙂.bin", SMALL.to_vec()),
        ("complete.bin", pattern()),
        ("broken.bin", pattern()[..16384].to_vec()),
        ("zero.bin", vec![]),
        ("recovery.bin", SMALL.to_vec()),
    ])
}
#[derive(Default)]
struct State {
    release: Mutex<[bool; 2]>,
    ready: Condvar,
    events: Mutex<Vec<Value>>,
    errors: Mutex<Vec<String>>,
}
struct Download {
    fixture: Fixture,
    state: Arc<State>,
}
impl Download {
    fn new(keys: Value) -> Result<Self> {
        let state = Arc::new(State::default());
        let owner = state.clone();
        let fixture = Fixture::raw(0, move |request, origin, stream| {
            *request.record.lock().unwrap() = json!({"method":request.method,"path":request.target,"headers":request.headers,"body_bytes":request.body});
            let url = request.url(origin)?;
            let path = url.path();
            if let Some(mode) = path.strip_prefix("/stream/") {
                let index = match mode {
                    "complete" => 0,
                    "broken" => 1,
                    _ => return Err("Unknown stream mode".into()),
                };
                let bytes = pattern();
                write!(
                    stream,
                    "HTTP/1.0 200 OK\r\nContent-Length: {}\r\n\r\n",
                    bytes.len()
                )?;
                stream.write_all(&bytes[..16384])?;
                stream.flush()?;
                owner
                    .events
                    .lock()
                    .unwrap()
                    .push(json!({"mode":mode,"event":"prefix-sent"}));
                let (released, timeout) = owner
                    .ready
                    .wait_timeout_while(
                        owner.release.lock().unwrap(),
                        Duration::from_secs(10),
                        |released| !released[index],
                    )
                    .unwrap();
                if timeout.timed_out() && !released[index] {
                    owner
                        .errors
                        .lock()
                        .unwrap()
                        .push(format!("stream release timed out: {mode}"));
                } else if index == 0 {
                    stream.write_all(&bytes[16384..])?;
                    stream.flush()?;
                }
                owner
                    .events
                    .lock()
                    .unwrap()
                    .push(json!({"mode":mode,"event":"closed"}));
                stream.shutdown(Shutdown::Write)?;
                return Ok(());
            }
            let mut reply = Reply::new(200, TEXT);
            if let Some(mode) = path.strip_prefix("/release/") {
                let index = match mode {
                    "complete" => 0,
                    "broken" => 1,
                    _ => return Err("Unknown release mode".into()),
                };
                owner
                    .events
                    .lock()
                    .unwrap()
                    .push(json!({"mode":mode,"event":"released"}));
                owner.release.lock().unwrap()[index] = true;
                owner.ready.notify_all();
                reply.body = b"ok".to_vec();
            } else if let Some(status) = path.strip_prefix("/status/") {
                reply = Reply::new(status.parse()?, "status fixture");
            } else if let Some(count) = path.strip_prefix("/redirect/") {
                let count = count.parse::<u32>()?;
                reply = if count > 0 {
                    Reply::new(302, vec![]).header("Location", format!("/redirect/{}", count - 1))
                } else {
                    Reply::new(200, SMALL)
                };
            } else if path == "/loop" {
                reply = Reply::new(302, vec![]).header("Location", "/loop");
            } else if path == "/file/small" {
                reply.body = SMALL.to_vec();
            } else if path == "/file/empty" {
                reply.body.clear();
            } else if let Some(kind) = path.strip_prefix("/age/") {
                reply.body = keys
                    .as_array()
                    .ok_or("Invalid age download keys")?
                    .iter()
                    .find(|key| key["type"] == kind)
                    .ok_or("Missing age download ciphertext")?["armor"]
                    .as_str()
                    .ok_or("Missing armor")?
                    .as_bytes()
                    .to_vec();
            } else if path == "/text/white" {
                reply.body = b" \t\r\n ".to_vec();
            } else if path == "/text/slow" {
                std::thread::sleep(Duration::from_millis(120));
            } else if [
                "/text/flow",
                "/text/flow-options",
                "/text/flow-valid",
                "/text/flow-exhausted",
            ]
            .contains(&path)
            {
                reply = reply.header(
                    "subscription-userinfo",
                    format!(
                        "upload=1; download=2; total={}; expire=4102444800",
                        if path.ends_with("exhausted") { 2 } else { 100 }
                    ),
                );
            } else if !path.starts_with("/text/") && path != format!("/proxy/{origin}/text/proxied")
            {
                reply = Reply::new(404, "unknown download fixture");
            }
            let _ = reply.write(stream, false);
            Ok(())
        })?;
        Ok(Self { fixture, state })
    }
    fn reset(&self) {
        *self.state.release.lock().unwrap() = [false; 2];
        self.state.events.lock().unwrap().clear();
        self.state.errors.lock().unwrap().clear();
    }
}
impl Drop for Download {
    fn drop(&mut self) {
        *self.state.release.lock().unwrap() = [true; 2];
        self.state.ready.notify_all();
    }
}

pub fn expected_wire(base: &str) -> Vec<Value> {
    let default = json!({"accept":"*/*","user-agent":"clash.meta/v1.19.23"});
    let mut rows = Vec::new();
    let mut add = |path: String, headers: Value, method: &str| {
        rows.push(json!({"method":method,"path":path,"headers":headers,"body_bytes":[]}))
    };
    add("/text/default".into(), default.clone(), "GET");
    let mut setting = default.clone();
    setting["user-agent"] = json!("download-default");
    add("/text/settings".into(), setting.clone(), "GET");
    let mut custom = default.clone();
    custom["user-agent"] = json!("download-custom");
    custom["x-download-token"] = json!("token-2");
    for path in ["/text/headers", "/text/pairs"] {
        add(path.into(), custom.clone(), "GET");
    }
    add("/text/invalid-headers".into(), setting, "GET");
    add(
        format!("/proxy/{base}/text/proxied"),
        default.clone(),
        "GET",
    );
    add("/text/invalid-regex".into(), default.clone(), "GET");
    for path in [
        "/text/ttl",
        "/text/ttl",
        "/text/ttl",
        "/text/ttl",
        "/text/threshold",
        "/text/threshold",
        "/text/optimistic",
        "/status/503",
        "/text/flow",
        "/text/flow-options",
        "/text/flow-valid",
        "/text/flow-exhausted",
        "/status/404",
        "/text/white",
        "/text/slow",
        "/text/recovered",
        "/age/x25519",
        "/status/500",
        "/age/mlkem768-x25519",
        "/status/500",
        "/text/reload",
    ] {
        add(path.into(), default.clone(), "GET");
    }
    for path in [
        "/file/small",
        "/file/small",
        "/redirect/2",
        "/redirect/1",
        "/redirect/0",
        "/status/201",
        "/status/404",
        "/redirect/3",
        "/redirect/2",
        "/redirect/1",
        "/redirect/0",
        "/loop",
    ] {
        add(path.into(), json!({}), "GET");
    }
    for mode in ["complete", "broken"] {
        add(format!("/stream/{mode}"), json!({}), "GET");
        add(format!("/release/{mode}"), json!({"accept":"*/*"}), "POST");
    }
    for path in ["/file/small", "/file/small", "/file/empty", "/file/small"] {
        add(path.into(), json!({}), "GET");
    }
    rows
}
pub fn check_wire(requests: &Value, base: &str) -> Result {
    let mut rows = requests.as_array().ok_or("Missing download wire")?.clone();
    for item in &mut rows {
        let headers = item["headers"]
            .as_object_mut()
            .ok_or("Missing download headers")?;
        require(
            headers.get("host").and_then(Value::as_str) == base.strip_prefix("http://"),
            "Download Host mismatch",
        )?;
        if let Some(length) = headers.get("content-length") {
            json::check(length, &json!("0"), false)?;
        }
        if let Some(connection) = headers.get("connection") {
            require(
                connection.as_str().is_some_and(|value| {
                    ["close", "keep-alive"].contains(&value.to_ascii_lowercase().as_str())
                }),
                "Unexpected Connection header",
            )?;
        }
        for key in ["host", "connection", "content-length"] {
            headers.shift_remove(key);
        }
    }
    json::check(&json!(rows), &json!(expected_wire(base)), false)
}
pub fn check_run(result: &Value, progress: &Value, resume: bool) -> Result {
    let ids = if resume {
        vec![]
    } else {
        probe::identifiers("download")?
    };
    if resume {
        let mut keys = result
            .as_object()
            .ok_or("Missing resumed download result")?
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>();
        keys.sort();
        require(
            keys == ["headersCache", "resourceCache", "storage"],
            "Unexpected resumed download fields",
        )?;
    } else {
        json::check(&result["passed"], &json!(ids), false)?;
        json::check(
            &json!(
                result["observations"]
                    .as_object()
                    .ok_or("Missing download observations")?
                    .keys()
                    .collect::<Vec<_>>()
            ),
            &json!(ids),
            false,
        )?;
    }
    json::check(
        progress,
        &json!(
            ids.iter()
                .flat_map(|id| [
                    json!({"id":id,"event":"start"}),
                    json!({"id":id,"event":"end"})
                ])
                .collect::<Vec<_>>()
        ),
        false,
    )
}
pub fn snapshot(state: &Path) -> Result<Value> {
    let expected = expected_files();
    let mut required = expected
        .keys()
        .map(|key| (*key).to_owned())
        .chain(
            [
                "sub-store.json",
                "sub-store-probe.json",
                "root.json",
                "download-input.json",
            ]
            .map(String::from),
        )
        .collect::<Vec<_>>();
    required.sort();
    let mut names = std::fs::read_dir(state)?
        .map(|entry| {
            Ok(entry?
                .file_name()
                .to_str()
                .ok_or("Invalid download filename")?
                .to_owned())
        })
        .collect::<Result<Vec<_>>>()?;
    names.sort();
    require(names == required, "Unexpected download state files")?;
    let mut files = serde_json::Map::new();
    for (name, expected) in expected {
        let actual = std::fs::read(state.join(name))?;
        require(
            actual == expected,
            format!("Downloaded file bytes differ: {name}"),
        )?;
        files.insert(
            name.into(),
            json!({"length":actual.len(),"sha256":process::sha256(&actual)}),
        );
    }
    Ok(json!(files))
}
pub fn check_state(result: &Value, persisted: &Value) -> Result {
    json::check(&result["storage"], persisted, false)?;
    for (field, key) in [
        ("resourceCache", "sub-store-cached-resource"),
        ("headersCache", "sub-store-cached-headers-resource"),
    ] {
        json::check(
            &result[field],
            &json::parse(
                persisted["root"][key]
                    .as_str()
                    .ok_or("Missing download cache")?,
            )?,
            false,
        )?;
    }
    json::check(
        &persisted["cache"]["downloadHelper"],
        &json!({"text":TEXT,"zero":0,"flag":false}),
        false,
    )
}
fn execute(
    kind: &str,
    binary: &crate::script_target::Target,
    state: &Path,
    url: &str,
    args: &[&str],
    output: &Path,
    label: &str,
) -> Result<process::Completed> {
    let mut environment = server::environment(state);
    server::set(&mut environment, "SUB_STORE_COMPAT_STATE_DIR", state);
    server::set(&mut environment, "SUB_STORE_COMPAT_UPSTREAM", url);
    let mut command = if kind == "node" {
        let mut command = Command::new(reference::node());
        command
            .arg("--require")
            .arg(crate::compat().join("register.cjs"))
            .arg(crate::compat().join("download-reference.cjs"));
        command
    } else {
        binary.command(&environment)?
    };
    command
        .args(args)
        .env_clear()
        .envs(environment)
        .current_dir(if kind == "node" {
            crate::root().join("backend")
        } else {
            crate::root()
        });
    if kind == "node" {
        process::run_direct(
            &mut command,
            output,
            label,
            Duration::from_secs(60),
            "Direct download process ownership allows watchdog cleanup on Windows",
        )
    } else {
        binary.run(&mut command, output, label)
    }
}
pub fn run(output: &Path, binary: &crate::script_target::Target) -> Result {
    let prepare = output.join("download-prepare-state");
    std::fs::create_dir(&prepare)?;
    require(
        execute(
            "node",
            binary,
            &prepare,
            "http://127.0.0.1:1",
            &["--prepare"],
            output,
            "download-prepare",
        )?
        .success(),
        "Download age preparation failed",
    )?;
    let input = std::fs::read(prepare.join("download-input.json"))?;
    let keys = json::parse(std::str::from_utf8(&input)?)?["keys"].clone();
    let upstream = Download::new(keys.clone())?;
    let url = &upstream.fixture.origin;
    let expected_events = json!(
        ["complete", "broken"]
            .iter()
            .flat_map(|mode| ["prefix-sent", "released", "closed"]
                .map(|event| json!({"mode":mode,"event":event})))
            .collect::<Vec<_>>()
    );
    let mut runs = Vec::new();
    for kind in ["node", "rust"] {
        upstream.reset();
        let state = output.join(format!("{kind}-download-state"));
        std::fs::create_dir(&state)?;
        for file in ["sub-store.json", "sub-store-probe.json", "root.json"] {
            json::write_new(state.join(file), &json!({}))?;
        }
        std::fs::write(state.join("download-input.json"), &input)?;
        for (name, bytes) in expected_files()
            .into_iter()
            .filter(|(name, _)| ["local🙂.txt", "empty.txt", "invalid-utf8.txt"].contains(name))
        {
            std::fs::write(state.join(name), bytes)?;
        }
        for resume in [false, true] {
            let label = format!("{kind}-download{}", if resume { "-resume" } else { "" });
            let offset = upstream.fixture.count();
            let args = if kind == "node" {
                if resume { vec!["--resume"] } else { vec![] }
            } else {
                vec![if resume {
                    "--download-resume"
                } else {
                    "--download"
                }]
            };
            let result = execute(kind, binary, &state, url, &args, output, &label)?;
            require(
                result.success(),
                format!("{label} failed: {}", result.stderr),
            )?;
            let progress = result
                .stderr
                .lines()
                .filter_map(|line| line.strip_prefix("COMPAT_DOWNLOAD "))
                .map(json::parse)
                .collect::<Result<Vec<_>>>()?;
            let value = json::parse(if kind == "node" {
                let lines = result
                    .stdout
                    .lines()
                    .filter_map(|line| line.strip_prefix("COMPAT_DOWNLOAD_RESULT "))
                    .collect::<Vec<_>>();
                require(lines.len() == 1, "Missing or duplicate download result")?;
                lines[0]
            } else {
                result
                    .stdout
                    .lines()
                    .last()
                    .ok_or("Missing download result")?
            })?;
            let requests = json!(upstream.fixture.since(offset));
            json::write_new(output.join(format!("{label}-wire.json")), &requests)?;
            check_run(&value, &json!(progress), resume)?;
            if resume {
                json::check(&requests, &json!([]), false)?;
                let previous: &Value = runs.last().ok_or("Missing initial download result")?;
                json::check(
                    &value,
                    &json!({"storage":previous["result"]["storage"],"resourceCache":previous["result"]["resourceCache"],"headersCache":previous["result"]["headersCache"]}),
                    false,
                )?;
            } else {
                check_wire(&requests, url)?;
            }
            let persisted = json!({"cache":json::read(state.join("sub-store.json"))?,"root":json::read(state.join("root.json"))?});
            check_state(&value, &persisted)?;
            let files = snapshot(&state)?;
            let events = json!(upstream.state.events.lock().unwrap().clone());
            json::check(&events, &expected_events, false)?;
            require(
                upstream.state.errors.lock().unwrap().is_empty(),
                "Download streaming fixture failed",
            )?;
            require(
                std::fs::read(state.join("download-input.json"))? == input,
                "Age input changed between download processes",
            )?;
            for key in keys.as_array().ok_or("Missing download age keys")? {
                let secret = key["age-secret-key"].as_str().ok_or("Missing age secret")?;
                require(
                    !result.stdout.contains(secret) && !result.stderr.contains(secret),
                    "Fixture age secret leaked into download logs",
                )?;
            }
            let record = json!({"name":label,"exit_code":result.status.code(),"result":value,"progress":progress,"requests":requests,"files":files,"stream_events":events,"input_sha256":process::sha256(&input)});
            json::write_new(output.join(format!("{label}.json")), &record)?;
            runs.push(record);
        }
    }
    for (index, run) in runs.iter().enumerate() {
        let label = run["name"].as_str().ok_or("Missing download stage")?;
        let command = json::read(output.join(format!("{label}.command.json")))?;
        json::check(
            &json!([command["exit_code"], command["timed_out"], run["exit_code"]]),
            &json!([0, false, 0]),
            false,
        )?;
        let stdout = std::fs::read_to_string(output.join(format!("{label}.stdout.log")))?;
        let stderr = std::fs::read_to_string(output.join(format!("{label}.stderr.log")))?;
        let progress = stderr
            .lines()
            .filter_map(|line| line.strip_prefix("COMPAT_DOWNLOAD "))
            .map(json::parse)
            .collect::<Result<Vec<_>>>()?;
        json::check(&run["progress"], &json!(progress), false)?;
        let value = json::parse(if index < 2 {
            stdout
                .lines()
                .find_map(|line| line.strip_prefix("COMPAT_DOWNLOAD_RESULT "))
                .ok_or("Missing raw download result")?
        } else {
            stdout.lines().last().ok_or("Missing raw download result")?
        })?;
        json::check(&value, &run["result"], false)?;
        check_run(&value, &json!(progress), index % 2 == 1)?;
        let wire = json::read(output.join(format!("{label}-wire.json")))?;
        json::check(&wire, &run["requests"], false)?;
        if index % 2 == 0 {
            check_wire(&wire, url)?;
        } else {
            json::check(&wire, &json!([]), false)?;
        }
    }
    json::check(&runs[0]["result"], &runs[2]["result"], false)?;
    json::check(&runs[1]["result"], &runs[3]["result"], false)?;
    let preparation = json::read(output.join("download-prepare.command.json"))?;
    json::check(
        &json!([preparation["exit_code"], preparation["timed_out"]]),
        &json!([0, false]),
        false,
    )?;
    upstream.fixture.healthy()?;
    json::write_new(
        output.join("download.json"),
        &json!({"runs":runs,"upstream":url,"http_requests_per_runtime":expected_wire(url).len(),"files_per_runtime":9,"stream_prefix_bytes":16384,"complete_bytes":pattern().len(),"partial_error_bytes":16384}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn download_wire_trace_files_and_caches_reject_mutations() {
        let root = process::scratch("download-self-check").unwrap();
        for (name, bytes) in expected_files() {
            std::fs::write(root.join(name), bytes).unwrap();
        }
        for file in [
            "sub-store.json",
            "sub-store-probe.json",
            "root.json",
            "download-input.json",
        ] {
            json::write_new(root.join(file), &json!({})).unwrap();
        }
        snapshot(&root).unwrap();
        std::fs::write(root.join("binary🙂.bin"), b"bad").unwrap();
        assert!(snapshot(&root).is_err());
        std::fs::write(root.join("binary🙂.bin"), SMALL).unwrap();
        std::fs::remove_file(root.join("binary🙂.bin")).unwrap();
        assert!(snapshot(&root).is_err());
        std::fs::write(root.join("binary🙂.bin"), SMALL).unwrap();
        std::fs::write(root.join("extra"), b"extra").unwrap();
        assert!(snapshot(&root).is_err());
        let base = "http://127.0.0.1:1234";
        let mut wire = expected_wire(base);
        for row in &mut wire {
            row["headers"]["host"] = json!("127.0.0.1:1234");
        }
        check_wire(&json!(wire), base).unwrap();
        for mutation in ["host", "header", "body", "order", "missing", "extra"] {
            let mut changed = wire.clone();
            match mutation {
                "host" => changed[0]["headers"]["host"] = json!("wrong"),
                "header" => changed[0]["headers"]["accept"] = json!("wrong"),
                "body" => changed[0]["body_bytes"] = json!([1]),
                "order" => changed.swap(0, 1),
                "missing" => {
                    changed.pop();
                }
                _ => changed.push(changed[0].clone()),
            }
            assert!(check_wire(&json!(changed), base).is_err(), "{mutation}");
        }
        let persisted = json!({"cache":{"downloadHelper":{"text":TEXT,"zero":0,"flag":false}},"root":{"sub-store-cached-resource":"{\"a\":false}","sub-store-cached-headers-resource":"{\"b\":0}"}});
        let value = json!({"storage":persisted,"resourceCache":{"a":false},"headersCache":{"b":0}});
        check_state(&value, &persisted).unwrap();
        let mut changed = value.clone();
        changed["resourceCache"]["a"] = json!(0);
        assert!(check_state(&changed, &persisted).is_err());
        let mut changed = persisted.clone();
        changed["cache"]["downloadHelper"]["flag"] = json!(0);
        assert!(check_state(&value, &changed).is_err());
    }
}
