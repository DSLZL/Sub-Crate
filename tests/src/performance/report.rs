use super::*;
use std::path::PathBuf;

fn items(value: &Value) -> &[Value] {
    value.as_array().map(Vec::as_slice).unwrap_or(&[])
}
fn nonrunner(value: &Value) -> Value {
    let mut value = value.clone();
    if let Some(map) = value.as_object_mut() {
        map.remove("runner");
    }
    value
}
fn identity(row: &Value) -> (String, u64, u64, String) {
    (
        row["case"].as_str().unwrap_or("").to_owned(),
        row["block"].as_u64().unwrap_or(u64::MAX),
        row["concurrency"].as_u64().unwrap_or(0),
        row["kind"].as_str().unwrap_or("").to_owned(),
    )
}
pub fn aggregate(
    prepared: &Value,
    workers: &[(String, Value)],
    diagnostic: Option<&str>,
) -> Result<Value> {
    let mut report = prepared.clone();
    report
        .as_object_mut()
        .ok_or("Preparation report missing")?
        .remove("preparation_complete");
    report["status"] = json!("running");
    report["samples"] = json!([]);
    report["workers"] = json!([]);
    report["matrix_complete"] = json!(false);
    let shards = shards();
    let expected = if diagnostic.is_some() {
        BTreeSet::from(["diagnostic".to_owned()])
    } else {
        shards.iter().map(|(name, _)| name.clone()).collect()
    };
    let mut seen = BTreeSet::new();
    let imports = items(&prepared["imports"]);
    let mut valid = prepared["status"] == "passed"
        && prepared["preparation_complete"] == true
        && imports.len() == 10
        && imports.iter().all(|row| row["restart"] == true);
    if let Some(diagnostic) = diagnostic {
        check(["parse", "health"].contains(&diagnostic), "input")?;
        report["diagnostic"] = json!(format!("{diagnostic}_diagnostic"));
    }
    for (shard, worker) in workers {
        valid &= expected.contains(shard) && seen.insert(shard.clone());
        valid &= nonrunner(&worker["provenance"]) == nonrunner(&prepared["provenance"])
            && worker["capture"] == prepared["capture"]
            && worker["script"] == prepared["script"];
        valid &= worker["native_pipeline_profile"].as_bool().unwrap_or(false)
            == prepared["native_pipeline_profile"]
                .as_bool()
                .unwrap_or(false);
        if prepared["native_pipeline_profile"] == true {
            valid &= worker["script"].get("profile_sha256").is_none();
        }
        let replay = items(&worker["replay"]);
        valid &= replay.len() == 2
            && replay
                .iter()
                .filter_map(|row| row["kind"].as_str())
                .collect::<BTreeSet<_>>()
                == KINDS.into_iter().collect()
            && replay.iter().all(|row| {
                row["equal"] == true && row["matching_sources"] == 23 && row["source_count"] == 23
            });
        let cells = items(&worker["samples"]);
        let actual = cells.iter().map(identity).collect::<BTreeSet<_>>();
        if let Some(diagnostic) = diagnostic {
            let cases = if diagnostic == "parse" {
                vec!["parse", "pre_health"]
            } else {
                vec!["pre_health", "cold", "warm", "no_publisher"]
            };
            let kinds = if diagnostic == "parse" {
                KINDS.to_vec()
            } else {
                COMPARISON_KINDS.to_vec()
            };
            let required = cases
                .into_iter()
                .flat_map(|case| {
                    kinds
                        .iter()
                        .map(move |kind| (case.to_owned(), 0, 1, (*kind).to_owned()))
                })
                .collect::<BTreeSet<_>>();
            valid &= actual == required && cells.len() == required.len();
            if diagnostic == "parse" {
                let direct = items(&worker["direct_sources"]);
                let required = KINDS
                    .into_iter()
                    .flat_map(|kind| (0..22).map(move |index| (kind.to_owned(), index)))
                    .collect::<BTreeSet<_>>();
                valid &= direct.len() == 44
                    && direct
                        .iter()
                        .map(|row| {
                            (
                                row["kind"].as_str().unwrap_or("").to_owned(),
                                row["source_index"].as_u64().unwrap_or(u64::MAX),
                            )
                        })
                        .collect::<BTreeSet<_>>()
                        == required
                    && direct.iter().all(|row| row["status"] == "passed");
            }
        } else if let Some((_, (case, block, clients))) =
            shards.iter().find(|(name, _)| name == shard)
        {
            valid &= cells.len() == 2
                && actual
                    == KINDS
                        .into_iter()
                        .map(|kind| ((*case).to_owned(), *block, *clients, kind.to_owned()))
                        .collect();
        } else {
            valid = false;
        }
        valid &= worker["status"] == "passed"
            && cells.iter().all(|row| {
                row["status"] == "passed"
                    && row["output_sha256"].as_str().is_some_and(|value| {
                        value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
                    })
            });
        report["samples"]
            .as_array_mut()
            .unwrap()
            .extend(cells.iter().map(|row| {
                let mut row = row.clone();
                if diagnostic.is_none() {
                    row["worker"] = json!(shard);
                }
                row
            }));
        let mut observation = json!({"runner":worker["provenance"]["runner"],"replay":replay,"fixture_calibration":worker["fixture_calibration"],"failure":worker["failure"],"status":worker["status"].as_str().unwrap_or("failed")});
        if diagnostic.is_none() {
            observation["shard"] = json!(shard);
        } else {
            report["direct_sources"] = worker.get("direct_sources").cloned().unwrap_or(json!([]));
        }
        report["workers"].as_array_mut().unwrap().push(observation);
        if shard == "real_0_1"
            && let Some(detector) = worker.get("detector")
        {
            report["detector"] = detector.clone();
        }
    }
    valid &= seen == expected;
    for case in ["parse", "pre_health", "full"] {
        let hashes = items(&report["samples"])
            .iter()
            .filter(|row| {
                row["case"] != "real"
                    && row["status"] == "passed"
                    && if [json!("parse"), json!("pre_health")].contains(&row["case"]) {
                        row["case"] == case
                    } else {
                        case == "full"
                    }
            })
            .map(|row| row.get("output_sha256").map(hash))
            .collect::<BTreeSet<_>>();
        valid &= hashes.len() <= 1;
    }
    report["matrix_complete"] =
        json!(diagnostic.is_none() && seen == expected && items(&report["samples"]).len() == 70);
    report["status"] = json!(if valid { "passed" } else { "failed" });
    if !valid {
        report["failure"] = json!("work_incomplete");
    }
    Ok(report)
}
pub fn latest_workers(directory: &Path, run: &str) -> Result<Vec<(String, Value)>> {
    if directory.join("metrics.json").is_file() {
        let worker = read(directory.join("metrics.json"))?;
        let shard = worker["shard"]
            .as_str()
            .unwrap_or(if worker.get("diagnostic").is_some() {
                "diagnostic"
            } else {
                ""
            });
        check(
            shard == "diagnostic" || shards().iter().any(|(name, _)| name == shard),
            "source",
        )?;
        return Ok(vec![(shard.to_owned(), worker)]);
    }
    let mut selected: BTreeMap<String, (u64, PathBuf)> = BTreeMap::new();
    let mut entries = std::fs::read_dir(directory)?.collect::<std::io::Result<Vec<_>>>()?;
    entries.sort_by_key(std::fs::DirEntry::file_name);
    for entry in entries {
        let path = entry.path().join("metrics.json");
        if !path.is_file() {
            continue;
        }
        let filename = entry.file_name();
        let filename = filename.to_str().ok_or("Invalid artifact name")?;
        let name = filename
            .strip_prefix("subscription-performance-shard-")
            .ok_or("Unexpected worker artifact")?;
        let mut parts = name.rsplitn(3, '-');
        let attempt = parts.next().ok_or("Missing attempt")?;
        let source_run = parts.next().ok_or("Missing run")?;
        let shard = parts.next().ok_or("Missing shard")?;
        check(
            source_run == run
                && !attempt.is_empty()
                && attempt.bytes().all(|byte| byte.is_ascii_digit()),
            "source",
        )?;
        let attempt = attempt.parse::<u64>()?;
        check(
            attempt > 0
                && (shard == "diagnostic" || shards().iter().any(|(name, _)| name == shard)),
            "source",
        )?;
        if selected
            .get(shard)
            .is_none_or(|(previous, _)| attempt > *previous)
        {
            selected.insert(shard.to_owned(), (attempt, path));
        }
    }
    selected
        .into_iter()
        .map(|(shard, (_, path))| Ok((shard, read(path)?)))
        .collect()
}
pub fn estimates(rows: &[Value]) -> Result<Value> {
    let mut estimates = Vec::new();
    for case in [
        "cold",
        "warm",
        "pressure",
        "parse",
        "pre_health",
        "no_publisher",
    ] {
        for clients in [1, 2, 4, 8] {
            let mut pairs = Vec::new();
            for block in 0..10 {
                let pair = KINDS.map(|kind| {
                    rows.iter().find(|row| {
                        row["case"] == case
                            && row["kind"] == kind
                            && row["block"] == block
                            && row["concurrency"] == clients
                            && row["status"] == "passed"
                    })
                });
                if let [Some(node), Some(rust)] = pair
                    && node["output_sha256"] == rust["output_sha256"]
                {
                    let metric = |row: &Value| -> Result<f64> {
                        if case == "pressure" {
                            row["rps"]
                                .as_f64()
                                .ok_or_else(|| "Throughput missing".into())
                        } else {
                            median(
                                &items(&row["latencies_s"])
                                    .iter()
                                    .map(|value| value.as_f64().ok_or("Latency missing"))
                                    .collect::<std::result::Result<Vec<_>, _>>()?,
                            )
                        }
                    };
                    pairs.push((metric(node)?, metric(rust)?));
                }
            }
            if !pairs.is_empty() {
                let mut interval = paired_interval(&pairs)?;
                interval["case"] = json!(case);
                interval["concurrency"] = json!(clients);
                estimates.push(interval);
            }
        }
    }
    Ok(json!(estimates))
}
pub fn pipeline_summary(samples: &[Value]) -> Result<String> {
    type ProcessorKey = (Option<u64>, Option<u64>, String);
    let mut text = String::new();
    for sample in samples {
        let profiles = items(&sample["pipeline_profiles"]);
        if profiles.is_empty() {
            continue;
        }
        text += &format!(
            "\nPipeline profile: {} ({} requests). Same candidate binary; profiler-on latency includes report writing. Active time is elapsed scoped work, not OS CPU time. Parent and child intervals overlap.\n\n| Process | Processor | Type | Mean wall ms | Mean active ms |\n|---:|---:|---|---:|---:|\n",
            sample["case"].as_str().ok_or("Case missing")?,
            profiles.len()
        );
        let mut processors: BTreeMap<ProcessorKey, (u128, u128)> = BTreeMap::new();
        let mut stages: BTreeMap<String, (u128, u128, u128)> = BTreeMap::new();
        for profile in profiles {
            let rows = items(&profile["rows"]);
            for row in rows {
                let stage = row["stage"].as_str().ok_or("Stage missing")?;
                let total = stages.entry(stage.to_owned()).or_default();
                total.0 += row["calls"].as_u64().ok_or("Calls missing")? as u128;
                total.1 += row["active_ns"].as_u64().ok_or("Active time missing")? as u128;
                total.2 += row["self_ns"].as_u64().ok_or("Self time missing")? as u128;
                if stage == "processor" {
                    let mut parent = row["parent"].as_u64().map(|value| value as usize);
                    while let Some(index) = parent {
                        check(index < rows.len(), "resource")?;
                        if rows[index]["stage"] == "process" {
                            break;
                        }
                        parent = rows[index]["parent"].as_u64().map(|value| value as usize);
                    }
                    let process = parent.and_then(|index| rows[index]["index"].as_u64());
                    let kind = rows
                        .iter()
                        .find(|child| child["parent"] == row["id"])
                        .and_then(|row| row["stage"].as_str())
                        .unwrap_or("skipped");
                    let total = processors
                        .entry((process, row["index"].as_u64(), kind.to_owned()))
                        .or_default();
                    total.0 += row["wall_ns"].as_u64().ok_or("Wall time missing")? as u128;
                    total.1 += row["active_ns"].as_u64().ok_or("Active time missing")? as u128;
                }
            }
        }
        let scale = profiles.len() as f64 * 1e6;
        for ((process, index, kind), (wall, active)) in processors {
            text += &format!(
                "| {process:?} | {index:?} | {kind} | {:.3} | {:.3} |\n",
                wall as f64 / scale,
                active as f64 / scale
            );
        }
        text += "\n| Stage | Mean calls | Mean active ms | Mean self ms |\n|---|---:|---:|---:|\n";
        let mut stages = stages.into_iter().collect::<Vec<_>>();
        stages.sort_by_key(|(_, (_, _, own))| std::cmp::Reverse(*own));
        for (stage, (calls, active, own)) in stages.into_iter().take(20) {
            text += &format!(
                "| {stage} | {:.1} | {:.3} | {:.3} |\n",
                calls as f64 / profiles.len() as f64,
                active as f64 / scale,
                own as f64 / scale
            );
        }
    }
    Ok(text)
}
pub fn publish(report: &mut Value, output: &Path) -> Result {
    public(report, &models(report))?;
    std::fs::create_dir_all(output)?;
    report["estimates"] = estimates(items(&report["samples"]))?;
    let mut text = format!(
        "# Supplied subscription: original / Rust\n\nStatus: **{}**. Linux x64 GitHub-hosted runners.\n\nOriginal: `{ORIGINAL}`; Rust: `{RUST}`. The harness revision is recorded separately.\n\nThe unchanged file is imported using the official frontend file picker. Twenty-two decoded source bodies are captured once before preprocessing and replayed in order, retaining the local source. The five-site stress variant removes maintenance caps; filtering, retries, timeouts, scripts, caches and snapshot publication remain active.\n\n| Case | Clients | Backend | Passed/measured blocks | Requests | Median ms | P95 ms | RPS | CPU s/request | Peak RSS MiB | OS lifetime peak MiB |\n|---|---:|---|---:|---:|---:|---:|---:|---:|---:|---:|\n",
        report["status"].as_str().ok_or("Report status missing")?
    );
    let rows = items(&report["samples"]);
    for case in CASES {
        for clients in [1, 2, 4, 8] {
            for kind in COMPARISON_KINDS {
                let group = rows
                    .iter()
                    .filter(|row| {
                        row["case"] == case
                            && row["kind"] == kind
                            && row["concurrency"] == clients
                            && row.get("elapsed_s").is_some()
                    })
                    .collect::<Vec<_>>();
                if group.is_empty() {
                    continue;
                }
                let latencies = group
                    .iter()
                    .flat_map(|row| items(&row["latencies_s"]))
                    .map(|value| value.as_f64().ok_or("Latency missing"))
                    .collect::<std::result::Result<Vec<_>, _>>()?;
                if latencies.is_empty() {
                    continue;
                }
                let dist = distribution(&latencies)?;
                let sum = |key: &str| {
                    group
                        .iter()
                        .map(|row| row[key].as_f64().unwrap_or(0.0))
                        .sum::<f64>()
                };
                let peak = |key: &str| {
                    group
                        .iter()
                        .map(|row| row[key].as_u64().unwrap_or(0))
                        .max()
                        .unwrap_or(0) as f64
                        / 1048576.0
                };
                let p95 = dist["p95"]
                    .as_f64()
                    .map(|value| format!("{:.2}", value * 1000.0))
                    .unwrap_or("insufficient n".to_owned());
                text += &format!(
                    "| {case} | {clients} | {kind} | {}/{} | {} | {:.2} | {p95} | {:.2} | {:.5} | {:.1} | {:.1} |\n",
                    group.iter().filter(|row| row["status"] == "passed").count(),
                    group.len(),
                    latencies.len(),
                    dist["median"].as_f64().unwrap() * 1000.0,
                    latencies.len() as f64 / sum("elapsed_s"),
                    sum("cpu_s") / latencies.len() as f64,
                    peak("rss_peak_bytes"),
                    peak("os_lifetime_peak_bytes")
                );
            }
        }
    }
    text += "\nFile import: five fresh states per backend; UI driver wall time and HAR API time are separate.\n";
    for kind in KINDS {
        let imports = items(&report["imports"])
            .iter()
            .filter(|row| row["kind"] == kind)
            .collect::<Vec<_>>();
        if imports.is_empty() {
            continue;
        }
        let measure = |key: &str| -> Result<f64> {
            median(
                &imports
                    .iter()
                    .map(|row| row[key].as_f64().ok_or("Import duration missing"))
                    .collect::<std::result::Result<Vec<_>, _>>()?,
            )
        };
        text += &format!(
            "- {kind}: {} imports; startup median {:.2} ms; API median {:.2} ms; UI wall median {:.2} ms; {} restart passes.\n",
            imports.len(),
            measure("startup_s")? * 1000.0,
            measure("api_import_s")? * 1000.0,
            measure("ui_import_wall_s")? * 1000.0,
            imports.iter().filter(|row| row["restart"] == true).count()
        );
    }
    text += "\nPaired block estimates: Node/Rust latency (>1 favors Rust); pressure uses Node/Rust RPS (<1 favors Rust). Only equivalent, correct pairs enter estimates.\n";
    for estimate in items(&report["estimates"]) {
        text += &format!(
            "- {} clients={}: {:.3} [{:.3}, {:.3}], {} paired blocks.\n",
            estimate["case"].as_str().unwrap(),
            estimate["concurrency"],
            estimate["node_over_rust"].as_f64().unwrap(),
            estimate["bootstrap_low"].as_f64().unwrap(),
            estimate["bootstrap_high"].as_f64().unwrap(),
            estimate["pairs"]
        );
    }
    if let Some(capture) = report.get("capture") {
        text += &format!(
            "\nCapture: {}/22 nonempty, {} empty/failed; elapsed {} s.\n",
            capture["nonempty"], capture["empty_or_failed"], capture["elapsed_s"]
        );
    }
    for row in items(&report["replay"]) {
        text += &format!(
            "- Raw replay {}: equal={}, matching {}/{}.\n",
            row["kind"].as_str().unwrap_or("failed"),
            row["equal"],
            row["matching_sources"],
            row["source_count"]
        );
    }
    if report.get("diagnostic").is_some() {
        text += "\nShort diagnostic only; the full matrix was not run.\n\n| Source | Backend | Input bytes | Nodes | Wall s | CPU s | Status |\n|---:|---|---:|---:|---:|---:|---|\n";
        for row in items(&report["direct_sources"]) {
            text += &format!(
                "| {} | {} | {} | {} | {} | {} | {} |\n",
                row["source_index"],
                row["kind"].as_str().unwrap_or("failed"),
                row["input_bytes"],
                row["output_count"],
                row["elapsed_s"],
                row["cpu_s"],
                row["status"].as_str().unwrap_or("failed")
            );
        }
    }
    for row in rows {
        if row["status"] == "failed" {
            text += &format!(
                "\nFAILED {} {} block={} clients={}: {}. Numeric field/type/hash diagnostics remain in metrics.json.\n",
                row["case"].as_str().unwrap_or("failed"),
                row["kind"].as_str().unwrap_or("failed"),
                row["block"],
                row["concurrency"],
                row["failure"].as_str().unwrap_or("unexpected")
            );
        }
        if let Some(work) = row.get("work") {
            text += &format!(
                "\nWork {} {}: selected={}, identities={}, verified={}, probes/unique={}/{}, output/snapshot={}/{}.\n",
                row["case"].as_str().unwrap_or("failed"),
                row["kind"].as_str().unwrap_or("failed"),
                row["counts"]["selected"],
                work["identities"],
                work["verified"],
                row["counts"]["probes"],
                row["counts"]["unique_probes"],
                row["output_count"],
                work["published"]
            );
        }
    }
    text += "\nAttribution and measurement headroom:\n";
    for kind in KINDS {
        for case in ["pressure", "real"] {
            let matches = rows
                .iter()
                .filter(|row| {
                    row["kind"] == kind && row["case"] == case && row.get("elapsed_s").is_some()
                })
                .collect::<Vec<_>>();
            if matches.is_empty() {
                continue;
            }
            let sum = |key: &str| {
                matches
                    .iter()
                    .filter_map(|row| row[key].as_f64())
                    .sum::<f64>()
            };
            let elapsed = sum("elapsed_s");
            text += &format!(
                "- {kind} {case}: backend {:.2} cores, driver {:.2}, fixture {:.2}, sampler {:.3}; detector {:.2} CPU s, peak RSS {:.1} MiB.\n",
                sum("cpu_s") / elapsed,
                sum("driver_cpu_s") / elapsed,
                sum("fixture_cpu_s") / elapsed,
                sum("sampler_cpu_s") / elapsed,
                sum("detector_cpu_s"),
                matches
                    .iter()
                    .filter_map(|row| row["detector_rss_peak_bytes"].as_u64())
                    .max()
                    .unwrap_or(0) as f64
                    / 1048576.0
            );
        }
    }
    if let Some(runner) = report["provenance"].get("runner") {
        text += &format!(
            "\nRunner: {}; logical/affinity CPUs {}/{}, memory {} bytes, cgroup limit {}, CPU quota/period {}/{} us.\n",
            runner["cpu_model"].as_str().unwrap_or("unavailable"),
            runner["logical_cpus"],
            runner["affinity_cpus"],
            runner["memory_bytes"],
            runner["memory_limit_bytes"],
            runner["cpu_quota_us"],
            runner["cpu_period_us"]
        );
    }
    if report.get("workers").is_some() {
        text += &format!(
            "\nParallel workers: {}. Each Node/Rust pair shares one worker; labels do not guarantee identical CPU models or contention.\n",
            items(&report["workers"]).len()
        );
    }
    text += &pipeline_summary(rows)?;
    text += "\nLimits: replay excludes provider transport/decryption and retains normal preprocessing, parsing, conversion, script, cache and state work. Controlled and real detectors are separate. Only top-level _latency is excluded from semantic hashes; parse-only hopping canonicalizes valid random ports inside unchanged ranges. Raw hashes and snapshot timestamps are retained. Warm-up requests are outside measured cells. RSS is sampled every 50 ms with actual intervals; VmHWM is process-lifetime peak. CPU tick granularity, low block counts and hosted-runner contention limit confidence; P95 needs at least 20 samples and no P99 or cross-platform ranking is claimed. Bootstrap uses 2000 deterministic resamples of block log-ratios. Raw subscriptions, credentials, state, HAR and child logs stay in private ephemeral scratch; public output is allowlisted metrics, counts and hashes.\n";
    public(report, &models(report))?;
    write(output.join("metrics.json"), report)?;
    std::fs::write(output.join("summary.md"), &text)?;
    if let Some(path) = std::env::var_os("GITHUB_STEP_SUMMARY") {
        std::fs::write(path, text)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn prepared() -> Value {
        json!({"status":"passed","preparation_complete":true,"input_sha256":INPUT_HASH,"imports":KINDS.into_iter().flat_map(|kind|(0..5).map(move |_|json!({"kind":kind,"restart":true}))).collect::<Vec<_>>(),"provenance":{"harness_sha":"a".repeat(40),"runner":{"cpu_model":"fixture"}},"capture":{"before_preprocessing":true},"script":{"stress_sha256":"b".repeat(64)},"samples":[]})
    }
    fn workers(prepared: &Value) -> Vec<(String, Value)> {
        shards().into_iter().map(|(name,(case,block,clients))|{let mut worker=prepared.clone();worker["replay"]=json!(KINDS.map(|kind|json!({"kind":kind,"equal":true,"source_count":23,"matching_sources":23})));worker["samples"]=json!(KINDS.map(|kind|json!({"kind":kind,"case":case,"block":block,"concurrency":clients,"status":"passed","output_sha256":"c".repeat(64)})));(name,worker)}).collect()
    }
    #[test]
    fn complete_matrix_rejects_partial_duplicate_changed_source_replay_binary_output_and_work()
    -> Result {
        let prepared = prepared();
        let mut complete = workers(&prepared);
        complete.last_mut().unwrap().1["detector"] =
            json!({"version":"1.3.0","assets_sha256":["f".repeat(64)]});
        let report = aggregate(&prepared, &complete, None)?;
        assert_eq!(report["status"], "passed");
        assert_eq!(report["matrix_complete"], true);
        assert_eq!(items(&report["samples"]).len(), 70);
        assert_eq!(items(&report["workers"]).len(), 35);
        assert!(prepared.get("workers").is_none());
        assert_eq!(report["detector"], complete.last().unwrap().1["detector"]);
        for defect in [
            "missing",
            "duplicate",
            "source",
            "replay",
            "binary",
            "output",
            "both-outputs-missing",
            "failed",
            "cell",
            "empty",
        ] {
            let mut broken = complete.clone();
            match defect {
                "missing" => {
                    broken.pop();
                }
                "duplicate" => broken.push(broken[0].clone()),
                "source" => broken[0].1["capture"] = json!({}),
                "replay" => broken[0].1["replay"][1]["matching_sources"] = json!(22),
                "binary" => broken[0].1["provenance"]["harness_sha"] = json!("d".repeat(40)),
                "output" => broken[0].1["samples"][1]["output_sha256"] = json!("d".repeat(64)),
                "both-outputs-missing" => {
                    for cell in broken[0].1["samples"].as_array_mut().unwrap() {
                        cell.as_object_mut().unwrap().remove("output_sha256");
                    }
                }
                "failed" => broken[0].1["samples"][1]["status"] = json!("failed"),
                "cell" => broken[0].1["samples"][1]["block"] = json!(99),
                _ => broken[0].1["samples"] = json!([]),
            }
            assert_eq!(
                aggregate(&prepared, &broken, None)?["status"],
                "failed",
                "{defect}"
            );
        }
        complete[0].1["provenance"]["runner"]["cpu_model"] = json!("another runner");
        assert_eq!(aggregate(&prepared, &complete, None)?["status"], "passed");
        Ok(())
    }
    #[test]
    fn diagnostics_require_every_direct_source_and_full_input_cell() -> Result {
        let prepared = prepared();
        for diagnostic in ["parse", "health"] {
            let mut worker = workers(&prepared)[0].1.clone();
            let cases = if diagnostic == "parse" {
                vec!["parse", "pre_health"]
            } else {
                vec!["pre_health", "cold", "warm", "no_publisher"]
            };
            let kinds = if diagnostic == "parse" {
                KINDS.to_vec()
            } else {
                COMPARISON_KINDS.to_vec()
            };
            worker["samples"]=json!(cases.iter().flat_map(|case|kinds.iter().map(move|kind|json!({"case":case,"block":0,"concurrency":1,"kind":kind,"status":"passed","output_sha256":"c".repeat(64)}))).collect::<Vec<_>>());
            worker["direct_sources"] = json!(
                KINDS
                    .into_iter()
                    .flat_map(|kind| (0..22).map(
                        move |index| json!({"kind":kind,"source_index":index,"status":"passed"})
                    ))
                    .collect::<Vec<_>>()
            );
            let pair = |worker: Value| vec![("diagnostic".to_owned(), worker)];
            assert_eq!(
                aggregate(&prepared, &pair(worker.clone()), Some(diagnostic))?["status"],
                "passed"
            );
            worker["samples"].as_array_mut().unwrap().pop();
            assert_eq!(
                aggregate(&prepared, &pair(worker.clone()), Some(diagnostic))?["status"],
                "failed"
            );
            worker["samples"] = json!([]);
            assert_eq!(
                aggregate(&prepared, &pair(worker), Some(diagnostic))?["status"],
                "failed"
            );
        }
        Ok(())
    }
    #[test]
    fn reruns_select_latest_attempt_per_pair_and_reject_other_runs() -> Result {
        let root = crate::process::scratch("performance-attempts")?;
        for (shard, attempt) in [("parse_0_1", 1), ("parse_0_1", 2), ("warm_0_1", 1)] {
            let path = root.join(format!(
                "subscription-performance-shard-{shard}-123-{attempt}"
            ));
            std::fs::create_dir(&path)?;
            write(path.join("metrics.json"), &json!({"attempt":attempt}))?;
        }
        let selected = latest_workers(&root, "123")?
            .into_iter()
            .collect::<BTreeMap<_, _>>();
        assert_eq!(
            selected,
            BTreeMap::from([
                ("parse_0_1".to_owned(), json!({"attempt":2})),
                ("warm_0_1".to_owned(), json!({"attempt":1}))
            ])
        );
        assert!(latest_workers(&root, "124").is_err());
        let flat = crate::process::scratch("performance-attempts-flat")?;
        let worker = json!({"diagnostic":"parse_diagnostic","samples":[]});
        write(flat.join("metrics.json"), &worker)?;
        assert_eq!(
            latest_workers(&flat, "123")?,
            vec![("diagnostic".to_owned(), worker)]
        );
        Ok(())
    }
    #[test]
    fn failed_correctness_keeps_measurements_but_cannot_produce_speedup_estimates() -> Result {
        let row = json!({"case":"parse","block":0,"concurrency":1,"latencies_s":[0.1],"elapsed_s":0.1,"cpu_s":0.05,"rps":10,"rss_peak_bytes":1000,"os_lifetime_peak_bytes":2000,"output_count":1,"output_sha256":"a".repeat(64)});
        let mut node = row.clone();
        node["kind"] = json!("node");
        node["status"] = json!("passed");
        let mut rust = row;
        rust["kind"] = json!("rust");
        rust["status"] = json!("failed");
        rust["failure"] = json!("output_mismatch");
        let mut report = json!({"status":"failed","imports":[],"samples":[node,rust]});
        let output = crate::process::scratch("performance-failed-publication")?;
        publish(&mut report, &output)?;
        assert_eq!(read(output.join("metrics.json"))?["estimates"], json!([]));
        let text = std::fs::read_to_string(output.join("summary.md"))?;
        assert!(text.contains("| parse | 1 | rust | 0/1 |") && text.contains("FAILED parse rust"));
        Ok(())
    }
}
