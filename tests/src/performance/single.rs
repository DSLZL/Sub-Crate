use super::*;
use std::{
    io::{BufRead, BufReader, Write},
    path::PathBuf,
    process::{Child, ChildStdin, Command, Stdio},
    sync::{
        atomic::{AtomicUsize, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};
use workload::{Arguments, Suite};

pub const BLOCKS: usize = 7;
pub const TAIL_CASES: [&str; 3] = ["clash_hysteria2", "hysteria2", "clash_json"];
pub const VARIANTS: [&str; 5] = [
    "node",
    "rust_baseline_warm_parser",
    "rust_baseline_fresh_parser",
    "rust_warm_parser",
    "rust_fresh_parser",
];
pub const BACKENDS: [&str; 3] = ["node", "rust_baseline", "rust"];
const BASELINE: &str = "9c19111bae2af2654caa339151e4eabd12b586bb";
const ENDPOINT: &str = "/backend/api/proxy/parse";
type ProfileEvents = (Vec<Value>, Vec<u64>);

struct Worker {
    child: Child,
    input: Option<ChildStdin>,
    responses: mpsc::Receiver<Result<Value>>,
    reader: Option<thread::JoinHandle<()>>,
}
impl Worker {
    fn new(command: &mut Command, root: &Path, kind: &str) -> Result<Self> {
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::from(
                std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(root.join(format!("pure-{kind}.log")))?,
            ))
            .spawn()?;
        let input = child.stdin.take();
        let output = child.stdout.take().ok_or("Parser stdout missing")?;
        let (tx, responses) = mpsc::channel();
        let reader = thread::spawn(move || {
            let mut output = BufReader::new(output);
            loop {
                let mut line = Vec::new();
                let result = (|| -> Result<Option<Value>> {
                    let count = output.read_until(b'\n', &mut line)?;
                    check(count > 0 && count <= 4 * 1024 * 1024, "resource")?;
                    let at = line.windows(12).position(|part| part == b"SINGLE_NODE ");
                    Ok(at
                        .map(|at| serde_json::from_slice(&line[at + 12..]))
                        .transpose()?)
                })();
                match result {
                    Ok(Some(value)) => {
                        if tx.send(Ok(value)).is_err() {
                            break;
                        }
                    }
                    Ok(None) => {}
                    Err(error) => {
                        let _ = tx.send(Err(error));
                        break;
                    }
                }
            }
        });
        let worker = Self {
            child,
            input,
            responses,
            reader: Some(reader),
        };
        write(
            root.join(format!("pure-{kind}.ownership.json")),
            &json!({"pid":worker.child.id(),"direct_reason":"Interactive parser protocol requires owned stdin/stdout and an actual child watchdog"}),
        )?;
        Ok(worker)
    }
    fn call(&mut self, index: usize, iterations: u64, warmup: u64, fresh: bool) -> Result<Value> {
        let input = self.input.as_mut().ok_or("Parser stdin closed")?;
        input.write_all(&canonical(
            &json!({"case":index,"iterations":iterations,"warmup":warmup,"fresh":fresh}),
        ))?;
        input.write_all(b"\n")?;
        input.flush()?;
        let row = self
            .responses
            .recv_timeout(Duration::from_secs(120))
            .map_err(|_| "Parser worker exited or exceeded watchdog")??;
        check(
            row["iterations"].as_u64() == Some(iterations)
                && row["checksum"].as_u64() == Some(iterations)
                && row["elapsed_ns"].as_u64().is_some_and(|value| value > 0),
            "resource",
        )?;
        check(
            row["outputs"]
                .as_array()
                .is_some_and(|rows| rows.len() == 2)
                && row["outputs"][0]
                    .as_array()
                    .is_some_and(|rows| rows.len() == 1)
                && row["outputs"][0] == row["outputs"][1],
            "output_mismatch",
        )?;
        Ok(row)
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        self.input.take();
        let started = Instant::now();
        while self.child.try_wait().ok().flatten().is_none()
            && started.elapsed() < Duration::from_secs(5)
        {
            thread::sleep(Duration::from_millis(10));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
}

pub fn build_binary(path: &Path) -> Result<PathBuf> {
    let mut binaries = Vec::new();
    for line in std::fs::read_to_string(path)?.lines() {
        let row: Value = serde_json::from_str(line)?;
        if row["reason"] == "compiler-artifact"
            && row["profile"]["test"] == true
            && let Some(executable) = row["executable"].as_str()
        {
            binaries.push(PathBuf::from(executable));
        }
    }
    check(binaries.len() == 1 && binaries[0].is_file(), "source")?;
    Ok(binaries.remove(0))
}
fn pure(
    suite: &mut Suite,
    fixture: &Path,
    cases: &[Value],
    report: &mut Value,
    candidate_test: &Path,
    baseline_test: &Path,
) -> Result {
    let state = suite.directory("pure-node-state")?;
    let mut environment = suite.environment.clone();
    crate::server::set(
        &mut environment,
        "SINGLE_NODE_FIXTURES",
        fixture.as_os_str(),
    );
    crate::server::set(
        &mut environment,
        "SUB_STORE_DATA_BASE_PATH",
        state.as_os_str(),
    );
    let mut workers = BTreeMap::new();
    for kind in BACKENDS {
        let mut command = if kind == "node" {
            let mut command = Command::new(&suite.node);
            command
                .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("performance/single-node.cjs"))
                .arg(&suite.args.original)
                .current_dir(suite.args.original.join("backend"));
            command
        } else {
            let mut command = Command::new(if kind == "rust" {
                candidate_test
            } else {
                baseline_test
            });
            command
                .args([
                    "single_node_profile::compare",
                    "--ignored",
                    "--exact",
                    "--nocapture",
                    "--test-threads=1",
                ])
                .current_dir(&suite.args.rust);
            command
        };
        command.env_clear().envs(&environment);
        if kind != "node" {
            command.env("PATH", "");
        }
        workers.insert(kind, Worker::new(&mut command, &suite.args.scratch, kind)?);
    }
    let worker_kind = |variant: &str| {
        if variant == "node" {
            "node"
        } else if variant.starts_with("rust_baseline_") {
            "rust_baseline"
        } else {
            "rust"
        }
    };
    for (index, case) in cases.iter().enumerate() {
        let expected = workers
            .get_mut("node")
            .unwrap()
            .call(index, 1000, 10000, false)?["outputs"]
            .clone();
        check(
            expected[0][0]["type"] == case["protocol"],
            "output_mismatch",
        )?;
        let mut counts = BTreeMap::new();
        for variant in VARIANTS {
            let worker = workers.get_mut(worker_kind(variant)).unwrap();
            let mut count = 1_u64;
            for _ in 0..5 {
                let sample = worker.call(index, count, 2, variant.ends_with("_fresh_parser"))?;
                crate::json::check(&sample["outputs"], &expected, false)?;
                let elapsed = sample["elapsed_ns"].as_u64().unwrap();
                if elapsed >= 125000000 {
                    break;
                }
                count = (count * 2)
                    .max(((count as f64 * 250000000.0 / elapsed.max(1) as f64).ceil()) as u64)
                    .min(1000000);
            }
            counts.insert(variant, count);
        }
        for block in 0..BLOCKS {
            let mut order = VARIANTS;
            if block % 2 == 1 {
                order.reverse();
            }
            for variant in order {
                let mut sample = workers.get_mut(worker_kind(variant)).unwrap().call(
                    index,
                    counts[variant],
                    0,
                    variant.ends_with("_fresh_parser"),
                )?;
                crate::json::check(&sample["outputs"], &expected, false)?;
                sample.as_object_mut().unwrap().remove("outputs");
                sample["case"] = case["id"].clone();
                sample["variant"] = json!(variant);
                sample["block"] = json!(block);
                report["pure"].as_array_mut().unwrap().push(sample);
            }
        }
        println!(
            "single-node pure complete: {}",
            case["id"].as_str().unwrap()
        );
    }
    Ok(())
}
fn payload(case: &Value, index: usize) -> Value {
    json!({"data":format!("{}{}",case["raw"].as_str().unwrap(),if index%2==1{"\n"}else{""}),"client":"JSON","type":"internal"})
}
fn validate_response(raw: &[u8], expected: &Value) -> Result {
    let response: Value = serde_json::from_slice(raw)?;
    crate::json::check(
        &response,
        &json!({"status":"success","data":expected}),
        false,
    )
}
fn endpoint(suite: &mut Suite, cases: &[Value], report: &mut Value, tail: bool) -> Result {
    let mut servers = BTreeMap::new();
    for kind in BACKENDS {
        let directory = suite.directory(&format!("http-{kind}"))?;
        servers.insert(kind, suite.server(kind, directory, false)?);
    }
    let mut first = None;
    for kind in BACKENDS {
        let began = Instant::now();
        let raw = transport::http(
            &format!("{}{ENDPOINT}", servers[kind].origin),
            "POST",
            Some(&payload(&cases[0], 0)),
        )?;
        let elapsed = began.elapsed().as_nanos() as u64;
        let body: Value = serde_json::from_slice(&raw)?;
        if let Some(expected) = &first {
            crate::json::check(&body, expected, false)?;
        } else {
            first = Some(body);
        }
        report["cold"]
            .as_array_mut()
            .unwrap()
            .push(json!({"variant":kind,"case":cases[0]["id"],"ns":elapsed}));
    }
    let source = crate::contracts::cases("conversion-local")?
        .into_iter()
        .find(|case| case["id"] == "P02-input-surge-native")
        .ok_or("Surge preflight missing")?["steps"][0]["body"]
        .clone();
    let text = source["data"]
        .as_str()
        .ok_or("Surge preflight input missing")?;
    let mut inputs = text.lines().map(str::to_owned).collect::<Vec<_>>();
    inputs.extend([text,"not a proxy","type: ss","/*prefix*/{type:'ss',name:'generic',server:'json.example',port:443,cipher:'aes-128-gcm',password:'secret'}//=ss,other.example,443,password=other","shadowsocks=ss.example.com:8388,method=aes-128-gcm,password=secret,tag=type: ss","\"Surge = ss, ss.example.com, 8388, encrypt-method=aes-128-gcm, password=secret\""].map(str::to_owned));
    for raw in &inputs {
        let mut expected = None;
        for kind in BACKENDS {
            let mut source = source.clone();
            source["data"] = json!(raw);
            let body: Value = serde_json::from_slice(&transport::http(
                &format!("{}{ENDPOINT}", servers[kind].origin),
                "POST",
                Some(&source),
            )?)?;
            if let Some(expected) = &expected {
                crate::json::check(&body, expected, false)?;
            } else {
                expected = Some(body);
            }
        }
    }
    println!(
        "single-node public parser preflight complete: inputs={}, backends=3",
        inputs.len()
    );
    let mut connections = BACKENDS
        .into_iter()
        .map(|kind| Ok((kind, transport::Connection::new(&servers[kind].origin)?)))
        .collect::<Result<BTreeMap<_, _>>>()?;
    for case in cases {
        let expected = transport::api(
            &servers["node"].origin,
            "proxy/parse",
            "POST",
            Some(&payload(case, 0)),
        )?;
        check(
            expected["par_res"]
                .as_array()
                .is_some_and(|rows| rows.len() == 1),
            "empty_output",
        )?;
        suite
            .expected
            .insert(case["id"].as_str().unwrap().to_owned(), expected.clone());
        for mode in ["new_connection", "keep_alive"] {
            let mut request = |kind: &str, index: usize| {
                let data = payload(case, index);
                if mode == "keep_alive" {
                    connections
                        .get_mut(kind)
                        .unwrap()
                        .request("POST", ENDPOINT, Some(&data), true)
                } else {
                    transport::http(
                        &format!("{}{ENDPOINT}", servers[kind].origin),
                        "POST",
                        Some(&data),
                    )
                }
            };
            for kind in BACKENDS {
                for index in 0..20 {
                    validate_response(&request(kind, index)?, &expected)?;
                }
            }
            for block in 0..BLOCKS {
                let mut order = BACKENDS;
                if block % 2 == 1 {
                    order.reverse();
                }
                for kind in order {
                    let mut samples = Vec::new();
                    for index in 0..if tail { 200 } else { 30 } {
                        let began = Instant::now();
                        let raw = request(kind, index)?;
                        samples.push(began.elapsed().as_nanos() as u64);
                        validate_response(&raw, &expected)?;
                    }
                    report["http"].as_array_mut().unwrap().push(json!({"case":case["id"],"variant":kind,"mode":mode,"block":block,"ns":samples}));
                }
            }
        }
        println!(
            "single-node HTTP complete: {}",
            case["id"].as_str().unwrap()
        );
    }
    if tail {
        return Ok(());
    }
    for clients in [1, 8] {
        for block in 0..3 {
            let mut order = BACKENDS;
            if block % 2 == 1 {
                order.reverse();
            }
            for kind in order {
                let meter = resources::Meter::start(servers[kind].owned.pid(), None, None)?;
                let next = AtomicUsize::new(0);
                let total = cases.len() * 8;
                let expected = &suite.expected;
                let origin = &servers[kind].origin;
                let samples = thread::scope(|scope| -> Result<Vec<u64>> {
                    let handles = (0..clients)
                        .map(|_| {
                            scope.spawn(|| -> Result<Vec<(usize, u64)>> {
                                let mut samples = Vec::new();
                                loop {
                                    let index = next.fetch_add(1, Ordering::Relaxed);
                                    if index >= total {
                                        break;
                                    }
                                    let case = &cases[index % cases.len()];
                                    let data = payload(case, index);
                                    let began = Instant::now();
                                    let raw = transport::http(
                                        &format!("{origin}{ENDPOINT}"),
                                        "POST",
                                        Some(&data),
                                    )?;
                                    let elapsed = began.elapsed().as_nanos() as u64;
                                    validate_response(
                                        &raw,
                                        &expected[case["id"].as_str().unwrap()],
                                    )?;
                                    samples.push((index, elapsed));
                                }
                                Ok(samples)
                            })
                        })
                        .collect::<Vec<_>>();
                    let mut all = Vec::new();
                    for handle in handles {
                        all.extend(
                            handle
                                .join()
                                .map_err(|_| "Concurrent parser client panicked")??,
                        );
                    }
                    all.sort_by_key(|(index, _)| *index);
                    check(
                        all.len() == total
                            && all
                                .iter()
                                .enumerate()
                                .all(|(index, (observed, _))| index == *observed),
                        "work_incomplete",
                    )?;
                    Ok(all.into_iter().map(|(_, elapsed)| elapsed).collect())
                })?;
                let mut row = meter.finish(samples.len())?;
                row["variant"] = json!(kind);
                row["clients"] = json!(clients);
                row["block"] = json!(block);
                row["ns"] = json!(samples);
                report["concurrency"].as_array_mut().unwrap().push(row);
            }
        }
    }
    println!("single-node concurrent load and retained-memory observations complete");
    Ok(())
}

fn profile_events(bytes: &[u8], count: usize) -> Result<Option<ProfileEvents>> {
    let mut workers = Vec::new();
    let mut http = Vec::new();
    for line in bytes
        .split_inclusive(|byte| *byte == b'\n')
        .filter(|line| line.ends_with(b"\n"))
    {
        if let Some(value) = line.strip_prefix(b"SUB_STORE_PROFILE ") {
            workers.push(serde_json::from_slice::<Value>(value)?);
        } else if let Some(value) = line.strip_prefix(b"SUB_STORE_HTTP_PROFILE ") {
            let row: Value = serde_json::from_slice(value)?;
            check(
                keys(&row, &["version", "wall_ns"])
                    && row["version"] == 1
                    && row["wall_ns"].as_u64().is_some_and(|value| value > 0),
                "resource",
            )?;
            http.push(row["wall_ns"].as_u64().unwrap());
        }
    }
    check(workers.len() <= count && http.len() <= count, "resource")?;
    if workers.len() != count || http.len() != count {
        return Ok(None);
    }
    for worker in &workers {
        validate_profile(worker, "other", false)?;
    }
    Ok(Some((workers, http)))
}
fn complete_profiles(
    path: &Path,
    offset: u64,
    count: usize,
    watchdog: Duration,
) -> Result<ProfileEvents> {
    use std::io::{Read, Seek};
    let began = Instant::now();
    while began.elapsed() < watchdog {
        let mut source = std::fs::File::open(path)?;
        source.seek(std::io::SeekFrom::Start(offset))?;
        let mut bytes = Vec::new();
        source.read_to_end(&mut bytes)?;
        if let Some(events) = profile_events(&bytes, count)? {
            return Ok(events);
        }
        thread::sleep(Duration::from_millis(10));
    }
    Err("Profile completion watchdog".into())
}
fn profiles(suite: &mut Suite, cases: &[Value], report: &mut Value, tail: bool) -> Result {
    suite.args.profile = true;
    let directory = suite.directory("profile-rust")?;
    let app = suite.server("rust_repeat", directory, false)?;
    let mut connection = transport::Connection::new(&app.origin)?;
    let path = &app.owned.process.stdout;
    for (index, case) in cases.iter().enumerate() {
        let expected = &suite.expected[case["id"].as_str().unwrap()];
        let offset = std::fs::metadata(path)?.len();
        for _ in 0..2 {
            let actual =
                transport::api(&app.origin, "proxy/parse", "POST", Some(&payload(case, 0)))?;
            crate::json::check(&actual, expected, false)?;
        }
        let (warmup, _) = complete_profiles(path, offset, 2, Duration::from_secs(30))?;
        conversion_lifecycle(&warmup)?;
        if index == 0 {
            check(
                warmup[0]["rows"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|row| row["stage"] == "conversion_miss"),
                "resource",
            )?;
            report["cold_profile"] = warmup[0].clone();
        }
        let offset = std::fs::metadata(path)?.len();
        let mut client = Vec::new();
        let count = if tail { 1000 } else { BLOCKS };
        for request in 0..count {
            let data = payload(case, if tail { request } else { 0 });
            let began = Instant::now();
            let raw = if tail {
                connection.request("POST", ENDPOINT, Some(&data), true)?
            } else {
                transport::http(&format!("{}{ENDPOINT}", app.origin), "POST", Some(&data))?
            };
            client.push(began.elapsed().as_nanos() as u64);
            validate_response(&raw, expected)?;
        }
        let (events, server) = complete_profiles(path, offset, count, Duration::from_secs(30))?;
        for event in &events {
            let rows = event["rows"].as_array().unwrap();
            check(
                !rows
                    .iter()
                    .any(|row| row["stage"] == "parse_utilities_init"),
                "resource",
            )?;
            if ["URI", "Surge", "QX"].contains(&case["format"].as_str().unwrap()) {
                check(
                    !rows.iter().any(|row| row["stage"] == "parse_context_init"),
                    "resource",
                )?;
            }
        }
        conversion_lifecycle(&events)?;
        report["profiles"]
            .as_array_mut()
            .unwrap()
            .push(json!({"case":case["id"],"events":events,"server_ns":server,"client_ns":client}));
    }
    Ok(())
}

fn path(key: &str) -> Result<PathBuf> {
    Ok(
        PathBuf::from(std::env::var_os(key).ok_or("Missing single-node input path")?)
            .canonicalize()?,
    )
}
pub fn run() -> Result {
    crate::actions_only();
    check(
        cfg!(all(target_os = "linux", target_arch = "x86_64")) && !cfg!(debug_assertions),
        "source",
    )?;
    let tail = std::env::var("PERF_DIAGNOSE_TAIL").as_deref() == Ok("1");
    let scratch = crate::process::scratch("single-node")?;
    let publish =
        PathBuf::from(std::env::var_os("PERF_PUBLIC").ok_or("Missing single-node output path")?);
    std::fs::create_dir(&publish)?;
    let frontend = scratch.join("unused-frontend");
    std::fs::create_dir_all(frontend.join("dist"))?;
    let args = Arguments {
        original: path("PERF_ORIGINAL")?,
        rust: path("PERF_RUST")?,
        baseline: Some(path("PERF_BASELINE")?),
        frontend,
        scratch,
        publish,
        diagnostic: None,
        profile: false,
    };
    let candidate_test = build_binary(&path("PERF_TEST_BUILD")?)?;
    let baseline_test = build_binary(&path("PERF_BASELINE_TEST_BUILD")?)?;
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("performance/single-node.json");
    let mut cases = read(&fixture)?
        .as_array()
        .ok_or("Single-node fixture list missing")?
        .clone();
    let mut suite = Suite::new(args, json!({}))?;
    check(
        suite
            .command(
                Command::new(&suite.node).arg("--version"),
                "node-version",
                30,
            )?
            .trim()
            == "v24.16.0",
        "source",
    )?;
    let source = std::fs::read_to_string(
        suite
            .args
            .original
            .join("backend/src/core/proxy-utils/parsers/index.js"),
    )?;
    let inventory =
        regex::Regex::new(r"(?s)function Clash_All\(\).*?!\[(.*?)\]\.includes\(proxy\.type\)")?
            .captures(&source)
            .ok_or("Pinned Clash inventory missing")?
            .get(1)
            .unwrap()
            .as_str();
    let types = regex::Regex::new(r"'([^']+)'")?
        .captures_iter(inventory)
        .map(|value| value[1].to_owned())
        .collect::<Vec<_>>();
    let covered = cases
        .iter()
        .filter(|case| case["format"] == "Clash JSON")
        .map(|case| {
            Ok(serde_json::from_str::<Value>(
                case["raw"].as_str().ok_or("Clash JSON text missing")?,
            )?["type"]
                .as_str()
                .ok_or("Clash type missing")?
                .to_owned())
        })
        .collect::<Result<BTreeSet<_>>>()?;
    check(
        cases.len() == 48
            && cases
                .iter()
                .map(|case| case["id"].as_str())
                .collect::<BTreeSet<_>>()
                .len()
                == cases.len()
            && covered == types.iter().cloned().collect()
            && cases
                .iter()
                .filter_map(|case| case["protocol"].as_str())
                .map(str::to_owned)
                .collect::<BTreeSet<_>>()
                == covered.into_iter().chain(["external".to_owned()]).collect()
            && cases
                .iter()
                .any(|case| case["protocol"] == "external" && case["format"] == "Surge"),
        "input",
    )?;
    if tail {
        cases.retain(|case| TAIL_CASES.contains(&case["id"].as_str().unwrap()));
        check(cases.len() == 3, "input")?;
    }
    let mut revisions = BTreeMap::new();
    for (kind, root) in [
        ("original", suite.args.original.clone()),
        ("rust", suite.args.rust.clone()),
        ("rust_baseline", suite.args.baseline.clone().unwrap()),
    ] {
        revisions.insert(
            kind,
            suite
                .command(
                    Command::new("git")
                        .arg("-C")
                        .arg(&root)
                        .args(["rev-parse", "HEAD"]),
                    "source-revision",
                    30,
                )?
                .trim()
                .to_owned(),
        );
        suite.command(
            Command::new("git")
                .arg("-C")
                .arg(root)
                .args(["diff", "--exit-code"]),
            "clean-source",
            30,
        )?;
    }
    check(
        revisions["original"] == ORIGINAL && revisions["rust_baseline"] == BASELINE,
        "source",
    )?;
    let binaries = json!({"rust":digest(std::fs::read(suite.args.rust.join("rust/target/release/sub-store-rust"))?),"rust_baseline":digest(std::fs::read(suite.args.baseline.as_ref().unwrap().join("rust/target/release/sub-store-rust"))?)});
    check(binaries["rust"] != binaries["rust_baseline"], "source")?;
    let mut report = json!({"version":1,"original":revisions["original"],"rust":revisions["rust"],"rust_baseline":revisions["rust_baseline"],"harness":suite.command(Command::new("git").args(["rev-parse","HEAD"]),"harness-revision",30)?.trim(),"fixture_sha256":digest(std::fs::read(&fixture)?),"machine":std::env::consts::ARCH,"binary_sha256":binaries,"clash_types":types,"case_info":cases.iter().map(|case|(case["id"].as_str().unwrap().to_owned(),json!({"protocol":case["protocol"],"format":case["format"]}))).collect::<serde_json::Map<_,_>>(),"cases":cases.iter().map(|case|case["id"].clone()).collect::<Vec<_>>(),"pure":[],"http":[],"profiles":[],"cold":[],"concurrency":[],"status":"running"});
    let result = (|| -> Result {
        if tail {
            for round in 0..3 {
                let start = report["http"].as_array().unwrap().len();
                endpoint(&mut suite, &cases, &mut report, true)?;
                for row in &mut report["http"].as_array_mut().unwrap()[start..] {
                    row["round"] = json!(round);
                }
            }
            profiles(&mut suite, &cases, &mut report, true)?;
        } else {
            pure(
                &mut suite,
                &fixture,
                &cases,
                &mut report,
                &candidate_test,
                &baseline_test,
            )?;
            endpoint(&mut suite, &cases, &mut report, false)?;
            profiles(&mut suite, &cases, &mut report, false)?;
            for mode in ["new_connection", "keep_alive"] {
                let ratios = single_report::ratios(&report, mode)?;
                check(median(&ratios)? < 1.0, "resource")?;
            }
        }
        Ok(())
    })();
    report["status"] = json!(if result.is_ok() { "passed" } else { "failed" });
    write(suite.args.publish.join("metrics.json"), &report)?;
    if let Err(error) = &result {
        std::fs::write(suite.args.publish.join("failure.txt"), format!("{error:?}"))?;
    }
    let text = if tail {
        single_report::tail_summary(&report)
    } else {
        single_report::summary(&report)
    };
    match text {
        Ok(text) => {
            std::fs::write(suite.args.publish.join("summary.md"), &text)?;
            println!("{text}");
        }
        Err(error) if result.is_ok() => {
            report["status"] = json!("failed");
            write(suite.args.publish.join("metrics.json"), &report)?;
            return Err(error);
        }
        Err(_) => {}
    }
    result
}
pub fn inspect() -> Result {
    crate::actions_only();
    let root = path("PERF_INSPECT")?;
    let report = read(single_report::one_metrics(&root)?)?;
    println!("{}", single_report::tail_summary(&report)?);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parser_output_equality_preserves_json_numbers_types_fields_and_order() -> Result {
        let expected = json!({"par_res":[{"type":"vmess","port":443,"alterId":0}],"pro_res":[]});
        let actual = json!({"par_res":[{"type":"vmess","port":443.0,"alterId":0.0}],"pro_res":[]});
        crate::json::check(&actual, &expected, false)?;
        validate_response(
            &canonical(&json!({"status":"success","data":actual})),
            &expected,
        )?;
        for altered in [json!(false), json!("0"), json!(null), json!(1)] {
            let mut changed = expected.clone();
            changed["par_res"][0]["alterId"] = altered;
            assert!(
                validate_response(
                    &canonical(&json!({"status":"success","data":changed})),
                    &expected,
                )
                .is_err()
            );
        }
        let mut missing = expected.clone();
        missing.as_object_mut().unwrap().remove("pro_res");
        assert!(crate::json::check(&missing, &expected, false).is_err());
        assert!(crate::json::check(&json!([1, 2]), &json!([2, 1]), false).is_err());
        Ok(())
    }
    fn event(stages: &[&str]) -> Value {
        json!({"rows":stages.iter().chain(["parse","parse_native","produce"].iter()).map(|stage|json!({"stage":stage,"calls":1})).collect::<Vec<_>>()})
    }
    #[test]
    fn lifecycle_rejects_false_reuse_missing_initialization_and_incomplete_cleanup() -> Result {
        let reused = [
            "worker_queue",
            "conversion_reuse",
            "conversion_cleanup",
            "conversion_retain",
        ];
        let created = [
            "worker_queue",
            "conversion_miss",
            "runtime_setup",
            "context_setup",
            "bootstrap",
            "conversion_cleanup",
            "conversion_retain",
        ];
        for stages in [
            reused.to_vec(),
            created.to_vec(),
            vec![
                "worker_queue",
                "conversion_reuse",
                "conversion_cleanup",
                "runtime_teardown",
            ],
        ] {
            conversion_lifecycle(&[event(&stages)])?;
        }
        let mut invalid = vec![
            reused[1..].to_vec(),
            reused[..3].to_vec(),
            vec!["worker_queue", "conversion_reuse", "conversion_retain"],
            created
                .iter()
                .copied()
                .filter(|stage| *stage != "context_setup")
                .collect(),
        ];
        for extra in ["bootstrap", "conversion_miss", "bootstrap_state_encode"] {
            invalid.push(reused.iter().copied().chain([extra]).collect());
        }
        for stages in invalid {
            assert!(conversion_lifecycle(&[event(&stages)]).is_err());
        }
        Ok(())
    }
    #[test]
    fn profile_completion_waits_for_late_teardown_and_rejects_extra_private_or_missing_events()
    -> Result {
        let root = crate::process::scratch("single-profile-completion")?;
        let path = root.join("server.log");
        let worker = json!({"version":1,"job":1,"operation":"other","success":true,"wall_ns":100,"dropped":0,"open_scopes":0,"unfinished_spans":0,"rows":[{"id":0,"parent":null,"stage":"runtime_teardown","index":null,"calls":1,"wall_ns":10,"active_ns":10,"self_ns":10,"max_wall_ns":10}]});
        let http = b"SUB_STORE_HTTP_PROFILE {\"version\":1,\"wall_ns\":90}\n";
        let teardown =
            format!("SUB_STORE_PROFILE {}\n", serde_json::to_string(&worker)?).into_bytes();
        let prefix = b"private preceding log\n";
        let complete = [prefix.as_slice(), http, &teardown].concat();
        std::fs::write(&path, &complete[..complete.len() - 3])?;
        let delayed = path.clone();
        let writer = thread::spawn(move || {
            thread::sleep(Duration::from_millis(25));
            std::fs::write(delayed, complete)
        });
        let (events, times) =
            complete_profiles(&path, prefix.len() as u64, 1, Duration::from_secs(2))?;
        writer.join().map_err(|_| "Profile writer panicked")??;
        assert_eq!(events, vec![worker]);
        assert_eq!(times, vec![90]);
        for bad in [
            [http.as_slice(), http].concat(),
            b"SUB_STORE_HTTP_PROFILE {\"version\":1,\"wall_ns\":90,\"secret\":\"private\"}\n"
                .to_vec(),
            b"SUB_STORE_HTTP_PROFILE {\"version\":1,\"wall_ns\":true}\n".to_vec(),
        ] {
            assert!(profile_events(&[bad, teardown.clone()].concat(), 1).is_err());
        }
        std::fs::write(&path, http)?;
        assert!(complete_profiles(&path, 0, 1, Duration::from_millis(25)).is_err());
        Ok(())
    }
    #[test]
    fn cargo_artifact_selection_rejects_empty_ambiguous_or_missing_test_executables() -> Result {
        let root = crate::process::scratch("single-build-json")?;
        let path = root.join("build.jsonl");
        let binary = root.join("owned-test");
        std::fs::write(&binary, b"fixture")?;
        let artifact =
            json!({"reason":"compiler-artifact","profile":{"test":true},"executable":binary});
        std::fs::write(&path, format!("{}\n", serde_json::to_string(&artifact)?))?;
        assert_eq!(build_binary(&path)?, binary);
        std::fs::write(
            &path,
            format!("{0}\n{0}\n", serde_json::to_string(&artifact)?),
        )?;
        assert!(build_binary(&path).is_err());
        for content in [
            "",
            r#"{"reason":"compiler-artifact","profile":{"test":true},"executable":"missing-owned-test"}"#,
        ] {
            std::fs::write(&path, content)?;
            assert!(build_binary(&path).is_err());
        }
        Ok(())
    }
}
