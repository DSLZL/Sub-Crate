use super::*;
use single::{BACKENDS, TAIL_CASES, VARIANTS};
use std::path::PathBuf;

pub fn one_metrics(root: &Path) -> Result<PathBuf> {
    fn visit(root: &Path, files: &mut Vec<PathBuf>) -> Result {
        check(!root.is_symlink(), "input")?;
        if root.is_file() {
            if root.file_name().is_some_and(|name| name == "metrics.json") {
                files.push(root.to_owned());
            }
        } else {
            for entry in std::fs::read_dir(root)? {
                visit(&entry?.path(), files)?;
            }
        }
        Ok(())
    }
    let mut files = Vec::new();
    visit(root, &mut files)?;
    check(files.len() == 1, "input")?;
    Ok(files.remove(0))
}
pub fn ns_distribution(samples: &Value) -> Result<Value> {
    let mut values = samples
        .as_array()
        .ok_or("Nanosecond samples missing")?
        .iter()
        .map(|value| {
            value
                .as_u64()
                .filter(|value| *value > 0)
                .ok_or("Nanosecond samples must be positive integers")
        })
        .collect::<std::result::Result<Vec<_>, _>>()?;
    check(!values.is_empty(), "resource")?;
    values.sort();
    let middle = values.len() / 2;
    let median = if values.len().is_multiple_of(2) {
        values[middle - 1] as f64 / 2.0 + values[middle] as f64 / 2.0
    } else {
        values[middle] as f64
    };
    Ok(
        json!({"requests":values.len(),"p50_ns":median,"p95_ns":values[(values.len()*95).div_ceil(100)-1],"max_ns":values[values.len()-1]}),
    )
}
fn append(lines: &mut Vec<String>, prefix: &str, value: Value) -> Result {
    lines.push(format!("{prefix} {}", serde_json::to_string(&value)?));
    Ok(())
}
fn stage_sum(event: &Value, stage: &str, key: &str) -> u64 {
    event["rows"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|row| row["stage"] == stage)
        .filter_map(|row| row[key].as_u64())
        .sum()
}
pub fn tail_summary(report: &Value) -> Result<String> {
    let mut lines=vec!["HTTP tail diagnostic; durations are ns. Profile-on requests are separate from timed batches.".to_owned()];
    append(
        &mut lines,
        "TAIL_SOURCE",
        Value::Object(
            [
                "original",
                "rust",
                "rust_baseline",
                "fixture_sha256",
                "binary_sha256",
                "status",
            ]
            .map(|key| (key.to_owned(), report[key].clone()))
            .into_iter()
            .collect(),
        ),
    )?;
    let rows = report["http"]
        .as_array()
        .ok_or("HTTP measurements missing")?
        .iter()
        .filter(|row| {
            row["case"]
                .as_str()
                .is_some_and(|case| TAIL_CASES.contains(&case))
        })
        .collect::<Vec<_>>();
    let groups = rows
        .iter()
        .map(|row| {
            Ok((
                row["case"].as_str().ok_or("Tail case missing")?,
                row["mode"].as_str().ok_or("Tail connection mode missing")?,
                row["variant"].as_str().ok_or("Tail backend missing")?,
            ))
        })
        .collect::<Result<BTreeSet<_>>>()?;
    for (case, mode, variant) in groups {
        let selected = rows
            .iter()
            .filter(|row| row["case"] == case && row["mode"] == mode && row["variant"] == variant)
            .collect::<Vec<_>>();
        let pooled = selected
            .iter()
            .flat_map(|row| row["ns"].as_array().into_iter().flatten().cloned())
            .collect::<Vec<_>>();
        let mut total = ns_distribution(&json!(pooled))?;
        total.as_object_mut().unwrap().extend(
            json!({"case":case,"mode":mode,"variant":variant,"scope":"all"})
                .as_object()
                .unwrap()
                .clone(),
        );
        append(&mut lines, "TAIL_HTTP", total)?;
        for row in selected {
            let mut result = ns_distribution(&row["ns"])?;
            let mut slowest = row["ns"]
                .as_array()
                .unwrap()
                .iter()
                .enumerate()
                .map(|(index, value)| (index, value.as_u64().unwrap()))
                .collect::<Vec<_>>();
            slowest.sort_by_key(|(_, value)| std::cmp::Reverse(*value));
            slowest.truncate(5);
            result.as_object_mut().unwrap().extend(json!({"case":case,"mode":mode,"variant":variant,"scope":"block","round":row.get("round").unwrap_or(&json!(0)),"block":row["block"],"slowest":slowest}).as_object().unwrap().clone());
            append(&mut lines, "TAIL_HTTP", result)?;
        }
    }
    for group in report["profiles"].as_array().into_iter().flatten() {
        if !group["case"]
            .as_str()
            .is_some_and(|case| TAIL_CASES.contains(&case))
        {
            continue;
        }
        let events = group["events"].as_array().ok_or("Profile events missing")?;
        let server = group["server_ns"]
            .as_array()
            .ok_or("Server profile times missing")?;
        let client = group["client_ns"]
            .as_array()
            .ok_or("Client profile times missing")?;
        check(
            events.len() == server.len() && events.len() == client.len() && !events.is_empty(),
            "resource",
        )?;
        let mut pairs = Vec::new();
        for (index, event) in events.iter().enumerate() {
            let mut pair = json!({"index":index,"client_ns":client[index],"server_ns":server[index],"worker_ns":event["wall_ns"]});
            for stage in [
                "worker_queue",
                "conversion_miss",
                "conversion_reuse",
                "input_decode",
                "parse_native",
                "produce",
                "output_encode",
                "conversion_cleanup",
                "runtime_teardown",
            ] {
                pair[stage] = json!(stage_sum(event, stage, "wall_ns"));
            }
            pairs.push(pair);
        }
        let mut slowest = pairs.iter().collect::<Vec<_>>();
        slowest.sort_by_key(|pair| std::cmp::Reverse(pair["client_ns"].as_u64().unwrap_or(0)));
        for pair in slowest.into_iter().take(10) {
            let mut pair = pair.clone();
            pair["case"] = group["case"].clone();
            append(&mut lines, "TAIL_PROFILE_SLOW", pair)?;
        }
        for key in [
            "client_ns",
            "server_ns",
            "worker_ns",
            "worker_queue",
            "parse_native",
            "produce",
        ] {
            let mut result = ns_distribution(&json!(
                pairs
                    .iter()
                    .map(|pair| pair[key].clone())
                    .collect::<Vec<_>>()
            ))?;
            result["case"] = group["case"].clone();
            result["stage"] = json!(key);
            append(&mut lines, "TAIL_PROFILE", result)?;
        }
        let mut lifecycle = json!({"case":group["case"]});
        for stage in [
            "conversion_miss",
            "conversion_reuse",
            "conversion_retain",
            "runtime_teardown",
        ] {
            lifecycle[stage] = json!(
                events
                    .iter()
                    .map(|event| stage_sum(event, stage, "calls"))
                    .sum::<u64>()
            );
        }
        append(&mut lines, "TAIL_LIFECYCLE", lifecycle)?;
    }
    Ok(lines.join("\n") + "\n")
}
fn http_samples(report: &Value, case: &Value, kind: &str, mode: &str) -> Result<Vec<f64>> {
    let values = report["http"]
        .as_array()
        .ok_or("HTTP measurements missing")?
        .iter()
        .filter(|row| &row["case"] == case && row["variant"] == kind && row["mode"] == mode)
        .flat_map(|row| row["ns"].as_array().into_iter().flatten())
        .map(|value| {
            value
                .as_u64()
                .filter(|value| *value > 0)
                .map(|value| value as f64)
                .ok_or("HTTP duration must be positive integer")
        })
        .collect::<std::result::Result<Vec<_>, _>>()?;
    check(!values.is_empty(), "work_incomplete")?;
    Ok(values)
}
pub fn ratios(report: &Value, mode: &str) -> Result<Vec<f64>> {
    report["cases"]
        .as_array()
        .ok_or("Cases missing")?
        .iter()
        .map(|case| {
            Ok(median(&http_samples(report, case, "rust", mode)?)?
                / median(&http_samples(report, case, "rust_baseline", mode)?)?)
        })
        .collect()
}
fn row(lines: &mut Vec<String>, labels: &[&str], values: &[f64]) {
    lines.push(format!(
        "| {} | {} |",
        labels.join(" | "),
        values
            .iter()
            .map(|value| format!("{value:.3}"))
            .collect::<Vec<_>>()
            .join(" | ")
    ));
}
pub fn summary(report: &Value) -> Result<String> {
    let cases = report["cases"].as_array().ok_or("Cases missing")?;
    let groups = report["profiles"].as_array().ok_or("Profiles missing")?;
    let mut lines=vec!["Single-node parsing; one node per function call or HTTP request.".to_owned(),"Pure: median of seven block means, us/call. HTTP: median of 210 requests, ms/request.".to_owned(),"Equivalent original/newline inputs alternate, avoiding last-input preprocessing cache hits.".to_owned(),"HTTP includes JSON production and transport, with empty application state; connection modes and profiler-on measurements remain separate.".to_owned(),format!("Coverage: {} fixtures, all {} pinned Clash node types and Surge external. This measures configuration conversion, not protocol network traffic.",cases.len(),report["clash_types"].as_array().ok_or("Protocol inventory missing")?.len()),"".to_owned(),"| Case | Protocol | Input | Node pure us | Previous Rust reused us | Previous Rust fresh us | Rust reused us | Rust fresh us | Node HTTP ms | Previous Rust HTTP ms | Rust HTTP ms |".to_owned(),"|---|---|---|---:|---:|---:|---:|---:|---:|---:|---:|".to_owned()];
    for case in cases {
        let id = case.as_str().ok_or("Case id missing")?;
        let mut values = Vec::new();
        for variant in VARIANTS {
            let means = report["pure"]
                .as_array()
                .ok_or("Pure measurements missing")?
                .iter()
                .filter(|row| row["case"] == *case && row["variant"] == variant)
                .map(|row| {
                    Ok(
                        row["elapsed_ns"].as_u64().ok_or("Pure duration missing")? as f64
                            / row["iterations"]
                                .as_u64()
                                .filter(|value| *value > 0)
                                .ok_or("Pure iterations missing")?
                                as f64
                            / 1000.0,
                    )
                })
                .collect::<Result<Vec<_>>>()?;
            check(means.len() == 7, "work_incomplete")?;
            values.push(median(&means)?);
        }
        for kind in BACKENDS {
            values.push(median(&http_samples(report, case, kind, "new_connection")?)? / 1e6);
        }
        let info = &report["case_info"][id];
        row(
            &mut lines,
            &[
                id,
                info["protocol"].as_str().unwrap(),
                info["format"].as_str().unwrap(),
            ],
            &values,
        );
    }
    lines.extend(["".to_owned(),"Reused HTTP connections, seven blocks of 30 requests per backend; socket identity is fixed and reconnection fails.".to_owned(),"| Case | Node HTTP ms | Previous Rust HTTP ms | Rust HTTP ms |".to_owned(),"|---|---:|---:|---:|".to_owned()]);
    for case in cases {
        let values = BACKENDS
            .into_iter()
            .map(|kind| Ok(median(&http_samples(report, case, kind, "keep_alive")?)? / 1e6))
            .collect::<Result<Vec<_>>>()?;
        row(&mut lines, &[case.as_str().unwrap()], &values);
    }
    lines.extend(["".to_owned(),"Full HTTP P95 (ms), in the P50 tables' backend order.".to_owned(),"| Case | Node new | Previous new | Rust new | Node keep-alive | Previous keep-alive | Rust keep-alive |".to_owned(),"|---|---:|---:|---:|---:|---:|---:|".to_owned()]);
    for case in cases {
        let mut values = Vec::new();
        for mode in ["new_connection", "keep_alive"] {
            for kind in BACKENDS {
                let mut samples = http_samples(report, case, kind, mode)?;
                samples.sort_by(f64::total_cmp);
                values.push(samples[(samples.len() * 95).div_ceil(100) - 1] / 1e6);
            }
        }
        row(&mut lines, &[case.as_str().unwrap()], &values);
    }
    for mode in ["new_connection", "keep_alive"] {
        let values = ratios(report, mode)?;
        lines.push(format!(
            "Median per-fixture candidate/previous P50 ratio, {mode}: {:.4}; improved {}/{}.",
            median(&values)?,
            values.iter().filter(|value| **value < 1.0).count(),
            values.len()
        ));
    }
    lines.extend(["".to_owned(),"First conversion after readiness: one observation per backend, not a cold-start distribution.".to_owned(),"| Backend | Case | First HTTP ms |".to_owned(),"|---|---|---:|".to_owned()]);
    for value in report["cold"]
        .as_array()
        .ok_or("Cold observations missing")?
    {
        row(
            &mut lines,
            &[
                value["variant"].as_str().unwrap(),
                value["case"].as_str().unwrap(),
            ],
            &[value["ns"].as_f64().unwrap() / 1e6],
        );
    }
    lines.extend(["".to_owned(),"Separate load: 384 changing requests across 48 fixtures per block, three blocks, new connections; RSS is retained process memory, not a JS heap leak measurement.".to_owned(),"| Backend | Clients | Block | P50 ms | P95 ms | Requests/s | CPU ms/request | End RSS MiB | Settled RSS MiB |".to_owned(),"|---|---:|---:|---:|---:|---:|---:|---:|---:|".to_owned()]);
    for value in report["concurrency"]
        .as_array()
        .ok_or("Load measurements missing")?
    {
        let stats = ns_distribution(&value["ns"])?;
        row(
            &mut lines,
            &[
                value["variant"].as_str().unwrap(),
                &value["clients"].to_string(),
                &value["block"].to_string(),
            ],
            &[
                stats["p50_ns"].as_f64().unwrap() / 1e6,
                stats["p95_ns"].as_f64().unwrap() / 1e6,
                value["rps"].as_f64().unwrap(),
                value["cpu_s_per_request"].as_f64().unwrap() * 1000.0,
                value["rss_end_bytes"].as_f64().unwrap() / 1048576.0,
                value["rss_settled_bytes"].as_f64().unwrap() / 1048576.0,
            ],
        );
    }
    lines.extend(["".to_owned(),"Separate Rust profiles: median active ms over seven requests; parent/child intervals overlap, and active time is not OS CPU time.".to_owned(),"| Stage | SS URI | Surge SS | QX SS |".to_owned(),"|---|---:|---:|---:|".to_owned()]);
    for stage in [
        "conversion_miss",
        "conversion_reuse",
        "conversion_cleanup",
        "conversion_retain",
        "executor_setup",
        "state_snapshot",
        "runtime_setup",
        "context_setup",
        "job",
        "bootstrap",
        "bootstrap_values",
        "bootstrap_load",
        "bootstrap_eval",
        "bootstrap_state_encode",
        "bootstrap_state_parse",
        "parse_runtime",
        "parse_context_init",
        "parse_utilities_init",
        "parse_scalar_probe",
        "parse_native",
        "value_project",
        "value_decode",
        "produce",
        "output_encode",
        "runtime_teardown",
    ] {
        let mut values = Vec::new();
        for case in ["ss_uri", "surge_ss", "qx_ss"] {
            let events = groups
                .iter()
                .find(|group| group["case"] == case)
                .ok_or("Required profiler fixture missing")?["events"]
                .as_array()
                .unwrap();
            values.push(median(
                &events
                    .iter()
                    .map(|event| stage_sum(event, stage, "active_ns") as f64 / 1e6)
                    .collect::<Vec<_>>(),
            )?);
        }
        row(&mut lines, &[stage], &values);
    }
    lines.extend(["".to_owned(),"Complete request components (separate profiler-on medians, us). Worker lifetime includes queueing and cleanup; server time ends before socket transmission. Intervals overlap: do not add them or subtract independent medians to infer exact overhead.".to_owned(),"| Case | Queue | Worker lifetime | Bootstrap | Native parse | Produce | Teardown | Server response ready | Client HTTP |".to_owned(),"|---|---:|---:|---:|---:|---:|---:|---:|---:|".to_owned()]);
    for group in groups {
        let events = group["events"].as_array().unwrap();
        let mut values = Vec::new();
        for stage in [
            "worker_queue",
            "",
            "bootstrap",
            "parse_native",
            "produce",
            "runtime_teardown",
        ] {
            values.push(
                median(
                    &events
                        .iter()
                        .map(|event| {
                            if stage.is_empty() {
                                event["wall_ns"].as_u64().unwrap() as f64
                            } else {
                                stage_sum(event, stage, "wall_ns") as f64
                            }
                        })
                        .collect::<Vec<_>>(),
                )? / 1000.0,
            );
        }
        for key in ["server_ns", "client_ns"] {
            values.push(
                median(
                    &group[key]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|value| value.as_f64().unwrap())
                        .collect::<Vec<_>>(),
                )? / 1000.0,
            );
        }
        row(&mut lines, &[group["case"].as_str().unwrap()], &values);
    }
    lines.extend(["".to_owned(),"Generic-parser setup (separate profiles).".to_owned(),"| Format | Median context calls | Context active ms | Median library calls | Library active ms |".to_owned(),"|---|---:|---:|---:|---:|".to_owned()]);
    for group in groups {
        let events = group["events"].as_array().unwrap();
        let mut values = Vec::new();
        for stage in ["parse_context_init", "parse_utilities_init"] {
            values.push(median(
                &events
                    .iter()
                    .map(|event| stage_sum(event, stage, "calls") as f64)
                    .collect::<Vec<_>>(),
            )?);
            values.push(median(
                &events
                    .iter()
                    .map(|event| stage_sum(event, stage, "active_ns") as f64 / 1e6)
                    .collect::<Vec<_>>(),
            )?);
        }
        row(&mut lines, &[group["case"].as_str().unwrap()], &values);
    }
    lines.extend([
        "".to_owned(),
        "Actual lifecycle counts over the seven profiled requests per fixture.".to_owned(),
        "| Case | Created | Reused | Retained | Destroyed |".to_owned(),
        "|---|---:|---:|---:|---:|".to_owned(),
    ]);
    for group in groups {
        let values = [
            "conversion_miss",
            "conversion_reuse",
            "conversion_retain",
            "runtime_teardown",
        ]
        .map(|stage| {
            group["events"]
                .as_array()
                .unwrap()
                .iter()
                .map(|event| stage_sum(event, stage, "calls"))
                .sum::<u64>() as f64
        });
        row(&mut lines, &[group["case"].as_str().unwrap()], &values);
    }
    let cold = report
        .get("cold_profile")
        .ok_or("First-conversion profile missing")?;
    append(
        &mut lines,
        "Separate first-conversion profile, us:",
        Value::Object(
            [
                "runtime_setup",
                "context_setup",
                "bootstrap",
                "execute",
                "conversion_cleanup",
            ]
            .map(|stage| {
                (
                    stage.to_owned(),
                    json!(stage_sum(cold, stage, "wall_ns") as f64 / 1000.0),
                )
            })
            .into_iter()
            .collect(),
        ),
    )?;
    Ok(lines.join("\n") + "\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn inspection_accepts_nested_artifacts_but_rejects_ambiguous_metrics() -> Result {
        let root = crate::process::scratch("single-inspection")?;
        std::fs::create_dir(root.join("results"))?;
        write(root.join("results/metrics.json"), &json!({"http":[]}))?;
        assert!(tail_summary(&read(one_metrics(&root)?)?)?.contains("TAIL_SOURCE "));
        write(root.join("metrics.json"), &json!({"http":[]}))?;
        assert!(one_metrics(&root).is_err());
        Ok(())
    }
    #[test]
    fn tail_statistics_pool_requests_without_losing_round_or_sample_identity() -> Result {
        let rows=[1000,2000].into_iter().enumerate().map(|(index,value)|json!({"case":"clash_hysteria2","mode":"keep_alive","variant":"rust","block":0,"round":index,"ns":std::iter::repeat_n(value,95).chain(std::iter::repeat_n(value*10,5)).collect::<Vec<_>>()})).collect::<Vec<_>>();
        let text = tail_summary(&json!({"http":rows}))?;
        let values = text
            .lines()
            .filter_map(|line| line.strip_prefix("TAIL_HTTP "))
            .map(serde_json::from_str::<Value>)
            .collect::<std::result::Result<Vec<_>, _>>()?;
        assert_eq!(
            (values[0]["requests"].clone(), values[0]["p95_ns"].clone()),
            (json!(200), json!(2000))
        );
        assert_eq!(values[1]["round"], 0);
        assert_eq!(values[2]["round"], 1);
        assert_eq!(values[2]["slowest"][0], json!([95, 20000]));
        assert_eq!(values[1]["p95_ns"], 1000);
        assert_eq!(values[2]["p95_ns"], 2000);
        for bad in [json!([]), json!([true]), json!([0]), json!([-1])] {
            assert!(ns_distribution(&bad).is_err());
        }
        Ok(())
    }
    #[test]
    fn slow_client_samples_retain_their_server_worker_and_stage_pairing() -> Result {
        let events = [(50, 1), (500, 400)].map(|(wall, queue)| {
            let rows = [("worker_queue", queue), ("parse_native", 10), ("produce", 5),
                ("conversion_reuse", 1), ("conversion_retain", 1)]
                .map(|(stage, time)| json!({"stage":stage,"wall_ns":time,"calls":1}));
            json!({"wall_ns":wall,"rows":rows})
        });
        let group = json!({"case":"clash_hysteria2","events":events,"client_ns":[100,1000],"server_ns":[90,900]});
        let text = tail_summary(&json!({"http":[],"profiles":[group]}))?;
        let slow = text
            .lines()
            .filter_map(|line| line.strip_prefix("TAIL_PROFILE_SLOW "))
            .map(serde_json::from_str::<Value>)
            .collect::<std::result::Result<Vec<_>, _>>()?;
        assert_eq!(
            (
                slow[0]["index"].clone(),
                slow[0]["client_ns"].clone(),
                slow[0]["worker_ns"].clone(),
                slow[0]["worker_queue"].clone()
            ),
            (json!(1), json!(1000), json!(500), json!(400))
        );
        let mut broken = group;
        broken["server_ns"] = json!([90]);
        assert!(tail_summary(&json!({"http":[],"profiles":[broken]})).is_err());
        Ok(())
    }
}
