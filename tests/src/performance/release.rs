//! Public, network-free application workloads for the latest release chart.
use super::*;
use std::time::{Duration, Instant};
use workload::Suite;

pub const NODES: usize = 10_000;
pub const TARGETS: [&str; 4] = ["ClashMeta", "Surge", "QX", "sing-box"];
const SCRIPT: &str = r#"function operator(proxies) {
  const seen = new Set();
  return proxies.filter(p => {
    if (p.port % 5 === 0) return false;
    const key = p.type + '://' + p.server + ':' + p.port;
    if (seen.has(key)) return false;
    seen.add(key);
    return true;
  }).map(p => ({ ...p, name: 'script-' + p.name }));
}"#;

fn content(variant: usize) -> String {
    let nodes = (0..NODES)
        .map(|index| {
            let name = format!("node-{index:05}");
            let index = index / 2;
            let mut node = json!({"name":name,"server":format!("node-{index}.example"),"port":10000+index});
            let fields = match index % 3 {
                0 => json!({"type":"ss","cipher":"aes-128-gcm","password":"public-fixture"}),
                1 => json!({"type":"vmess","uuid":"11111111-1111-4111-8111-111111111111","alterId":0,"cipher":"auto","tls":true}),
                _ => json!({"type":"trojan","password":"public-fixture","tls":true}),
            };
            node.as_object_mut()
                .unwrap()
                .extend(fields.as_object().unwrap().clone());
            node
        })
        .collect::<Vec<_>>();
    format!(
        "{}{}",
        json!({"proxies":nodes}),
        if variant == 0 { "" } else { "\n" }
    )
}

fn validate(bytes: &[u8], expected: &[u8], javascript: bool) -> Result {
    if javascript {
        let actual: Value = serde_json::from_slice(bytes)?;
        let expected: Value = serde_json::from_slice(expected)?;
        check(
            actual.as_array().is_some_and(|nodes| nodes.len() == 4000),
            "work_incomplete",
        )?;
        crate::json::check(&actual, &expected, false)
    } else {
        check(bytes == expected, "output_mismatch")?;
        check(
            std::str::from_utf8(bytes)?.matches("bench-node-").count() == NODES,
            "work_incomplete",
        )
    }
}

pub fn measure(suite: &mut Suite, report: &mut Value) -> Result {
    let environment = suite.environment.clone();
    crate::server::set(&mut suite.environment, "SUB_STORE_BODY_JSON_LIMIT", "8mb");
    let mut servers = BTreeMap::new();
    let inputs = [content(0), content(1)];
    for kind in KINDS {
        let directory = suite.directory(&format!("release-{kind}"))?;
        let app = suite.server(kind, directory, false)?;
        for javascript in [false, true] {
            for (variant, input) in inputs.iter().enumerate() {
                let process = if javascript {
                    json!([{"type":"Script Operator","args":{"mode":"script","content":SCRIPT}}])
                } else {
                    json!([{"type":"Regex Rename Operator","args":[{"expr":"^","now":"bench-"}]}])
                };
                transport::api(
                    &app.origin,
                    "subs",
                    "POST",
                    Some(
                        &json!({"name":format!("release-{javascript}-{variant}"),"source":"local","content":input,"process":process}),
                    ),
                )?;
            }
        }
        servers.insert(kind, app);
    }
    suite.environment = environment;
    report["release_workloads"] = json!([]);
    for (phase, target) in TARGETS
        .into_iter()
        .map(|target| ("conversion", target))
        .chain([("javascript", "JSON")])
    {
        let javascript = phase == "javascript";
        let paths = [0, 1].map(|variant| {
            format!("/backend/download/release-{javascript}-{variant}?target={target}")
        });
        let expected = paths
            .iter()
            .map(|path| transport::http(&format!("{}{path}", servers["node"].origin), "GET", None))
            .collect::<Result<Vec<_>>>()?;
        let mut connections = KINDS
            .into_iter()
            .map(|kind| Ok((kind, transport::Connection::new(&servers[kind].origin)?)))
            .collect::<Result<BTreeMap<_, _>>>()?;
        let mut ordinals = KINDS
            .map(|kind| (kind, 0_usize))
            .into_iter()
            .collect::<BTreeMap<_, _>>();
        for kind in KINDS {
            for _ in 0..5 {
                let ordinal = ordinals.get_mut(kind).unwrap();
                let variant = *ordinal % 2;
                *ordinal += 1;
                let bytes = connections.get_mut(kind).unwrap().request(
                    "GET",
                    &paths[variant],
                    None,
                    true,
                )?;
                validate(&bytes, &expected[variant], javascript)?;
            }
        }
        for block in 0..single::BLOCKS {
            for kind in if block % 2 == 0 {
                KINDS
            } else {
                ["rust", "node"]
            } {
                let pid = servers[kind].owned.pid();
                let before = resources::counters(pid)?;
                let began = Instant::now();
                let mut samples = Vec::new();
                let cpu = loop {
                    let ordinal = ordinals.get_mut(kind).unwrap();
                    let variant = *ordinal % 2;
                    *ordinal += 1;
                    let started = Instant::now();
                    let bytes = connections.get_mut(kind).unwrap().request(
                        "GET",
                        &paths[variant],
                        None,
                        true,
                    )?;
                    samples.push(started.elapsed().as_nanos() as u64);
                    let after = resources::counters(pid)?;
                    check(after.identity == before.identity, "resource")?;
                    validate(&bytes, &expected[variant], javascript)?;
                    let cpu = after.cpu - before.cpu;
                    if samples.len() >= 4 && cpu >= 1.0 {
                        break cpu;
                    }
                    check(
                        samples.len() < 256 && began.elapsed() < Duration::from_secs(60),
                        "resource",
                    )?;
                };
                println!(
                    "Release workload {phase}/{target} {kind} block={block}: {} requests, {cpu:.3} CPU seconds",
                    samples.len()
                );
                report["release_workloads"].as_array_mut().unwrap().push(json!({"variant":kind,"phase":phase,"target":target,"block":block,"input_nodes":NODES,"output_nodes":if javascript{4000}else{NODES},"input_sha256":inputs.iter().map(digest).collect::<Vec<_>>(),"output_sha256":expected.iter().map(digest).collect::<Vec<_>>(),"script_sha256":if javascript{Some(digest(SCRIPT))}else{None},"requests":samples.len(),"elapsed_ns":samples.iter().sum::<u64>(),"cpu_s":cpu,"samples_ns":samples}));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn public_batch_has_exact_work_and_alternates_equivalent_uncached_text() -> Result {
        let first = content(0);
        let second = content(1);
        assert_ne!(first, second);
        let data: Value = serde_json::from_str(&first)?;
        assert_eq!(data, serde_json::from_str::<Value>(&second)?);
        let nodes = data["proxies"].as_array().unwrap();
        assert_eq!(nodes.len(), NODES);
        assert_eq!(nodes[0]["server"], nodes[1]["server"]);
        assert_ne!(nodes[0]["name"], nodes[1]["name"]);
        assert_eq!(
            nodes
                .iter()
                .filter(|node| node["port"].as_u64().unwrap() % 5 != 0)
                .count()
                / 2,
            4000
        );
        assert!(validate(b"[]", b"[]", true).is_err());
        assert!(validate(b"bench-node-one", b"bench-node-one", false).is_err());
        Ok(())
    }
}
