use super::*;
use std::{
    process::Command,
    time::{Duration, Instant},
};
use workload::{Application, Suite};

pub fn snapshot_data(root: &Value) -> Result<(String, Value, Value)> {
    let cache: Value = serde_json::from_str(
        root["sub-store-cached-script-resource"]
            .as_str()
            .ok_or("Script resource cache missing")?,
    )?;
    let cache = cache.as_object().ok_or("Script cache object missing")?;
    let health = cache
        .iter()
        .filter(|(key, _)| key.starts_with("http-meta:health-inc:v1:"))
        .map(|(_, value)| &value["data"])
        .collect::<Vec<_>>();
    let snapshots = cache
        .iter()
        .filter(|(key, _)| key.starts_with("health-snapshot:v1:"))
        .map(|(key, value)| (key, &value["data"]))
        .collect::<Vec<_>>();
    check(health.len() == 1 && snapshots.len() == 1, "snapshot")?;
    Ok((
        snapshots[0].0.clone(),
        snapshots[0].1.clone(),
        health[0]["records"].clone(),
    ))
}
#[allow(clippy::too_many_arguments)]
pub fn audit(
    snapshot: &Value,
    records: &Value,
    output: &Value,
    projected: &Value,
    counts: &Value,
    cold: Option<bool>,
    real: bool,
    now: f64,
) -> Result<Value> {
    let outputs = output.as_array().ok_or("Output nodes missing")?;
    let records = records.as_object().ok_or("Health records missing")?;
    check(
        snapshot["version"] == 1
            && snapshot["count"].as_u64() == Some(outputs.len() as u64)
            && !outputs.is_empty(),
        "snapshot",
    )?;
    let generated = snapshot["generatedAt"]
        .as_f64()
        .ok_or("Snapshot timestamp missing")?;
    check(
        generated.is_finite() && now >= generated && now - generated < 300000.0,
        "snapshot",
    )?;
    check(
        nodes_hash(projected, false)? == nodes_hash(output, false)?,
        "snapshot",
    )?;
    check(
        records.values().all(|record| {
            record["lastProbe"]
                .as_f64()
                .is_some_and(|value| value > 0.0)
        }),
        "work_incomplete",
    )?;
    match cold {
        Some(true) => {
            check(
                counts["starts"] == 1
                    && counts["stops"] == 1
                    && counts["selected"].as_u64() == Some(records.len() as u64)
                    && !records.is_empty(),
                "work_incomplete",
            )?;
            if !real {
                check(
                    counts["unique_probes"].as_u64() == Some(records.len() as u64)
                        && counts["probes"]
                            .as_u64()
                            .is_some_and(|count| count >= records.len() as u64),
                    "work_incomplete",
                )?;
            }
        }
        Some(false) => check(
            ["starts", "selected", "probes"]
                .iter()
                .all(|key| counts[key] == 0),
            "warm_miss",
        )?,
        None => {}
    }
    Ok(
        json!({"identities":records.len(),"published":snapshot["count"],"generated_at_ms":snapshot["generatedAt"],"snapshot_sha256":nodes_hash(&snapshot["proxies"],false)?,"verified":records.values().filter(|record|record["lastOk"].as_f64().is_some_and(|value|value>0.0)).count()}),
    )
}
impl Suite {
    pub fn audit(
        &mut self,
        app: &Application,
        output: &Value,
        cold: Option<bool>,
        counts: &Value,
        real: bool,
        cross: bool,
    ) -> Result<Value> {
        let (key, snapshot, records) = snapshot_data(&read(app.state.join("root.json"))?)?;
        if !real {
            if let Some(expected) = &self.snapshot {
                if cross {
                    compare_nodes(expected, &snapshot["proxies"], false)?;
                }
            } else {
                self.snapshot_hash = Some(nodes_hash(&snapshot["proxies"], false)?);
                self.snapshot = Some(snapshot["proxies"].clone());
            }
        }
        let projected = transport::api(
            &app.origin,
            "preview/sub",
            "POST",
            Some(
                &json!({"name":"benchmark-snapshot-validation","source":"local","content":"ss://YWVzLTEyOC1nY206YmVuY2g@127.0.0.1:1#validation","process":[{"type":"Script Operator","args":{"mode":"script","content":"function operator() { const s = scriptResourceCache.get($arguments.key); if (!s) throw new Error('missing snapshot'); return ProxyUtils.produce(JSON.parse(JSON.stringify(s.proxies)), 'JSON', 'internal'); }","arguments":{"key":key}}}]}),
            ),
        )?;
        audit(
            &snapshot,
            &records,
            output,
            &projected["processed"],
            counts,
            cold,
            real,
            workload::now_ms(),
        )
    }
    pub fn check_output(&mut self, raw: &[u8], compare: bool, key: &str) -> Result<Value> {
        let output: Value = serde_json::from_slice(raw)?;
        check(
            output
                .as_array()
                .is_some_and(|nodes| !nodes.is_empty() && nodes.iter().all(Value::is_object)),
            "empty_output",
        )?;
        if compare {
            let fingerprint = nodes_hash(&output, key == "parse")?;
            let expected = self
                .expected
                .entry(key.to_owned())
                .or_insert_with(|| output.clone());
            compare_nodes(expected, &output, key == "parse")?;
            self.expected_hashes
                .entry(key.to_owned())
                .or_insert(fingerprint);
        }
        Ok(output)
    }
    pub fn diagnose_sources(&mut self) -> Result {
        let mut expected = Vec::new();
        self.report["direct_sources"] = json!([]);
        for kind in KINDS {
            let directory = self.directory(&format!("direct-{kind}"))?;
            let app = self.server(kind, directory, true)?;
            for index in 0..22 {
                let source = std::fs::read_to_string(
                    self.args.scratch.join(format!("mirrors/{index}.txt")),
                )?;
                let before = resources::counters(app.owned.pid())?.cpu;
                let began = Instant::now();
                let result = transport::api(
                    &app.origin,
                    "preview/sub",
                    "POST",
                    Some(
                        &json!({"name":"parse-diagnostic","source":"local","content":source,"process":[]}),
                    ),
                )?;
                let elapsed = began.elapsed().as_secs_f64();
                let cpu = resources::counters(app.owned.pid())?.cpu - before;
                let output = &result["processed"];
                check(output.is_array(), "http")?;
                let mut row = json!({"kind":kind,"source_index":index,"input_bytes":source.len(),"elapsed_s":elapsed,"cpu_s":cpu,"output_count":output.as_array().unwrap().len(),"output_sha256":nodes_hash(output,false)?,"status":"passed"});
                if kind == "node" {
                    expected.push(output.clone());
                } else if let Err(error) = compare_nodes(&expected[index], output, false) {
                    row["status"] = json!("failed");
                    row.as_object_mut()
                        .unwrap()
                        .extend(failure(error.as_ref()).as_object().unwrap().clone());
                }
                self.report["direct_sources"]
                    .as_array_mut()
                    .unwrap()
                    .push(row);
                println!(
                    "Direct parse {kind} source={index}: elapsed_s={elapsed:.3}, cpu_s={cpu:.3}, nodes={}",
                    output.as_array().unwrap().len()
                );
                self.publish()?;
            }
        }
        Ok(())
    }
    #[allow(clippy::too_many_arguments)]
    fn measure_case(
        &mut self,
        kind: &str,
        case: &str,
        original: &Value,
        clients: u64,
        repeats: Option<usize>,
        directory: &std::path::Path,
        row: &mut Value,
    ) -> Result {
        let app = self.server(kind, directory.to_owned(), true)?;
        let variant = if ["parse", "pre_health", "no_publisher"].contains(&case) {
            case
        } else {
            "full"
        };
        let config = self.configure(&app.origin, original, true, variant)?;
        row["startup_s"] = json!(app.startup);
        let name = config["name"]
            .as_str()
            .ok_or("Configured subscription name missing")?;
        let warm = ["warm", "pressure", "no_publisher"].contains(&case);
        let mut warm_output = Value::Null;
        let mut warm_hash = String::new();
        if warm {
            row["warmup_output_counts"] = json!([]);
            for _ in 0..2 {
                let raw = self.download(&app.origin, name, "JSON")?;
                warm_output = self.check_output(&raw, false, "full")?;
                warm_hash = digest(&raw);
                row["warmup_output_counts"]
                    .as_array_mut()
                    .unwrap()
                    .push(json!(warm_output.as_array().unwrap().len()));
            }
            row["warmup_counts"] = self.counts(false, false)?;
            if case != "no_publisher" {
                self.audit(
                    &app,
                    &warm_output,
                    None,
                    &self.counts(false, false)?,
                    false,
                    false,
                )?;
            }
            self.counts(true, false)?;
        }
        let offset = std::fs::metadata(&app.owned.process.stdout)?.len();
        let key = if ["parse", "pre_health"].contains(&case) {
            case
        } else {
            "full"
        };
        let mut mismatch = None;
        let mut last = Value::Null;
        let mut raw_hash = String::new();
        let mut timing_error = None;
        if case == "pressure" {
            let fingerprint = nodes_hash(&warm_output, false)?;
            self.expected_hashes
                .entry("full".to_owned())
                .or_insert_with(|| fingerprint.clone());
            self.expected
                .entry("full".to_owned())
                .or_insert_with(|| warm_output.clone());
            if self.expected_hashes["full"] != fingerprint {
                mismatch = Some(warm_output.clone());
            }
        }
        let meter = resources::Meter::start(
            app.owned.pid(),
            self.fixture.as_ref().map(workload::Owned::pid),
            if case == "real" {
                self.meta.as_ref().map(workload::Owned::pid)
            } else {
                None
            },
        )?;
        let mut latencies = Vec::new();
        let mut output_counts = Vec::new();
        if case == "pressure" {
            let until = Instant::now() + Duration::from_secs(20);
            let url = format!(
                "{}/backend/download/{}?target=JSON",
                app.origin,
                crate::fixture::quote(name)
            );
            let batches = std::thread::scope(|scope| {
                let handles = (0..clients)
                    .map(|_| {
                        let url = &url;
                        let expected = &warm_hash;
                        scope.spawn(move || {
                            let mut samples = Vec::new();
                            let mut error = None;
                            while Instant::now() < until {
                                let began = Instant::now();
                                match transport::http(url, "GET", None) {
                                    Ok(raw) => {
                                        let elapsed = began.elapsed().as_secs_f64();
                                        let fingerprint = digest(&raw);
                                        samples.push((elapsed, fingerprint.clone()));
                                        if &fingerprint != expected {
                                            error = Some(Box::new(Failure {
                                                code: "output_mismatch",
                                                observations: None,
                                            })
                                                as Box<dyn std::error::Error + Send + Sync>);
                                            break;
                                        }
                                    }
                                    Err(failed) => {
                                        error = Some(failed);
                                        break;
                                    }
                                }
                            }
                            (samples, error)
                        })
                    })
                    .collect::<Vec<_>>();
                handles
                    .into_iter()
                    .map(|handle| handle.join().map_err(|_| "Pressure client panicked".into()))
                    .collect::<Result<Vec<_>>>()
            });
            match batches {
                Ok(batches) => {
                    for (samples, error) in batches {
                        for (elapsed, fingerprint) in samples {
                            latencies.push(elapsed);
                            output_counts.push(warm_output.as_array().unwrap().len());
                            raw_hash = fingerprint;
                        }
                        if error.is_some() {
                            timing_error = error;
                        }
                    }
                    last = warm_output;
                }
                Err(error) => timing_error = Some(error),
            }
        } else {
            let count = repeats.unwrap_or(
                if ["warm", "parse", "pre_health", "no_publisher"].contains(&case) {
                    20
                } else {
                    1
                },
            );
            for _ in 0..count {
                let began = Instant::now();
                let raw = match self.download(&app.origin, name, "JSON") {
                    Ok(raw) => raw,
                    Err(error) => {
                        timing_error = Some(error);
                        break;
                    }
                };
                let elapsed = began.elapsed().as_secs_f64();
                raw_hash = digest(&raw);
                latencies.push(elapsed);
                let output = match self.check_output(&raw, false, case) {
                    Ok(output) => output,
                    Err(error) => {
                        timing_error = Some(error);
                        break;
                    }
                };
                output_counts.push(output.as_array().unwrap().len());
                if case != "real" {
                    let fingerprint = nodes_hash(&output, case == "parse")?;
                    self.expected
                        .entry(key.to_owned())
                        .or_insert_with(|| output.clone());
                    self.expected_hashes
                        .entry(key.to_owned())
                        .or_insert_with(|| fingerprint.clone());
                    if self.expected_hashes[key] != fingerprint && mismatch.is_none() {
                        mismatch = Some(output.clone());
                    }
                }
                last = output;
            }
        }
        row["latencies_s"] = json!(latencies);
        row["output_counts"] = json!(output_counts);
        row.as_object_mut()
            .unwrap()
            .extend(meter.finish(latencies.len())?.as_object().unwrap().clone());
        if let Some(error) = timing_error {
            return Err(error);
        }
        check(!latencies.is_empty() && last.is_array(), "empty_output")?;
        row["output_count"] = json!(last.as_array().unwrap().len());
        row["output_sha256"] = json!(nodes_hash(&last, case == "parse")?);
        row["raw_output_sha256"] = json!(raw_hash);
        row["counts"] = self.counts(false, false)?;
        if self.args.profile {
            row["profiler_enabled"] = json!(kind == "rust_repeat");
            let profiles =
                pipeline_profiles(&app.owned.process.stdout, offset, "download_sub", true)?;
            check(
                profiles.len()
                    == if kind == "rust_repeat" {
                        latencies.len()
                    } else {
                        0
                    },
                "resource",
            )?;
            row["pipeline_profiles"] = json!(profiles);
        }
        if self.args.diagnostic.as_deref() == Some("health")
            && !self.args.profile
            && case != "pre_health"
        {
            row["health_profile"] =
                read(app.state.join("root.json"))?["benchmark-health-profile"].clone();
            check(row["health_profile"].is_object(), "resource")?;
        }
        if !["parse", "pre_health", "no_publisher"].contains(&case) {
            row["work"] = self.audit(
                &app,
                &last,
                Some(!warm),
                &row["counts"],
                case == "real",
                false,
            )?;
        }
        if case == "warm" && row["block"] == 0 {
            let clash = self.download(&app.origin, name, "ClashMeta")?;
            let directory = self.directory("clash-output-validation")?;
            let bytes=transport::pipe(Command::new(&self.node).args(["-e","const fs=require('fs'),yaml=require('yaml');process.stdout.write(JSON.stringify(yaml.parse(fs.readFileSync(0,'utf8'))))"]).current_dir(self.args.original.join("backend")).env_clear().envs(&self.environment),clash,&directory,"yaml",Duration::from_secs(90))?;
            let parsed: Value = serde_json::from_slice(&bytes)?;
            let proxies = parsed.get("proxies").unwrap_or(&parsed);
            let count = proxies.as_array().ok_or("Clash node list missing")?.len();
            let fingerprint = hash(proxies);
            row["clash_count"] = json!(count);
            row["clash_sha256"] = json!(fingerprint);
            check(count == last.as_array().unwrap().len(), "clash")?;
            if let Some(expected) = &self.clash_hash {
                check(expected == &fingerprint, "clash")?;
            } else {
                self.clash_hash = Some(fingerprint);
            }
        }
        if let Some(mismatch) = mismatch {
            compare_nodes(&self.expected[key], &mismatch, case == "parse")?;
        }
        if case != "real"
            && let Some(work) = row.get("work")
        {
            check(
                work["snapshot_sha256"].as_str() == self.snapshot_hash.as_deref(),
                "output_mismatch",
            )?;
        }
        row["status"] = json!("passed");
        Ok(())
    }
    pub fn run_case(
        &mut self,
        kind: &str,
        case: &str,
        block: u64,
        original: &Value,
        clients: u64,
        repeats: Option<usize>,
    ) -> Result {
        let directory = self.directory(&format!("{case}-{kind}"))?;
        self.counts(true, case == "real")?;
        let mut row = json!({"kind":kind,"case":case,"block":block,"concurrency":clients,"status":"running","latencies_s":[],"output_counts":[],"errors":0});
        let index = self.report["samples"]
            .as_array()
            .ok_or("Samples missing")?
            .len();
        self.report["samples"]
            .as_array_mut()
            .unwrap()
            .push(row.clone());
        self.publish()?;
        let result =
            self.measure_case(kind, case, original, clients, repeats, &directory, &mut row);
        let mut fatal = None;
        if let Err(error) = result {
            row["status"] = json!("failed");
            row["errors"] = json!(1);
            row.as_object_mut()
                .unwrap()
                .extend(failure(error.as_ref()).as_object().unwrap().clone());
            row["counts"] = self.counts(false, false)?;
            std::fs::write(directory.join("failure.txt"), format!("{error:?}"))?;
            let mut logs = String::new();
            for filename in ["server.stdout.log", "server.stderr.log"] {
                if let Ok(log) = std::fs::read_to_string(directory.join(filename)) {
                    logs += &log;
                }
            }
            let pattern = regex::Regex::new(
                r"input=(\d+) supported=(\d+) identities=(\d+)\s+probe_coalesced=(\d+) cache=(?:hit|empty)\s+probed=(\d+)/(\d+) ok=(\d+) failed=(\d+)\s+guarded=(\d+) budget_skipped=(\d+)\s+output=(\d+)",
            )?;
            row["health_counts"] = json!(
                pattern
                    .captures_iter(&logs)
                    .map(|capture| (1..12)
                        .map(|index| capture[index].parse::<u64>())
                        .collect::<std::result::Result<Vec<_>, _>>())
                    .collect::<std::result::Result<Vec<_>, _>>()?
            );
            if let Ok(root) = read(directory.join("state/root.json"))
                && let Some(cache) = root["sub-store-cached-script-resource"].as_str()
            {
                let cache: Value = serde_json::from_str(cache)?;
                let health = cache
                    .as_object()
                    .ok_or("Failed-cell cache missing")?
                    .iter()
                    .filter(|(key, _)| key.starts_with("http-meta:health-inc:v1:"))
                    .map(|(_, value)| &value["data"]["records"])
                    .collect::<Vec<_>>();
                row["health_records"] = json!(
                    health
                        .iter()
                        .filter_map(|records| records.as_object().map(serde_json::Map::len))
                        .sum::<usize>()
                );
                row["health_verified"] = json!(
                    health
                        .iter()
                        .filter_map(|records| records.as_object())
                        .flat_map(serde_json::Map::values)
                        .filter(|record| record["lastOk"].as_f64().is_some_and(|value| value > 0.0))
                        .count()
                );
            }
            if ![
                "output_mismatch",
                "snapshot",
                "work_incomplete",
                "warm_miss",
                "clash",
                "empty_output",
                "http",
                "timeout",
            ]
            .contains(&row["failure"].as_str().unwrap_or("unexpected"))
            {
                fatal = Some(error);
            }
        }
        self.report["samples"][index] = row.clone();
        self.publish()?;
        println!(
            "Cell {case}/{kind} block={} clients={clients}: {}; requests={}, output={}, elapsed_s={}, failure={}",
            block + 1,
            row["status"].as_str().unwrap_or("failed"),
            row["completed"],
            row["output_count"],
            row["elapsed_s"],
            row["failure"].as_str().unwrap_or("none")
        );
        if let Some(error) = fatal {
            return Err(error);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn partial_probe_work_empty_or_stale_snapshots_and_warm_cache_misses_fail() -> Result {
        let now = workload::now_ms();
        let snapshot = json!({"version":1,"count":1,"generatedAt":now,"proxies":[{"name":"x"}]});
        let records = json!({"a":{"lastProbe":1,"lastOk":1},"b":{"lastProbe":1,"lastOk":1}});
        let output = json!([{"name":"x"}]);
        let counts = json!({"starts":1,"stops":1,"selected":2,"unique_probes":2,"probes":2});
        assert_eq!(
            audit(
                &snapshot,
                &records,
                &output,
                &output,
                &counts,
                Some(true),
                false,
                now
            )?["identities"],
            2
        );
        let mut retry = counts.clone();
        retry["probes"] = json!(3);
        audit(
            &snapshot,
            &records,
            &output,
            &output,
            &retry,
            Some(true),
            false,
            now,
        )?;
        retry["starts"] = json!(2);
        audit(
            &snapshot, &records, &output, &output, &retry, None, false, now,
        )?;
        for key in ["selected", "unique_probes", "probes"] {
            let mut broken = counts.clone();
            broken[key] = json!(1);
            assert!(
                audit(
                    &snapshot,
                    &records,
                    &output,
                    &output,
                    &broken,
                    Some(true),
                    false,
                    now
                )
                .is_err()
            );
        }
        assert!(
            audit(
                &snapshot,
                &records,
                &json!([]),
                &output,
                &counts,
                Some(true),
                false,
                now
            )
            .is_err()
        );
        assert!(
            audit(
                &snapshot,
                &records,
                &output,
                &output,
                &counts,
                Some(false),
                false,
                now
            )
            .is_err()
        );
        assert!(
            audit(
                &snapshot,
                &records,
                &output,
                &output,
                &counts,
                Some(true),
                false,
                now + 300001.0
            )
            .is_err()
        );
        assert!(
            audit(
                &snapshot,
                &records,
                &output,
                &json!([{"name":"changed"}]),
                &counts,
                None,
                false,
                now
            )
            .is_err()
        );
        Ok(())
    }
}
