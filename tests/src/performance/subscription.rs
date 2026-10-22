use super::*;
use base64::{Engine, engine::general_purpose::STANDARD};
use std::{
    io::{Read, Write},
    path::PathBuf,
    process::Command,
    time::{Duration, Instant},
};
use workload::{Arguments, Owned, Suite};

fn path(name: &str) -> Result<PathBuf> {
    Ok(
        PathBuf::from(std::env::var_os(name).ok_or("Missing required performance path")?)
            .canonicalize()?,
    )
}
pub fn diagnostic() -> Result<Option<String>> {
    let value = std::env::var("PERF_DIAGNOSTIC").unwrap_or_default();
    check(["", "parse", "health"].contains(&value.as_str()), "input")?;
    Ok((!value.is_empty()).then_some(value))
}
pub fn matrix() -> Result {
    crate::actions_only();
    let names = if diagnostic()?.is_some() {
        vec!["diagnostic".to_owned()]
    } else {
        shards().into_iter().map(|(name, _)| name).collect()
    };
    let matrix =
        json!({"include":names.into_iter().map(|name|json!({"shard":name})).collect::<Vec<_>>()});
    let mut output = std::fs::OpenOptions::new()
        .append(true)
        .open(std::env::var_os("GITHUB_OUTPUT").ok_or("Missing matrix output")?)?;
    writeln!(
        output,
        "value={}\nattempt={}",
        String::from_utf8(canonical(&matrix))?,
        std::env::var("GITHUB_RUN_ATTEMPT")?
    )?;
    Ok(())
}
pub fn run(prepare: bool, fixture_binary: &Path) -> Result {
    crate::actions_only();
    check(
        cfg!(all(target_os = "linux", target_arch = "x86_64")) && !cfg!(debug_assertions),
        "source",
    )?;
    let scratch = crate::process::scratch("performance")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&scratch, std::fs::Permissions::from_mode(0o700))?;
    }
    let publish =
        PathBuf::from(std::env::var_os("PERF_PUBLIC").ok_or("Missing public performance output")?);
    std::fs::create_dir(&publish)?;
    let diagnostic = diagnostic()?;
    let profile = std::env::var("PERF_PROFILE_PIPELINE").as_deref() == Ok("1");
    let baseline = std::env::var_os("PERF_BASELINE")
        .map(PathBuf::from)
        .map(std::fs::canonicalize)
        .transpose()?;
    check(
        baseline.is_some() == (diagnostic.as_deref() == Some("health"))
            && (!profile || diagnostic.as_deref() == Some("health")),
        "input",
    )?;
    let args = Arguments {
        original: path("PERF_ORIGINAL")?,
        rust: path("PERF_RUST")?,
        frontend: path("PERF_FRONTEND")?,
        baseline,
        scratch: scratch.clone(),
        publish,
        diagnostic: diagnostic.clone(),
        profile,
    };
    let mut report =
        json!({"status":"running","input_sha256":INPUT_HASH,"imports":[],"samples":[]});
    if let Some(diagnostic) = &diagnostic {
        report["diagnostic"] = json!(format!("{diagnostic}_diagnostic"));
    }
    if profile {
        report["native_pipeline_profile"] = json!(true);
    }
    let mut suite = Suite::new(args, report)?;
    // Each nextest invocation owns this process; suppress private panic values in JUnit.
    std::panic::set_hook(Box::new(|_| {
        eprintln!("Performance invariant failed; private evidence remains on runner")
    }));
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        execute(&mut suite, prepare, fixture_binary)
    }));
    let result = match result {
        Ok(result) => result,
        Err(_) => Err(Box::new(Failure {
            code: "unexpected",
            observations: None,
        }) as Box<dyn std::error::Error + Send + Sync>),
    };
    if let Err(error) = result {
        std::fs::write(
            scratch.join("failure.txt"),
            format!("{error:?}\n{}", std::backtrace::Backtrace::force_capture()),
        )?;
        suite.report["status"] = json!("failed");
        suite
            .report
            .as_object_mut()
            .unwrap()
            .extend(failure(error.as_ref()).as_object().unwrap().clone());
        suite.publish()?;
        return Err(Box::new(Failure {
            code: suite.report["failure"]
                .as_str()
                .and_then(|code| ERRORS.split_whitespace().find(|known| *known == code))
                .unwrap_or("unexpected"),
            observations: None,
        }));
    }
    suite.publish()?;
    check(suite.report["status"] == "passed", "work_incomplete")
}
fn execute(suite: &mut Suite, prepare: bool, fixture_binary: &Path) -> Result {
    suite.report["provenance"] = suite.provenance()?;
    let raw = STANDARD.decode(std::env::var("BENCHMARK_SUBSCRIPTION_B64").unwrap_or_default())?;
    check(digest(&raw) == INPUT_HASH, "input")?;
    let original: Value = serde_json::from_slice(&raw)?;
    check(
        original["process"]
            .as_array()
            .is_some_and(|items| items.len() == 7)
            && original["url"]
                .as_str()
                .is_some_and(|url| url.trim().lines().count() == 22),
        "input",
    )?;
    let input_file = suite.args.scratch.join(INPUT_NAME);
    std::fs::write(&input_file, &raw)?;
    let frozen =
        PathBuf::from(std::env::var_os("PERF_FROZEN").ok_or("Missing sealed capture path")?);
    if prepare {
        std::fs::create_dir(suite.args.scratch.join("mirrors"))?;
        let source = suite.script()?;
        let mut variant = stress_script(&source)?;
        for preserved in [
            "if (shouldRefresh && maxChecks > 0)",
            "attempt <= retries",
            "await mapLimit(batch, concurrency",
            "cache.set(cacheKey",
            "budget_skipped=${skippedBudget}",
        ] {
            check(
                source.contains(preserved) && variant.contains(preserved),
                "script_anchor",
            )?;
        }
        check(
            source
                .lines()
                .zip(variant.lines())
                .filter(|(a, b)| a != b)
                .count()
                == 5,
            "script_anchor",
        )?;
        suite.patch(&source, &variant, "stress.patch")?;
        suite.report["script"] = json!({"blob":SCRIPT_BLOB,"original_sha256":SCRIPT_HASH,"stress_sha256":digest(variant.as_bytes()),"changed_sites":5});
        if suite.args.diagnostic.as_deref() == Some("health") && !suite.args.profile {
            let instrumented = health_script(&variant, true);
            suite.patch(&variant, &instrumented, "health-profile.patch")?;
            suite.report["script"]["profile_sha256"] = json!(digest(instrumented.as_bytes()));
            variant = instrumented;
        }
        std::fs::write(suite.args.scratch.join("mirrors/stress.js"), variant)?;
        suite.publish()?;
        println!("Importing the unchanged file on both backends (five paired observations).");
        suite.imports(&original, &input_file)?;
        println!("Capturing source text before preprocessing through the original backend.");
        suite.capture(&original)?;
    } else {
        let output = suite.directory("capsule-open")?;
        let prepared = capsule::open(
            &suite.node,
            &input_file,
            &frozen,
            &suite.args.scratch,
            &suite.report,
            suite.args.profile,
            &output,
        )?;
        suite.pristine = KINDS
            .into_iter()
            .map(|kind| {
                (
                    kind.to_owned(),
                    suite.args.scratch.join(format!("pristine-{kind}")),
                )
            })
            .collect();
        for key in ["capture", "imports", "script", "chromium"] {
            suite.report[key] = prepared[key].clone();
        }
    }
    let meta_port = crate::server::free_port()?;
    suite.fixture(fixture_binary, meta_port)?;
    suite.replay(&original)?;
    if prepare {
        suite.report["status"] = json!("passed");
        suite.report["preparation_complete"] = json!(true);
        suite.publish()?;
        let output = suite.directory("capsule-seal")?;
        capsule::seal(
            &suite.node,
            &input_file,
            &suite.args.scratch,
            &suite.pristine,
            &suite.report,
            &frozen,
            &output,
        )?;
        return Ok(());
    }
    let diagnostic = suite.args.diagnostic.clone();
    let shard = std::env::var("PERF_SHARD").unwrap_or_default();
    let selected = if diagnostic.is_some() {
        check(shard == "diagnostic", "input")?;
        None
    } else {
        Some(
            shards()
                .into_iter()
                .find(|(name, _)| name == &shard)
                .ok_or("Unknown required performance shard")?
                .1,
        )
    };
    if diagnostic.as_deref() == Some("parse") {
        suite.diagnose_sources()?;
    }
    let cells = match diagnostic.as_deref() {
        Some("health") => vec![
            ("pre_health", 0, 1),
            ("cold", 0, 1),
            ("warm", 0, 1),
            ("no_publisher", 0, 1),
        ],
        Some("parse") => vec![("parse", 0, 1), ("pre_health", 0, 1)],
        _ => vec![selected.ok_or("Performance pair missing")?],
    };
    for (case, block, clients) in cells {
        if case == "real" {
            real_detector(suite, meta_port)?;
        }
        let mut order = if suite.args.baseline.is_some() {
            COMPARISON_KINDS.to_vec()
        } else {
            KINDS.to_vec()
        };
        if block % 2 == 1 {
            order.reverse();
        }
        for kind in order {
            println!(
                "Measuring {case} / {kind}, block {}, clients {clients}.",
                block + 1
            );
            let repeats = (diagnostic.is_some() && case != "cold").then_some(3);
            suite.run_case(kind, case, block, &original, clients, repeats)?;
        }
    }
    let samples = suite.report["samples"]
        .as_array()
        .ok_or("Samples missing")?;
    let direct = suite.report["direct_sources"].as_array();
    let passed = !samples.is_empty()
        && samples.iter().all(|row| row["status"] == "passed")
        && direct.is_none_or(|rows| rows.iter().all(|row| row["status"] == "passed"));
    suite.report["status"] = json!(if passed { "passed" } else { "failed" });
    if diagnostic.is_none() {
        suite.report["shard"] = json!(shard);
        suite.report["matrix_complete"] = json!(false);
    }
    Ok(())
}
fn real_detector(suite: &mut Suite, port: u16) -> Result {
    let directory = suite.directory("http-meta")?;
    std::fs::create_dir(directory.join("meta"))?;
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(90))
        .user_agent("sub-store-nextest-performance")
        .build()?;
    let assets = [
        (
            "https://github.com/xream/http-meta/releases/download/1.3.0/http-meta.bundle.js",
            "912d378dc36ca1939cda079c7b2db4b85ce7445ce4e67119592c0d98192ed03b",
            "http-meta.bundle.js",
        ),
        (
            "https://github.com/xream/http-meta/releases/download/1.3.0/tpl.yaml",
            "bcf79d9de3c72526435e8dc5f2cf2b641ac79c8cb8e593aecf34da2fba744ec4",
            "meta/tpl.yaml",
        ),
        (
            "https://github.com/MetaCubeX/mihomo/releases/download/v1.19.32/mihomo-linux-amd64-compatible-v1.19.32.gz",
            "ba3ce607747a07f948fc35780e108a4a7c7f552a38b9bd4d115f313ebcb89c20",
            "mihomo.gz",
        ),
    ];
    for (url, fingerprint, name) in assets {
        let bytes = workload::fetch_pinned(
            || {
                Ok(client
                    .get(url)
                    .send()?
                    .error_for_status()?
                    .bytes()?
                    .to_vec())
            },
            fingerprint,
            || std::thread::sleep(Duration::from_secs(1)),
        )?;
        std::fs::write(directory.join(name), bytes)?;
    }
    let binary = directory.join("meta/http-meta");
    let mut decompressed = Vec::new();
    flate2::read::GzDecoder::new(std::fs::File::open(directory.join("mihomo.gz"))?)
        .read_to_end(&mut decompressed)?;
    std::fs::write(&binary, decompressed)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o700))?;
    }
    suite.report["detector"] = json!({"version":"1.3.0","mihomo_version":"1.19.32","assets_sha256":assets.map(|(_,hash,_)|hash)});
    let mut environment = suite.environment.clone();
    for (key, value) in [
        ("HOST", "127.0.0.1".into()),
        ("PORT", port.to_string().into()),
        ("META_FOLDER", directory.join("meta").into_os_string()),
        ("META_TEMP_FOLDER", directory.clone().into_os_string()),
        ("META_BINARY_PATH", binary.into_os_string()),
        ("BODY_JSON_LIMIT", "100mb".into()),
    ] {
        crate::server::set(&mut environment, key, value);
    }
    let mut child = Owned::spawn(
        Command::new(&suite.node)
            .arg(directory.join("http-meta.bundle.js"))
            .current_dir(&directory)
            .env_clear()
            .envs(environment),
        &directory,
        "service",
    )?;
    let began = Instant::now();
    let mut ready = false;
    while began.elapsed() < Duration::from_secs(30) && child.process.child.try_wait()?.is_none() {
        if transport::http(
            &format!("http://127.0.0.1:{port}/stats"),
            "POST",
            Some(&json!({})),
        )
        .is_ok()
        {
            ready = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    check(ready, "startup")?;
    suite.meta = Some(child);
    Ok(())
}
pub fn aggregate() -> Result {
    crate::actions_only();
    let prepared = path("PERF_PREPARED")?;
    let workers = path("PERF_WORKERS")?;
    let output = PathBuf::from(std::env::var_os("PERF_PUBLIC").ok_or("Missing aggregate output")?);
    std::fs::create_dir(&output)?;
    let prepared_report = read(prepared.join("metrics.json"))?;
    let workers = report::latest_workers(&workers, &std::env::var("GITHUB_RUN_ID")?)?;
    let mut report = report::aggregate(&prepared_report, &workers, diagnostic()?.as_deref())?;
    for file in std::fs::read_dir(prepared)? {
        let file = file?;
        if file
            .path()
            .extension()
            .is_some_and(|extension| extension == "patch")
        {
            std::fs::copy(file.path(), output.join(file.file_name()))?;
        }
    }
    report::publish(&mut report, &output)?;
    println!(
        "Aggregation {}: workers={}, cells={}, matrix_complete={}",
        report["status"].as_str().unwrap_or("failed"),
        report["workers"].as_array().unwrap().len(),
        report["samples"].as_array().unwrap().len(),
        report["matrix_complete"]
    );
    check(report["status"] == "passed", "work_incomplete")
}
pub fn inspect() -> Result {
    crate::actions_only();
    let report = read(path("PERF_INSPECT")?.join("metrics.json"))?;
    public(&report, &models(&report))?;
    let selected = [
        "status",
        "failure",
        "failure_stack",
        "provenance",
        "capture",
        "replay",
        "estimates",
    ]
    .into_iter()
    .filter_map(|key| report.get(key).map(|value| (key.to_owned(), value.clone())))
    .collect::<serde_json::Map<_, _>>();
    println!("Report {}", serde_json::to_string(&selected)?);
    let samples = report["samples"]
        .as_array()
        .ok_or("Inspected samples missing")?;
    let measured = samples
        .iter()
        .filter(|row| {
            row["latencies_s"]
                .as_array()
                .is_some_and(|values| !values.is_empty())
                && ["elapsed_s", "cpu_s", "rss_peak_bytes", "output_count"]
                    .iter()
                    .all(|key| row.get(key).is_some())
        })
        .collect::<Vec<_>>();
    let groups = measured
        .iter()
        .map(|row| {
            Ok((
                row["case"].as_str().ok_or("Case missing")?,
                row["kind"].as_str().ok_or("Backend missing")?,
                row["concurrency"].as_u64().ok_or("Concurrency missing")?,
            ))
        })
        .collect::<Result<BTreeSet<_>>>()?;
    for (case, kind, clients) in groups {
        let rows = measured
            .iter()
            .copied()
            .filter(|row| {
                row["case"] == case
                    && row["kind"] == kind
                    && row["concurrency"] == clients
                    && row.get("elapsed_s").is_some()
            })
            .collect::<Vec<_>>();
        let times = rows
            .iter()
            .flat_map(|row| row["latencies_s"].as_array().into_iter().flatten())
            .filter_map(Value::as_f64)
            .collect::<Vec<_>>();
        let stats = distribution(&times)?;
        let mut summary = json!({"requests":stats["n"],"median_s":stats["median"],"p95_s":stats["p95"]});
        summary["case"] = json!(case);
        summary["kind"] = json!(kind);
        summary["clients"] = json!(clients);
        summary["blocks"] = json!(rows.len());
        summary["passed"] = json!(rows.iter().filter(|row| row["status"] == "passed").count());
        summary["rps"] = json!(
            times.len() as f64
                / rows
                    .iter()
                    .filter_map(|row| row["elapsed_s"].as_f64())
                    .sum::<f64>()
        );
        summary["cpu_s_per_request"] = json!(
            rows.iter()
                .filter_map(|row| row["cpu_s"].as_f64())
                .sum::<f64>()
                / times.len() as f64
        );
        for (source, output) in [
            ("rss_peak_bytes", "rss_peak_mib"),
            ("rss_end_bytes", "rss_end_mib"),
            ("rss_settled_bytes", "rss_settled_mib"),
        ] {
            let peak = rows
                .iter()
                .map(|row| row[source].as_u64().ok_or("RSS bytes missing"))
                .collect::<std::result::Result<Vec<_>, _>>()?
                .into_iter()
                .max()
                .ok_or("RSS samples missing")?;
            summary[output] = json!(peak as f64 / 1048576.0);
        }
        summary["output_counts"] = json!(
            rows.iter()
                .filter_map(|row| row["output_count"].as_u64())
                .collect::<BTreeSet<_>>()
        );
        public(&summary, &BTreeSet::new())?;
        println!("Aggregate {}", serde_json::to_string(&summary)?);
    }
    for row in samples {
        if (row["block"] != 0 || row["concurrency"] != 1) && row["status"] != "failed" {
            continue;
        }
        let mut selected = [
            "kind",
            "case",
            "status",
            "failure",
            "failure_observations",
            "counts",
            "warmup_counts",
            "output_count",
            "health_records",
            "health_verified",
            "work",
            "health_profile",
            "profiler_enabled",
            "pipeline_profiles",
        ]
        .into_iter()
        .filter_map(|key| row.get(key).map(|value| (key.to_owned(), value.clone())))
        .collect::<serde_json::Map<_, _>>();
        selected.insert(
            "health_counts".to_owned(),
            json!(
                row["health_counts"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .take(2)
                    .collect::<Vec<_>>()
            ),
        );
        println!("{}", serde_json::to_string(&selected)?);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn health_wrappers_preserve_results_errors_receiver_identity_and_restore_helpers() -> Result {
        let root = crate::process::scratch("performance-health-wrapper")?;
        let source = "async function operator(value){if(ProxyUtils.produce(value)!==value)throw Error('identity');ProxyUtils.hex_md5('public');scriptResourceCache.get('public');scriptResourceCache.set('public',1);if(value.fail)throw Error('original failure');return value;}";
        assert_eq!(health_script(source, false), source);
        let fixture = "const assert=require('node:assert/strict');let profile;const ProxyUtils={produce(value){assert.equal(this,ProxyUtils);return value;},hex_md5(){return 'x';}};const scriptResourceCache={get(){},set(){}};const original={...ProxyUtils};const $substore={write(value,key){assert.equal(key,'#benchmark-health-profile');profile=value;}};";
        let checks = "(async()=>{const value={};assert.equal(await operator(value),value);assert.deepEqual(ProxyUtils,original);assert.equal(profile.md5_calls,1);assert.equal(profile.produce_calls,1);assert.equal(profile.cache_get_calls,1);assert.equal(profile.cache_set_calls,1);await assert.rejects(operator({fail:true}),/original failure/);assert.deepEqual(ProxyUtils,original);assert(Object.values(profile).every(v=>typeof v==='number'&&Number.isFinite(v)));process.stdout.write(JSON.stringify(profile));})().catch(()=>{process.exitCode=1;});";
        let result = crate::process::checked(
            Command::new(crate::reference::node()).args([
                "-e",
                &format!("{fixture}{}{checks}", health_script(source, true)),
            ]),
            &root,
            "wrapper",
            Duration::from_secs(30),
        )?;
        public(
            &json!({"health_profile":serde_json::from_str::<Value>(&result.stdout)?}),
            &BTreeSet::new(),
        )
    }
}
