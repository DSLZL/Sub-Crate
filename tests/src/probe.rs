use crate::{
    Result, dns, json, mmdb, process, providers, reference, require, server, upstream::Upstream,
};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

pub fn identifiers(suite: &str) -> Result<Vec<Value>> {
    let inventory = json::read(crate::compat().join("fixtures/script-cases.json"))?;
    let required = inventory[suite]
        .as_array()
        .ok_or("Missing required script inventory")?;
    let mut names = std::collections::HashSet::new();
    require(
        !required.is_empty()
            && required.iter().all(|id| {
                id.as_str()
                    .is_some_and(|id| !id.is_empty() && names.insert(id))
            }),
        "Empty, duplicate or invalid required script identifiers",
    )?;
    Ok(required.clone())
}

pub fn check_progress(events: &[Value], resume: bool) -> Result {
    let expected = if resume {
        Vec::new()
    } else {
        identifiers("processor")?
            .iter()
            .flat_map(|id| {
                [
                    json!({"id":id,"event":"start"}),
                    json!({"id":id,"event":"end"}),
                ]
            })
            .collect()
    };
    json::check(&json!(events), &json!(expected), false)
}

pub fn check_host_requests(requests: &[Value]) -> Result {
    let mut expected = Vec::new();
    let mut add = |method: &str,
                   path: &str,
                   body: Vec<u8>,
                   accept: Option<&str>,
                   probe: Option<&str>,
                   authorization: Option<&str>| {
        expected.push(json!({"method":method,"path":path,"body_bytes":body,"headers":{"accept":accept,"x-probe":probe,"authorization":authorization}}));
    };
    for method in ["GET", "POST", "PUT", "DELETE", "HEAD", "OPTIONS", "PATCH"] {
        add(
            method,
            "/host/echo",
            if ["GET", "HEAD"].contains(&method) {
                vec![]
            } else {
                "订阅🙂\r\nsecond\n".as_bytes().to_vec()
            },
            Some("*/*"),
            Some(method),
            None,
        );
    }
    for path in ["/host/binary", "/host/text", "/host/utf8", "/host/utf8"] {
        add("GET", path, vec![], Some("*/*"), None, None);
    }
    for _ in 0..2 {
        add("POST", "/host/echo", vec![0, 255, 13, 10], None, None, None);
    }
    add(
        "GET",
        "/host/echo",
        vec![],
        Some("application/json"),
        None,
        Some("Basic dXNlckBuYW1lOnBAc3M6JTIw"),
    );
    add("GET", "/host/echo", vec![], Some("*/*"), Some("hook"), None);
    for path in [
        "/host/echo",
        "/host/delay",
        "/host/echo",
        "/host/status",
        "/host/redirect/2",
        "/host/redirect/1",
        "/host/text",
        "/host/redirect/3",
        "/host/redirect/2",
        "/host/redirect/1",
        "/host/text",
        "/host/redirect/4",
        "/host/redirect/3",
        "/host/redirect/2",
        "/host/redirect/1",
    ] {
        add("GET", path, vec![], Some("*/*"), None, None);
    }
    let actual=requests.iter().map(|item| json!({"method":item["method"],"path":item["path"],"body_bytes":item["body_bytes"],"headers":{"accept":item["headers"].get("accept"),"x-probe":item["headers"].get("x-probe"),"authorization":item["headers"].get("authorization")}})).collect::<Vec<_>>();
    json::check(&json!(actual), &json!(expected), false)
}

pub fn check_artifact_requests(requests: &[Value]) -> Result {
    let default = "clash.meta/v1.19.23";
    let paths = [
        ("script", default),
        ("clash", default),
        ("invalid", default),
        ("invalid", default),
        ("sub", "Artifact Override/1"),
        ("sub", "Artifact Override/1"),
        ("slow", "Artifact Pass/1"),
        ("sub", "Artifact Fixed/1"),
        ("file", default),
        ("bad", default),
        ("slow", default),
        ("sub", "Artifact Fixed/1"),
        ("file", default),
        ("sub", default),
    ];
    let mut expected=paths.into_iter().map(|(path,ua)| json!({"method":"GET","path":format!("/host/artifact/{path}"),"body_bytes":[],"headers":{"user-agent":ua,"accept":"*/*"}})).collect::<Vec<_>>();
    let mut actual=requests.iter().map(|item| json!({"method":item["method"],"path":item["path"],"body_bytes":item["body_bytes"],"headers":item["headers"].as_object().unwrap().iter().filter(|(key,_)| !["host","connection","accept-encoding","accept-language","sec-fetch-mode"].contains(&key.as_str())).map(|(key,value)| (key.clone(),value.clone())).collect::<serde_json::Map<_,_>>()})).collect::<Vec<_>>();
    require(
        actual.len() == expected.len(),
        "Artifact wire request count mismatch",
    )?;
    for rows in [&mut actual, &mut expected] {
        for start in [6, 8, 10] {
            rows[start..start + 2]
                .sort_by(|left, right| left["path"].as_str().cmp(&right["path"].as_str()));
        }
    }
    json::check(&json!(actual), &json!(expected), false)
}

pub fn check_artifact_state(result: &Value, persisted: &Value) -> Result {
    json::check(&result["storage"], persisted, false)?;
    json::check(
        &persisted["cache"]["artifactHelper"],
        &json!({"text":"远程文件🙂\r\nline\n|remote","zero":0,"flag":false}),
        false,
    )?;
    for (field, key) in [
        ("resourceCache", "sub-store-cached-script-resource"),
        ("headersCache", "sub-store-cached-headers-resource"),
        ("dnsCache", "sub-store-cached-resource"),
    ] {
        json::check(
            &result[field],
            &json::parse(
                persisted["root"][key]
                    .as_str()
                    .ok_or("Missing persisted resource cache")?,
            )?,
            false,
        )?;
    }
    Ok(())
}

pub fn check_processor_requests(requests: &[Value]) -> Result {
    require(requests.len() >= 113, "Missing processor wire requests")?;
    let expected = json!([
        {"method":"GET","path":"/host/echo?name=zero","body_bytes":[],"probe":"processor-shortcut","accept":"*/*"},
        {"method":"GET","path":"/host/echo?name=false","body_bytes":[],"probe":"processor-shortcut","accept":"*/*"},
        {"method":"POST","path":"/host/echo","body_bytes":"订阅🙂\r\n".as_bytes(),"probe":"processor-response","accept":"*/*"},
        {"method":"POST","path":"/host/echo","body_bytes":"嵌套🙂\r\n".as_bytes(),"probe":"processor-nested","accept":"*/*"}
    ]);
    let actual=requests[..4].iter().map(|item| json!({"method":item["method"],"path":item["path"],"body_bytes":item["body_bytes"],"probe":item["headers"].get("x-probe"),"accept":item["headers"].get("accept")})).collect::<Vec<_>>();
    json::check(&json!(actual), &expected, false)?;
    let mut flow = vec![("HEAD", "head", "Flow Helper/1", Some("direct")); 3];
    flow.extend([
        ("HEAD", "get", "Flow Default/1", None),
        ("GET", "get", "Flow Default/1", None),
        ("GET", "body", "Flow Fragment/1", Some("fragment")),
        ("GET", "fallback", "Flow Explicit/1", Some("argument")),
        ("GET", "badstatus", "Flow Explicit/1", Some("argument")),
        ("GET", "rejected", "Flow Default/1", None),
        ("HEAD", "empty", "Flow Default/1", None),
        ("GET", "empty", "Flow Default/1", None),
        ("GET", "delay", "Flow Default/1", None),
        ("HEAD", "queued/first", "Flow Default/1", None),
        ("HEAD", "queued/second", "Flow Default/1", None),
        ("HEAD", "head", "Flow Default/1", None),
    ]);
    let expected=flow.into_iter().map(|(method,path,ua,token)| json!({"method":method,"path":format!("/host/flow/{path}"),"body_bytes":[],"user_agent":ua,"token":token,"accept":"*/*"})).collect::<Vec<_>>();
    let actual=requests[4..19].iter().map(|item| json!({"method":item["method"],"path":item["path"],"body_bytes":item["body_bytes"],"user_agent":item["headers"].get("user-agent"),"token":item["headers"].get("x-flow-token"),"accept":item["headers"].get("accept")})).collect::<Vec<_>>();
    json::check(&json!(actual), &json!(expected), false)?;
    let mut expected=["gzip","x-gzip","deflate","compress","x-compress","nested","status","unknown","too-many"].into_iter().map(|mode| json!({"method":"GET","path":format!("/host/compressed/{mode}"),"body":"","accept":"*/*","proxy_authorization":null})).collect::<Vec<_>>();
    for authorization in [None, Some("Basic Zml4dHVyZTpzZWNyZXQ=")] {
        expected.push(json!({"method":"GET","path":"http://ip-api.com/json/1.2.3.4","body":"","accept":null,"proxy_authorization":authorization}));
    }
    let actual=requests[19..30].iter().map(|item| json!({"method":item["method"],"path":item["path"],"body":item["body"],"accept":item["headers"].get("accept"),"proxy_authorization":item["headers"].get("proxy-authorization")})).collect::<Vec<_>>();
    json::check(&json!(actual), &json!(expected), false)?;
    dns::check_doh_requests(&requests[30..53])?;
    providers::check_gist_requests(&requests[53..113])?;
    check_artifact_requests(&requests[113..])
}

pub fn shared(output: &Path) -> Result {
    reference::verify(output)?;
    let executable = PathBuf::from(
        std::env::var_os("TEST_PROBE_BINARY")
            .ok_or("Missing actual Rust script probe executable")?,
    )
    .canonicalize()?;
    let upstream = Upstream::new(0)?;
    let dns = dns::ControlledDns::new()?;
    for suite in ["host", "processor"] {
        let mut observed = serde_json::Map::new();
        let filename = if suite == "host" {
            "sub-store-probe.json"
        } else {
            "sub-store.json"
        };
        for kind in ["node", "rust"] {
            let state = output.join(format!("{kind}-{suite}-state"));
            std::fs::create_dir(&state)?;
            json::write_new(
                state.join(filename),
                &json!({"settings":{"logsMaxCount":0}}),
            )?;
            let root_seed = if suite == "host" {
                json!({})
            } else {
                json!({"sub-store-cached-resource":"{}","sub-store-cached-headers-resource":"{}","sub-store-cached-script-resource":"{}"})
            };
            json::write_new(state.join("root.json"), &root_seed)?;
            let mut environment = server::environment(&state);
            for key in [
                "HTTP_PROXY",
                "http_proxy",
                "HTTPS_PROXY",
                "https_proxy",
                "ALL_PROXY",
                "all_proxy",
                "NO_PROXY",
                "no_proxy",
                "CARGO_BUILD_TARGET",
                "RUSTFLAGS",
                "CARGO_ENCODED_RUSTFLAGS",
            ] {
                environment.remove(std::ffi::OsStr::new(key));
            }
            server::set(&mut environment, "SUB_STORE_COMPAT_STATE_DIR", &state);
            server::set(
                &mut environment,
                "SUB_STORE_COMPAT_UPSTREAM",
                upstream.url(),
            );
            server::set(
                &mut environment,
                "SUB_STORE_COMPAT_DNS_SERVERS",
                json::stringify(&json!(dns.urls)),
            );
            if suite == "processor" {
                environment.extend(mmdb::prepare(&state)?);
                server::set(&mut environment, "TZ", "UTC");
                if kind == "node" {
                    server::set(&mut environment, "NODE_DEBUG", "tls");
                }
            }
            for resume in [false, true] {
                let label = format!("{kind}-{suite}{}", if resume { "-resume" } else { "" });
                let mut command = if kind == "node" {
                    let mut command = Command::new(reference::node());
                    command
                        .arg("--require")
                        .arg(crate::compat().join("register.cjs"))
                        .arg(crate::compat().join(format!("{suite}-reference.cjs")));
                    if resume {
                        command.arg("--resume");
                    }
                    command
                } else {
                    let mut command = Command::new(&executable);
                    if suite == "host" {
                        if resume {
                            command.arg("--host-resume");
                        }
                    } else {
                        command.arg(if resume {
                            "--processors-resume"
                        } else {
                            "--processors"
                        });
                    }
                    command
                };
                command.env_clear().envs(&environment).current_dir(
                    if kind == "node" && suite == "processor" {
                        crate::root().join("backend")
                    } else {
                        crate::root()
                    },
                );
                let (offset, dns_offset, connection_offset, error_offset) = (
                    upstream.fixture.count(),
                    dns.requests().len(),
                    dns.connections().len(),
                    dns.errors().len(),
                );
                let result = process::run_direct(
                    &mut command,
                    output,
                    &label,
                    Duration::from_secs(60),
                    "Direct native ownership allows the watchdog to kill the evaluated host on Windows",
                )?;
                dns.wait_for_idle()?;
                let requests = upstream.fixture.since(offset);
                let dns_requests = dns.requests()[dns_offset..].to_vec();
                let connections = dns.connections()[connection_offset..].to_vec();
                let errors = dns.errors()[error_offset..].to_vec();
                let progress = result
                    .stderr
                    .lines()
                    .filter_map(|line| line.strip_prefix("COMPAT_PROCESSOR "))
                    .map(json::parse)
                    .collect::<Result<Vec<_>>>()?;
                let mut report = json!({"name":label,"exit_code":result.status.code(),"upstream_requests":requests,"dns_requests":dns_requests,"dns_connections":connections,"dns_errors":errors,"progress":progress,"gist_provider_state":upstream.state.script_gist.snapshot()});
                let checked = (|| -> Result {
                    require(
                        result.success(),
                        format!("Script probe {label} failed or timed out: {}",result.stderr.chars().rev().take(5000).collect::<String>().chars().rev().collect::<String>()),
                    )?;
                    if suite == "processor" {
                        check_progress(&progress, resume)?;
                        providers::check_gist_state(&report["gist_provider_state"])?;
                    }
                    let prefix = format!("COMPAT_{}_RESULT ", suite.to_uppercase());
                    let lines = if kind == "node" {
                        result
                            .stdout
                            .lines()
                            .filter_map(|line| line.strip_prefix(&prefix))
                            .collect::<Vec<_>>()
                    } else {
                        result
                            .stdout
                            .lines()
                            .filter(|line| !line.trim().is_empty())
                            .rev()
                            .take(1)
                            .collect()
                    };
                    require(lines.len() == 1, "Missing or duplicate script result")?;
                    let mut value = json::parse(lines[0])?;
                    if kind == "rust" && suite == "host" && !resume {
                        require(
                            value["os"] == std::env::consts::OS
                                && value["arch"] == std::env::consts::ARCH,
                            "Probe executable does not match native OS/architecture",
                        )?;
                        report["probe"] = value.clone();
                        value = value["host"].clone();
                    }
                    let persisted = json!({"cache":json::read(state.join(filename))?,"root":json::read(state.join("root.json"))?});
                    report["result"] = value.clone();
                    report["persisted"] = persisted.clone();
                    if suite == "processor" {
                        report["mmdb_state_sha256"] = mmdb::check_state(&state)?;
                        check_artifact_state(&value, &persisted)?;
                    }
                    json::check(&value["storage"], &persisted, false)?;
                    if resume {
                        let mut expected = json!({"storage":observed[kind]["storage"]});
                        if suite == "processor" {
                            for key in ["resourceCache", "headersCache", "dnsCache"] {
                                expected[key] = observed[kind][key].clone();
                            }
                        }
                        json::check(&value, &expected, false)?;
                        json::check(&json!(requests), &json!([]), false)?;
                        require(
                            dns_requests.is_empty() && connections.is_empty() && errors.is_empty(),
                            "Resume must not perform DNS operations",
                        )?;
                    } else {
                        json::check(&value["passed"], &json!(identifiers(suite)?), false)?;
                        if suite == "processor" {
                            let mut names = value["observations"]
                                .as_object()
                                .ok_or("Missing processor observations")?
                                .keys()
                                .cloned()
                                .collect::<Vec<_>>();
                            let mut required = identifiers(suite)?
                                .iter()
                                .map(|id| id.as_str().unwrap().to_owned())
                                .collect::<Vec<_>>();
                            names.sort();
                            required.sort();
                            require(names == required, "Missing or extra processor observations")?;
                        }
                        if kind == "node" {
                            observed.insert(kind.into(), value.clone());
                        } else {
                            json::check(&value, &observed["node"], false)?;
                        }
                        observed.insert(kind.into(), value);
                        if suite == "host" {
                            check_host_requests(&requests)?;
                            require(
                                dns_requests.is_empty()
                                    && connections.is_empty()
                                    && errors.is_empty(),
                                "Host-only probe must not perform DNS",
                            )?;
                        } else {
                            check_processor_requests(&requests)?;
                            dns::check_native(&dns_requests, &connections, &errors)?;
                        }
                    }
                    Ok(())
                })();
                if let Err(error) = &checked {
                    report["error"] = json!(error.to_string());
                }
                json::write_new(output.join(format!("{label}.json")), &report)?;
                checked?;
            }
        }
    }
    upstream.fixture.healthy()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn processor_trace_rejects_incomplete_reordered_and_mutated_events() {
        crate::actions_only();
        assert_eq!(identifiers("host").unwrap().len(), 11);
        assert_eq!(identifiers("processor").unwrap().len(), 126);
        let events = identifiers("processor")
            .unwrap()
            .iter()
            .flat_map(|id| {
                [
                    json!({"id":id,"event":"start"}),
                    json!({"id":id,"event":"end"}),
                ]
            })
            .collect::<Vec<_>>();
        check_progress(&events, false).unwrap();
        check_progress(&[], true).unwrap();
        for mutation in [
            "missing",
            "duplicate",
            "id",
            "event",
            "order",
            "extra-field",
            "resume",
        ] {
            let mut actual = events.clone();
            match mutation {
                "missing" => {
                    actual.pop();
                }
                "duplicate" => actual.push(actual.last().unwrap().clone()),
                "id" => actual.last_mut().unwrap()["id"] = json!("unknown"),
                "event" => actual.last_mut().unwrap()["event"] = json!(false),
                "order" => actual.swap(0, 1),
                "extra-field" => actual.last_mut().unwrap()["ignored"] = json!(true),
                _ => (),
            }
            assert!(check_progress(&actual, mutation == "resume").is_err());
        }
    }
}
