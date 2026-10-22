use crate::{
    Result,
    fixture::{Fixture, Reply, Request, unquote},
    json,
    providers::{self, ScriptGist},
};
use base64::Engine;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::io::Write;
use std::sync::{
    Arc, Condvar, Mutex,
    atomic::{AtomicUsize, Ordering},
};
use std::time::Duration;

#[derive(Default)]
pub struct Event {
    flag: Mutex<bool>,
    changed: Condvar,
}
impl Event {
    pub fn set(&self) {
        *self.flag.lock().unwrap() = true;
        self.changed.notify_all();
    }
    pub fn clear(&self) {
        *self.flag.lock().unwrap() = false;
    }
    pub fn wait(&self, timeout: Duration) -> bool {
        *self
            .changed
            .wait_timeout_while(self.flag.lock().unwrap(), timeout, |set| !*set)
            .unwrap()
            .0
    }
}

#[derive(Default)]
pub struct State {
    pub runtime_files: Mutex<BTreeMap<String, (u16, Vec<u8>)>>,
    pub gists: Mutex<Value>,
    pub script_gist: ScriptGist,
    pub background_started: Event,
    pub background_release: Event,
    flow_active: AtomicUsize,
    dns: Mutex<(usize, bool)>,
    dns_changed: Condvar,
    dns_release: Event,
}

pub struct Upstream {
    pub fixture: Fixture,
    pub state: Arc<State>,
}

#[cfg(test)]
mod proxy_tests {
    use super::*;
    #[test]
    fn node_info_proxy_replies_locally_and_rejects_other_authorities() -> Result {
        use std::{io::{Read,Write},net::TcpStream};let upstream=Upstream::new(0)?;
        for(target,status,expected)in[("http://ip-api.com/json/2001%3Adb8%3A%3A1?lang=zh-CN",200,json!({"status":"success","query":"2001:db8::1","countryCode":"US","city":"fixture-city","lat":0,"mobile":false})),("http://other.invalid/",502,json!({"message":"Fixture proxy forbids this authority"}))]{let url=reqwest::Url::parse(upstream.url())?;let mut socket=TcpStream::connect(("127.0.0.1",url.port().unwrap()))?;socket.set_read_timeout(Some(Duration::from_secs(3)))?;write!(socket,"GET {target} HTTP/1.1\r\nHost: fixture\r\nConnection: close\r\n\r\n")?;let mut raw=Vec::new();socket.read_to_end(&mut raw)?;let response=crate::raw_http::decode(&raw,false)?;assert_eq!(response.status,status);assert_eq!(json::parse(std::str::from_utf8(&response.body)?)?,expected);}
        assert_eq!(upstream.requests().iter().map(|row|row["path"].clone()).collect::<Vec<_>>(),vec![json!("http://ip-api.com/json/2001%3Adb8%3A%3A1?lang=zh-CN"),json!("http://other.invalid/")]);upstream.fixture.healthy()
    }
}
impl Upstream {
    pub fn new(port: u16) -> Result<Self> {
        let state = Arc::new(State {
            gists: Mutex::new(json!({})),
            ..State::default()
        });
        let shared = state.clone();
        let fixture = Fixture::new(port, move |request, origin| {
            shared.response(request, origin)
        })?;
        Ok(Self { fixture, state })
    }
    pub fn url(&self) -> &str {
        &self.fixture.origin
    }
    pub fn requests(&self) -> Vec<Value> {
        self.fixture.snapshot()
    }
    pub fn gists(&self) -> Value {
        self.state.gists.lock().unwrap().clone()
    }
}

pub fn substitute(value: &Value, origin: &str) -> Value {
    match value {
        Value::String(text) => Value::from(text.replace("$UPSTREAM", origin)),
        Value::Array(items) => Value::Array(
            items
                .iter()
                .map(|value| substitute(value, origin))
                .collect(),
        ),
        Value::Object(items) => Value::Object(
            items
                .iter()
                .map(|(key, value)| (key.clone(), substitute(value, origin)))
                .collect(),
        ),
        _ => value.clone(),
    }
}

impl State {
    fn response(&self, request: &Request, origin: &str) -> Result<Reply> {
        let url = request.url(origin)?;
        let path = url.path();
        if (request.target.starts_with("http://") || request.target.starts_with("https://"))
            && (url.scheme() != "http"
                || url.host_str() != Some("ip-api.com")
                || url.port_or_known_default() != Some(80)
                || request.method != "GET")
        {
            return Ok(Reply::json(
                502,
                &json!({"message":"Fixture proxy forbids this authority"}),
            )
            .header(
                "subscription-userinfo",
                "upload=10; download=20; total=1000",
            ));
        }
        if path.starts_with("/host/") {
            let reply = if path.starts_with("/host/gist/") {
                self.script_gist.response(request, origin)?
            } else if path.starts_with("/host/dns/") {
                self.dns_response(request, &url)?
            } else {
                self.host_response(request, path)?
            };
            return Ok(reply.header("X-Fixture", "host-contract"));
        }
        let reply = if path.starts_with("/json/") {
            let address = unquote(path.trim_start_matches("/json/"))?;
            let (status, value) = match address.as_str() {
                "rejected.invalid" => (
                    200,
                    json!({"status":"fail","message":"fixture lookup rejected"}),
                ),
                "http-error.invalid" => (
                    503,
                    json!({"status":"success","query":address,"countryCode":"ZZ"}),
                ),
                "1.2.3.4" | "2001:db8::1" => (
                    200,
                    json!({"status":"success","query":address,"countryCode":"US","city":"fixture-city","lat":0,"mobile":false}),
                ),
                _ => (
                    404,
                    json!({"status":"fail","message":"fixture address missing"}),
                ),
            };
            Reply::json(status, &value)
        } else if request.target.starts_with("/gists") || request.target.starts_with("/gist-raw/") {
            providers::patch_gists(&mut self.gists.lock().unwrap(), request, origin)?
        } else {
            Reply::new(
                if request.target == "/subscription" {
                    200
                } else {
                    404
                },
                "ss://YWVzLTEyOC1nY206cGFzcw==@127.0.0.1:8388#fixture",
            )
            .header("Content-Type", "text/plain; charset=utf-8")
        };
        Ok(reply.header(
            "subscription-userinfo",
            "upload=10; download=20; total=1000",
        ))
    }

    fn dns_response(&self, request: &Request, url: &reqwest::Url) -> Result<Reply> {
        let mode = url.path().trim_start_matches("/host/dns/");
        let encoded = url
            .query_pairs()
            .find(|(key, _)| key == "dns")
            .ok_or("Missing fixture DNS query")?
            .1
            .into_owned();
        let packet = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(encoded.trim_end_matches('='))?;
        if mode == "delay" {
            std::thread::sleep(Duration::from_millis(200));
        }
        if mode == "release" {
            self.dns_release.set();
        }
        if ["race-fast", "race-slow"].contains(&mode) {
            let mut state = self.dns.lock().unwrap();
            if state.0 == 0 {
                state.1 = false;
                self.dns_release.clear();
            }
            state.0 += 1;
            if state.0 == 2 {
                state.1 = true;
            }
            self.dns_changed.notify_all();
            let (state, _) = self
                .dns_changed
                .wait_timeout_while(state, Duration::from_secs(2), |state| !state.1)
                .unwrap();
            request.record.lock().unwrap()["dns_overlap"] = json!(state.1);
            drop(state);
            if mode == "race-slow" {
                self.dns_release.wait(Duration::from_secs(3));
            }
            self.dns.lock().unwrap().0 -= 1;
        }
        Ok(Reply::new(
            if mode == "status" { 503 } else { 200 },
            crate::dns::response_bytes(&packet, mode)?,
        )
        .header("Content-Type", "application/dns-message"))
    }

    fn host_response(&self, request: &Request, path: &str) -> Result<Reply> {
        if let Some(size) = path.strip_prefix("/host/header-size/") {
            return Ok(
                Reply::new(200, "header-fixture").header("X-Padding", "x".repeat(size.parse()?))
            );
        }
        if let Some((status, content)) = self.runtime_files.lock().unwrap().get(path) {
            return Ok(Reply::new(*status, content.clone())
                .header("Content-Type", "application/octet-stream"));
        }
        if path.starts_with("/host/notification") {
            return Ok(Reply::new(
                if path.starts_with("/host/notification-error") {
                    503
                } else {
                    200
                },
                r#"{"ok":true}"#,
            )
            .header("Content-Type", "application/json"));
        }
        if let Some(name) = path.strip_prefix("/host/pipeline/") {
            if name == "failed" {
                return Ok(Reply::new(503, "pipeline rejected"));
            }
            if name == "slow" {
                std::thread::sleep(Duration::from_millis(40));
            }
            return Ok(Reply::new(
                200,
                format!("ss://YWVzLTEyOC1nY206cGFzcw==@{name}.invalid:8388#remote-{name}"),
            ));
        }
        if let Some(mode) = path.strip_prefix("/host/artifact/") {
            if mode == "slow" {
                std::thread::sleep(Duration::from_millis(50));
            }
            let text = match mode {
                "sub" => "ss://YWVzLTEyOC1nY206c2VjcmV0@ss.example.com:8388#remote",
                "slow" => "ss://YWVzLTEyOC1nY206c2VjcmV0@ss.example.com:8388#slow",
                "clash" => {
                    "proxies:\n  - name: remote🙂\n    type: ss\n    server: ss.example.com\n    port: 8388\n    cipher: aes-128-gcm\n    password: secret\n"
                }
                "invalid" => "not a subscription",
                "file" => "远程文件🙂\r\nline\n",
                "script" => {
                    r#"function operator(proxies) { return proxies.map(p => ({...p, name: p.name + "-" + $arguments.suffix})); }"#
                }
                "bad" => "artifact upstream rejected",
                _ => return Err("Unknown artifact fixture".into()),
            };
            return Ok(Reply::new(if mode == "bad" { 503 } else { 200 }, text)
                .header("Content-Type", "text/plain; charset=utf-8"));
        }
        if let Some(mode) = path.strip_prefix("/host/script/") {
            if mode == "missing" {
                return Ok(Reply::new(404, "no script"));
            }
            let script = if mode == "response" {
                "function transformFunction(res) { res.body += $arguments.text; return res; }"
            } else {
                "async function operator(proxies, target, context) {\n  await $substore.wait(1);\n  for(const proxy of proxies) proxy.linked={arguments:$arguments,target,options:$options??null,raw:context.raw??null,source:context.source?.marker??null};\n  return proxies;\n}"
            };
            return Ok(Reply::new(200, script).header(
                "Subscription-Userinfo",
                if mode == "flow" {
                    "upload=3; download=0; total=3"
                } else {
                    "upload=1; download=2; total=100"
                },
            ));
        }
        if let Some(mode) = path.strip_prefix("/host/download/") {
            if let Some(kind) = mode.strip_prefix("age/") {
                let vectors = json::read(crate::compat().join("fixtures/age-native-vectors.json"))?;
                let pairs = vectors.as_array().ok_or("Invalid age vectors")?;
                let pair = pairs
                    .iter()
                    .find(|item| item["type"] == kind)
                    .unwrap_or(&pairs[0]);
                return Ok(Reply::new(
                    200,
                    pair["items"][1]["armor"]
                        .as_str()
                        .ok_or("Missing age armor")?,
                ));
            }
            if mode == "text/background" {
                self.background_started.set();
                self.background_release.wait(Duration::from_secs(5));
            }
            if let Some(status) = mode.strip_prefix("status/") {
                return Ok(Reply::new(status.parse()?, "provider rejected"));
            }
            match mode {
                "text/white" => return Ok(Reply::new(200, " \t\r\n")),
                "text/slow" => std::thread::sleep(Duration::from_millis(200)),
                "text/preprocess-good" => {
                    return Ok(Reply::new(
                        200,
                        "proxies:\n  - {name: 预处理, type: ss, server: example.invalid, port: 8388, cipher: aes-128-gcm, password: pass}\n",
                    ));
                }
                "text/preprocess-invalid" => return Ok(Reply::new(200, "<!DOCTYPE html>no nodes")),
                _ => (),
            }
            return Ok(Reply::new(200, "下载🙂\r\n\0尾\n").header(
                "Subscription-Userinfo",
                if mode == "text/flow-exhausted" {
                    "upload=3; download=0; total=3"
                } else {
                    "upload=1; download=2; total=100; expire=4102444800"
                },
            ));
        }
        let text = "订阅🙂\r\nsecond\n";
        if let Some(mode) = path.strip_prefix("/host/compressed/") {
            if mode == "too-many" {
                return Ok(
                    Reply::new(200, Vec::new()).header("Content-Encoding", ["gzip"; 6].join(", "))
                );
            }
            let mut gzip = flate2::GzBuilder::new()
                .mtime(0)
                .write(Vec::new(), flate2::Compression::default());
            gzip.write_all(text.as_bytes())?;
            let mut encoded = gzip.finish()?;
            if mode == "unknown" {
                return Ok(Reply::new(200, encoded).header("Content-Encoding", "gzip, unknown"));
            }
            if ["deflate", "compress", "x-compress", "nested"].contains(&mode) {
                let mut encoder =
                    flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
                encoder.write_all(if mode == "nested" {
                    &encoded
                } else {
                    text.as_bytes()
                })?;
                encoded = encoder.finish()?;
            }
            return Ok(
                Reply::new(if mode == "status" { 503 } else { 200 }, encoded).header(
                    "Content-Encoding",
                    if mode == "nested" {
                        "gzip, deflate"
                    } else if mode == "status" {
                        "gzip"
                    } else {
                        mode
                    },
                ),
            );
        }
        if let Some(mode) = path.strip_prefix("/host/flow/") {
            let flow = |reply: Reply| {
                reply
                    .header(
                        "Subscription-Userinfo",
                        "upload=10; download=20; total=1000",
                    )
                    .header("Profile-Web-Page-Url", "https://example.invalid/flow")
                    .header("Plan-Name", "Flow plan")
            };
            if let Some(name) = mode.strip_prefix("queued/") {
                let active = self.flow_active.fetch_add(1, Ordering::SeqCst) + 1;
                std::thread::sleep(Duration::from_millis(30));
                self.flow_active.fetch_sub(1, Ordering::SeqCst);
                return Ok(Reply::new(200, Vec::new())
                    .header("Subscription-Userinfo", "download=0; total=1")
                    .header("Plan-Name", format!("{name}-active-{active}")));
            }
            return Ok(match mode {
                "delay" => {
                    std::thread::sleep(Duration::from_millis(200));
                    Reply::new(200, "download=1; total=2")
                }
                "head" | "get" | "fallback" | "badstatus" => {
                    let reply = Reply::new(
                        if mode == "badstatus" { 503 } else { 200 },
                        "invalid flow body",
                    );
                    if mode == "get" && request.method == "HEAD" {
                        reply
                    } else {
                        flow(reply)
                    }
                }
                "body" => Reply::new(200, " upload=30; download=40; total=2000 "),
                "rejected" | "empty" => {
                    Reply::new(if mode == "rejected" { 503 } else { 200 }, "no flow")
                }
                _ => Reply::new(404, "unexpected flow fixture"),
            });
        }
        Ok(match path {
            "/host/delay" => {
                std::thread::sleep(Duration::from_millis(200));
                Reply::new(200, text)
            }
            "/host/text" => {
                Reply::new(200, text).header("Content-Type", "text/plain; charset=utf-8")
            }
            "/host/utf8" => {
                let mut body = vec![0xef, 0xbb, 0xbf, 0, 0xff, 0xc3, 0x28];
                body.extend("订阅🙂\r\n".as_bytes());
                Reply::new(200, body).header("Content-Type", "text/plain; charset=utf-8")
            }
            "/host/echo" => Reply::json(
                200,
                &json!({"method":request.method,"body":request.body,"accept":request.header("Accept"),"probe":request.header("X-Probe"),"authorization":request.header("Authorization")}),
            ),
            "/host/binary" => Reply::new(200, vec![0, 255, 195, 40, 13, 10])
                .header("Set-Cookie", "first=1")
                .header("Set-Cookie", "second=2"),
            "/host/status" => Reply::new(503, "provider rejected"),
            path if path.starts_with("/host/redirect/") => {
                let remaining = path.rsplit('/').next().unwrap().parse::<u32>()?;
                Reply::new(302, Vec::new()).header(
                    "Location",
                    if remaining > 1 {
                        format!("/host/redirect/{}", remaining - 1)
                    } else {
                        "/host/text".to_owned()
                    },
                )
            }
            _ => Reply::new(404, "unexpected host request"),
        })
    }
}
