use crate::Result;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
    path::Path,
};

pub mod capsule;
pub mod detector;
pub mod measure;
pub mod report;
pub mod resources;
pub mod single;
mod single_report;
pub mod subscription;
pub mod transport;
pub mod workload;

pub const ORIGINAL: &str = "9cc6b5fac966993364ae33da8797f446bbf7f571";
pub const RUST: &str = "af08b0649c4d2f2814fe38ce6943b1397f5e1c14";
pub const RUST_BASELINE: &str = "ee64ab9f7376a68c126950caee256bf3fb9c2b12";
pub const FRONTEND: &str = "4bd7b0acbe65466b933ccea14fc61dbebe036ead";
pub const INPUT_HASH: &str = "a6e6bb9cbe0587f33de0cf130ec391ecffd3483009888fed0ea59d39492ae890";
pub const INPUT_NAME: &str = "sub-store_subscription_大杂烩-cron-new_2026-10-20_14-11-50.json";
pub const SCRIPT_BLOB: &str = "5fc44546b85b965abaa710924e507fae3f50a83d";
pub const SCRIPT_HASH: &str = "03d4c889ddec7a56576eb14f191e5187a7571d8122bc987431bb4eb48b3b1da8";
pub const KINDS: [&str; 2] = ["node", "rust"];
pub const COMPARISON_KINDS: [&str; 4] = ["node", "rust", "rust_baseline", "rust_repeat"];
pub const CASES: [&str; 8] = [
    "import",
    "cold",
    "warm",
    "pressure",
    "real",
    "parse",
    "pre_health",
    "no_publisher",
];
pub const STAGES: &str = "worker_queue executor_setup state_snapshot runtime_setup context_setup runtime_teardown job bootstrap input_decode execute output_encode conversion_miss conversion_reuse conversion_cleanup conversion_retain bootstrap_compile bootstrap_load bootstrap_eval bootstrap_values bootstrap_state_encode bootstrap_state_parse sources pipeline_parse pipeline_process pipeline_produce pipeline_utility process processor script_operator script_filter useless_filter region_filter regex_filter type_filter conditional_filter quick_setting_operator flag_operator sort_operator regex_sort_operator regex_rename_operator regex_delete_operator handle_duplicate_operator add_proxies_operator resolve_domain_operator unknown_processor builtin_prepare builtin_apply script_load script_prepare script_compile script_bind script_call script_await script_fallback response_transformer response_normalize input_clone clone_stringify clone_parse parse parse_runtime parse_context_init parse_utilities_init parse_scalar_probe parse_native parse_preprocess parse_nodes produce produce_validate produce_filter filter_callback produce_prepare prepare_callback produce_collect produce_project_nodes produce_project_options produce_reconcile produce_native produce_decode value_project project_stringify project_extract project_parse project_restore value_stringify value_decode decode_classify decode_encode decode_parse decode_restore decode_build";
const NODE_FIELDS: &str = "name type server port ports password uuid sni tls _subName _subDisplayName _latency _resolved resolved id network cipher udp tfo skip-cert-verify client-fingerprint ws-opts reality-opts grpc-opts http-opts alpn servername";
const ERRORS: &str = "input asset source startup http browser import_fields import_network persistence capture script_anchor empty_output output_mismatch snapshot work_incomplete warm_miss timeout fixture privacy unexpected clash resource external_io";

#[derive(Debug)]
pub struct Failure {
    pub code: &'static str,
    pub observations: Option<Value>,
}
impl fmt::Display for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.code)
    }
}
impl std::error::Error for Failure {}
pub fn check(condition: bool, code: &'static str) -> Result {
    if condition {
        Ok(())
    } else {
        Err(Box::new(Failure {
            code,
            observations: None,
        }))
    }
}
pub fn failure(error: &(dyn std::error::Error + Send + Sync + 'static)) -> Value {
    match error.downcast_ref::<Failure>() {
        Some(error) => json!({"failure":error.code,"failure_observations":error.observations}),
        None => json!({"failure":"unexpected"}),
    }
}
pub fn canonical(value: &Value) -> Vec<u8> {
    fn sorted(value: &Value) -> Value {
        match value {
            Value::Object(map) => Value::Object(
                map.iter()
                    .map(|(key, value)| (key.clone(), sorted(value)))
                    .collect::<BTreeMap<_, _>>()
                    .into_iter()
                    .collect(),
            ),
            Value::Array(items) => Value::Array(items.iter().map(sorted).collect()),
            _ => value.clone(),
        }
    }
    serde_json::to_vec(&sorted(value)).expect("JSON values serialize")
}
pub fn digest(bytes: impl AsRef<[u8]>) -> String {
    aws_lc_rs::digest::digest(&aws_lc_rs::digest::SHA256, bytes.as_ref())
        .as_ref()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
pub fn hash(value: &Value) -> String {
    digest(canonical(value))
}
pub fn write(path: impl AsRef<Path>, value: &Value) -> Result {
    std::fs::write(path, canonical(value))?;
    Ok(())
}
pub fn read(path: impl AsRef<Path>) -> Result<Value> {
    Ok(serde_json::from_slice(&std::fs::read(path)?)?)
}

pub fn shards() -> Vec<(String, (&'static str, u64, u64))> {
    let mut rows = Vec::new();
    for (case, blocks) in [
        ("parse", 3),
        ("pre_health", 1),
        ("cold", 10),
        ("warm", 5),
        ("pressure", 3),
        ("no_publisher", 3),
    ] {
        for clients in if case == "pressure" {
            vec![1, 2, 4, 8]
        } else {
            vec![1]
        } {
            for block in 0..blocks {
                rows.push((format!("{case}_{block}_{clients}"), (case, block, clients)));
            }
        }
    }
    rows.push(("real_0_1".to_owned(), ("real", 0, 1)));
    rows
}
pub fn public(value: &Value, models: &BTreeSet<String>) -> Result {
    let allowed=STAGES.split_whitespace().chain(ERRORS.split_whitespace()).chain(NODE_FIELDS.split_whitespace()).chain(COMPARISON_KINDS).chain(CASES).chain("passed failed running fixture real_network empty_or_failed nonempty linux_x64 not_run warming unpaired partial_sources parse_diagnostic health_diagnostic download_sub download_collection download_file other missing object array string number boolean null".split_whitespace()).collect::<BTreeSet<_>>();
    let shard_names = shards()
        .into_iter()
        .map(|(name, _)| name)
        .collect::<BTreeSet<_>>();
    let version_or_hash = regex::Regex::new(r"^(?:[0-9a-f]{40}|[0-9a-f]{64}|\d+(?:\.\d+){1,3})$")?;
    let key_pattern = regex::Regex::new(r"^[a-z][a-z0-9_]*$")?;
    fn visit(
        value: &Value,
        allowed: &BTreeSet<&str>,
        shards: &BTreeSet<String>,
        models: &BTreeSet<String>,
        strings: &regex::Regex,
        keys: &regex::Regex,
    ) -> Result {
        match value {
            Value::Null | Value::Bool(_) => Ok(()),
            Value::Number(number) => check(number.as_f64().is_some_and(f64::is_finite), "privacy"),
            Value::String(text) => check(
                allowed.contains(text.as_str())
                    || shards.contains(text)
                    || models.contains(text)
                    || strings.is_match(text),
                "privacy",
            ),
            Value::Array(items) => {
                for item in items {
                    visit(item, allowed, shards, models, strings, keys)?;
                }
                Ok(())
            }
            Value::Object(map) => {
                for (key, item) in map {
                    check(keys.is_match(key), "privacy")?;
                    visit(item, allowed, shards, models, strings, keys)?;
                }
                Ok(())
            }
        }
    }
    visit(
        value,
        &allowed,
        &shard_names,
        models,
        &version_or_hash,
        &key_pattern,
    )
}
pub fn models(report: &Value) -> BTreeSet<String> {
    std::iter::once(&report["provenance"]["runner"])
        .chain(
            report["workers"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|worker| &worker["runner"]),
        )
        .filter_map(|runner| runner["cpu_model"].as_str().map(str::to_owned))
        .collect()
}

pub fn semantic(nodes: &Value, hopping: bool) -> Result<Value> {
    let mut result = nodes.as_array().ok_or("Node list required")?.clone();
    let pattern = regex::Regex::new(r"^\d+(?:-\d+)?(?:[,/]\d+(?:-\d+)?)*$")?;
    for item in &mut result {
        let map = item.as_object_mut().ok_or("Node object required")?;
        map.remove("_latency");
        if hopping
            && let (Some(ports), Some(port)) = (
                map.get("ports").and_then(Value::as_str),
                map.get("port").and_then(Value::as_u64),
            )
            && pattern.is_match(ports)
        {
            let ranges = ports
                .split([',', '/'])
                .map(|part| {
                    let mut pair = part.split('-');
                    let low = pair.next()?.parse::<u64>().ok()?;
                    let high = pair.next().map_or(Some(low), |value| value.parse().ok())?;
                    Some((low, high))
                })
                .collect::<Option<Vec<_>>>();
            if let Some(ranges) = ranges
                && ranges
                    .iter()
                    .all(|(low, high)| 1 <= *low && low <= high && *high <= 65535)
                && ranges
                    .iter()
                    .any(|(low, high)| *low <= port && port <= *high)
            {
                map.insert("port".to_owned(), json!(ranges[0].0));
            }
        }
    }
    Ok(json!(result))
}
pub fn nodes_hash(nodes: &Value, hopping: bool) -> Result<String> {
    Ok(hash(&semantic(nodes, hopping)?))
}
fn value_type(value: Option<&Value>) -> &'static str {
    match value {
        None => "missing",
        Some(Value::Null) => "null",
        Some(Value::Bool(_)) => "boolean",
        Some(Value::Number(_)) => "number",
        Some(Value::String(_)) => "string",
        Some(Value::Array(_)) => "array",
        Some(Value::Object(_)) => "object",
    }
}
pub fn compare_nodes(expected: &Value, actual: &Value, hopping: bool) -> Result {
    let (left, right) = (semantic(expected, hopping)?, semantic(actual, hopping)?);
    if hash(&left) == hash(&right) {
        return Ok(());
    }
    fn visit(
        left: Option<&Value>,
        right: Option<&Value>,
        path: Vec<Value>,
        differences: &mut Vec<Value>,
    ) {
        if differences.len() >= 20 || left == right {
            return;
        }
        match(left,right){
            (Some(Value::Object(a)),Some(Value::Object(b)))=>{
                for key in a.keys().chain(b.keys()).collect::<BTreeSet<_>>(){let mut at=path.clone();at.push(json!(if NODE_FIELDS.split_whitespace().any(|field|field==key){key.clone()}else{digest(key.as_bytes())}));visit(a.get(key),b.get(key),at,differences);}
            },
            (Some(Value::Array(a)),Some(Value::Array(b)))=>{for index in 0..a.len().max(b.len()){let mut at=path.clone();at.push(json!(index));visit(a.get(index),b.get(index),at,differences);}},
            _=>differences.push(json!({"path":path,"expected_type":value_type(left),"actual_type":value_type(right),"expected_sha256":left.map(hash),"actual_sha256":right.map(hash)}))
        }
    }
    let mut differences = Vec::new();
    visit(Some(&left), Some(&right), vec![], &mut differences);
    Err(Box::new(Failure {
        code: "output_mismatch",
        observations: Some(
            json!({"expected_count":left.as_array().unwrap().len(),"actual_count":right.as_array().unwrap().len(),"differences":differences}),
        ),
    }))
}
pub fn import_transform(original: &Value, payload: &Value) -> Result {
    let name = original["name"].as_str().ok_or("Original name missing")?;
    let imported = payload["name"].as_str().unwrap_or("");
    let suffix = imported.strip_prefix(name).unwrap_or("");
    check(
        regex::Regex::new(r"^_\d{13}$")?.is_match(suffix),
        "import_fields",
    )?;
    let mut expected = original.clone();
    expected["name"] = json!(format!("{name}{suffix}"));
    let display = format!(
        "{}{suffix}",
        original["displayName"]
            .as_str()
            .ok_or("Original display name missing")?
    );
    expected["displayName"] = json!(display);
    expected["display-name"] = json!(display);
    check(payload == &expected, "import_fields")
}
pub fn configured(original: &Value, origin: &str, variant: &str) -> Result<Value> {
    let mut config = original.clone();
    config["url"] = json!(
        (0..22)
            .map(|index| format!("{origin}/source/{index}"))
            .collect::<Vec<_>>()
            .join("\n")
    );
    let port = reqwest::Url::parse(origin)?
        .port()
        .ok_or("Fixture port missing")?;
    let linked = &mut config["process"][4]["args"];
    linked["arguments"]["http_meta_port"] = json!(port.to_string());
    linked["content"] = json!(format!(
        "{origin}/health.js#{}",
        crate::fixture::quote(std::str::from_utf8(&canonical(&linked["arguments"]))?)
    ));
    let processors = config["process"]
        .as_array_mut()
        .ok_or("Processors missing")?;
    match variant {
        "parse" => processors.clear(),
        "pre_health" => processors.truncate(4),
        "no_publisher" => {
            processors.pop();
        }
        _ => {}
    }
    Ok(config)
}
pub fn inspection(config: &Value) -> Value {
    let mut result = config.clone();
    result["process"] = json!([{"type":"Script Operator","args":{"mode":"script","content":"function operator(proxies, target, context) { return context.raw; }"}}]);
    result
}
pub fn stress_script(source: &str) -> Result<String> {
    let mut result = source.to_owned();
    for (old, new) in [
        (
            "const concurrency = Math.floor(num('concurrency', 150, 1, 256));",
            "const concurrency = Math.max(1, proxies.length);",
        ),
        (
            "const batchSize = Math.floor(num('batch_size', 200, 1, 500));",
            "const batchSize = Math.max(1, proxies.length);",
        ),
        (
            "const maxChecks = Math.floor(num('max_checks', 600, 0, 5000));",
            "const maxChecks = Math.max(1, proxies.length);",
        ),
        (
            "const maxRunMs = Math.floor(num('max_run_ms', 90000, 10000, 600000));",
            "const maxRunMs = Number.POSITIVE_INFINITY;",
        ),
        (
            "const leaseMs = Math.max(60000, startDelay + nWaves * Math.max(oneProbeMax, proxyLeaseTimeout) + 45000);",
            "const leaseMs = 600000;",
        ),
    ] {
        check(result.matches(old).count() == 1, "script_anchor")?;
        result = result.replace(old, new);
    }
    Ok(result)
}
pub fn health_script(source: &str, enabled: bool) -> String {
    if !enabled {
        return source.to_owned();
    }
    format!(
        "{source}{}",
        r#"
const benchmarkOperator = operator;
operator = async function (...args) {
  const profile = {}, restore = [], began = Date.now();
  function measure(owner, key, name) {
    const original = owner[key], calls = name + '_calls', elapsed = name + '_ms';
    profile[calls] = profile[elapsed] = 0;
    owner[key] = function (...values) {
      const start = Date.now(); profile[calls]++;
      try { return Reflect.apply(original, this, values); }
      finally { profile[elapsed] += Date.now() - start; }
    };
    restore.push(() => { owner[key] = original; });
  }
  measure(ProxyUtils, 'hex_md5', 'md5');
  measure(ProxyUtils, 'produce', 'produce');
  measure(scriptResourceCache, 'get', 'cache_get');
  measure(scriptResourceCache, 'set', 'cache_set');
  try { return await Reflect.apply(benchmarkOperator, this, args); }
  finally {
    profile.total_ms = Date.now() - began;
    for (const reset of restore) reset();
    $substore.write(profile, '#benchmark-health-profile');
  }
};
"#
    )
}
pub fn median(values: &[f64]) -> Result<f64> {
    check(
        !values.is_empty() && values.iter().all(|value| value.is_finite()),
        "resource",
    )?;
    let mut ordered = values.to_vec();
    ordered.sort_by(f64::total_cmp);
    let middle = ordered.len() / 2;
    Ok(if ordered.len().is_multiple_of(2) {
        (ordered[middle - 1] + ordered[middle]) / 2.0
    } else {
        ordered[middle]
    })
}
pub fn distribution(values: &[f64]) -> Result<Value> {
    if values.is_empty() {
        return Ok(json!({"n":0}));
    }
    let mut ordered = values.to_vec();
    check(ordered.iter().all(|value| value.is_finite()), "resource")?;
    ordered.sort_by(f64::total_cmp);
    let mean = ordered.iter().sum::<f64>() / ordered.len() as f64;
    let stdev = if ordered.len() > 1 {
        (ordered
            .iter()
            .map(|value| (value - mean).powi(2))
            .sum::<f64>()
            / (ordered.len() - 1) as f64)
            .sqrt()
    } else {
        0.0
    };
    let mut result = json!({"n":ordered.len(),"median":median(&ordered)?,"min":ordered[0],"max":ordered[ordered.len()-1],"mean":mean,"stdev":stdev});
    if ordered.len() >= 20 {
        result["p95"] = json!(ordered[(ordered.len() as f64 * 0.95).ceil() as usize - 1]);
    }
    Ok(result)
}
pub fn paired_interval(pairs: &[(f64, f64)]) -> Result<Value> {
    check(
        !pairs.is_empty()
            && pairs
                .iter()
                .all(|(a, b)| *a > 0.0 && *b > 0.0 && a.is_finite() && b.is_finite()),
        "resource",
    )?;
    let logs = pairs.iter().map(|(a, b)| (a / b).ln()).collect::<Vec<_>>();
    let mut seed = 20261020_u64;
    let mut boots = Vec::with_capacity(2000);
    for _ in 0..2000 {
        let mut sum = 0.0;
        for _ in &logs {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            sum += logs[((seed as u128 * logs.len() as u128) >> 64) as usize];
        }
        boots.push((sum / logs.len() as f64).exp());
    }
    boots.sort_by(f64::total_cmp);
    Ok(
        json!({"pairs":pairs.len(),"node_over_rust":(logs.iter().sum::<f64>()/logs.len()as f64).exp(),"bootstrap_low":boots[49],"bootstrap_high":boots[1949]}),
    )
}

fn keys(value: &Value, expected: &[&str]) -> bool {
    value.as_object().is_some_and(|map| {
        map.keys().map(String::as_str).collect::<BTreeSet<_>>()
            == expected.iter().copied().collect()
    })
}
pub fn validate_profile(report: &Value, operation: &str, processor: bool) -> Result {
    public(report, &BTreeSet::new())?;
    check(
        keys(
            report,
            &[
                "version",
                "job",
                "operation",
                "success",
                "wall_ns",
                "dropped",
                "open_scopes",
                "unfinished_spans",
                "rows",
            ],
        ),
        "resource",
    )?;
    check(
        report["version"] == 1
            && report["operation"] == operation
            && report["success"] == true
            && ["dropped", "open_scopes", "unfinished_spans"]
                .iter()
                .all(|key| report[key] == 0),
        "resource",
    )?;
    let rows = report["rows"].as_array().ok_or("Profile rows missing")?;
    check(!rows.is_empty() && rows.len() <= 2048, "resource")?;
    let mut own = 0_u128;
    let mut roots = 0_u128;
    for (index, row) in rows.iter().enumerate() {
        check(
            keys(
                row,
                &[
                    "id",
                    "parent",
                    "stage",
                    "index",
                    "calls",
                    "wall_ns",
                    "active_ns",
                    "self_ns",
                    "max_wall_ns",
                ],
            ),
            "resource",
        )?;
        check(
            row["id"].as_u64() == Some(index as u64)
                && (row["parent"].is_null()
                    || row["parent"]
                        .as_u64()
                        .is_some_and(|parent| parent < index as u64)),
            "resource",
        )?;
        check(
            row["stage"]
                .as_str()
                .is_some_and(|stage| STAGES.split_whitespace().any(|known| stage == known))
                && (row["index"].is_null() || row["index"].as_u64().is_some()),
            "resource",
        )?;
        check(
            ["calls", "wall_ns", "active_ns", "self_ns", "max_wall_ns"]
                .iter()
                .all(|key| row[key].as_u64().is_some()),
            "resource",
        )?;
        let (calls, wall, active, own_ns, max) = (
            row["calls"].as_u64().unwrap(),
            row["wall_ns"].as_u64().unwrap(),
            row["active_ns"].as_u64().unwrap(),
            row["self_ns"].as_u64().unwrap(),
            row["max_wall_ns"].as_u64().unwrap(),
        );
        check(
            calls > 0 && own_ns <= active && active <= wall && max <= wall,
            "resource",
        )?;
        own += own_ns as u128;
        if row["parent"].is_null() {
            roots += active as u128;
        }
    }
    check(
        own == roots && (!processor || rows.iter().any(|row| row["stage"] == "processor")),
        "resource",
    )
}
pub fn pipeline_profiles(
    path: &Path,
    offset: u64,
    operation: &str,
    processor: bool,
) -> Result<Vec<Value>> {
    use std::io::{BufRead, Seek};
    let mut file = std::fs::File::open(path)?;
    file.seek(std::io::SeekFrom::Start(offset))?;
    let mut reports = Vec::new();
    for line in std::io::BufReader::new(file).split(b'\n') {
        let line = line?;
        if let Some(body) = line.strip_prefix(b"SUB_STORE_PROFILE ") {
            let report = serde_json::from_slice(body).map_err(|_| Failure {
                code: "resource",
                observations: None,
            })?;
            validate_profile(&report, operation, processor)?;
            reports.push(report);
        }
    }
    Ok(reports)
}
pub fn conversion_lifecycle(events: &[Value]) -> Result {
    for event in events {
        let rows = event["rows"].as_array().ok_or("Lifecycle rows missing")?;
        let count = |stage: &str| {
            rows.iter()
                .filter(|row| row["stage"] == stage)
                .map(|row| row["calls"].as_u64().unwrap_or(0))
                .sum::<u64>()
        };
        check(
            count("worker_queue") == 1
                && count("conversion_cleanup") == 1
                && count("conversion_miss") + count("conversion_reuse") == 1
                && count("conversion_retain") + count("runtime_teardown") == 1,
            "resource",
        )?;
        check(
            ["parse", "parse_native", "produce"]
                .iter()
                .all(|stage| count(stage) == 1)
                && ["runtime_setup", "context_setup", "bootstrap"]
                    .iter()
                    .all(|stage| count(stage) == count("conversion_miss"))
                && ["executor_setup", "state_snapshot", "bootstrap_state_encode"]
                    .iter()
                    .all(|stage| count(stage) == 0),
            "resource",
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn equivalence_retains_credentials_metadata_order_nested_latency_and_hopping_bounds() -> Result
    {
        let nodes = json!([{"name":"x","password":"secret","_latency":"1","_subName":"x","nested":{"_latency":"a"}}]);
        let mut other = nodes.clone();
        other[0]["_latency"] = json!("2");
        compare_nodes(&nodes, &other, false)?;
        for key in ["password", "_subName", "nested"] {
            let mut changed = other.clone();
            changed[0][key] = json!("different");
            assert_ne!(nodes_hash(&nodes, false)?, nodes_hash(&changed, false)?);
        }
        assert!(
            compare_nodes(
                &json!([{"name":"a"},{"name":"b"}]),
                &json!([{"name":"b"},{"name":"a"}]),
                false
            )
            .is_err()
        );
        let left = json!([{"name":"x","ports":"100-102,200","port":101}]);
        let right = json!([{"name":"x","ports":"100-102,200","port":200}]);
        assert_ne!(nodes_hash(&left, false)?, nodes_hash(&right, false)?);
        compare_nodes(&left, &right, true)?;
        for patch in [
            json!({"port":99}),
            json!({"port":"101"}),
            json!({"ports":"100-103,200"}),
            json!({"password":"different"}),
        ] {
            let mut changed = right.clone();
            changed[0]
                .as_object_mut()
                .unwrap()
                .extend(patch.as_object().unwrap().clone());
            assert!(compare_nodes(&left, &changed, true).is_err());
        }
        assert!(compare_nodes(&left, &json!([{"name":"x","ports":"100-102,200"}]), true).is_err());
        assert_eq!(left[0]["port"], 101);
        Ok(())
    }
    #[test]
    fn private_fields_and_values_never_enter_public_mismatch_reports() -> Result {
        let error = compare_nodes(
            &json!([{"password":"secret-a","private-token-key":["private-value"],"_latency":1}]),
            &json!([{"password":"secret-b","private-token-key":["changed"],"_latency":2}]),
            false,
        )
        .unwrap_err();
        let report = failure(error.as_ref());
        public(&report, &BTreeSet::new())?;
        let text = serde_json::to_string(&report)?;
        assert!(text.contains("password"));
        for private in [
            "secret-a",
            "secret-b",
            "private-token-key",
            "private-value",
            "changed",
            "_latency",
        ] {
            assert!(!text.contains(private));
        }
        for value in [
            json!({"url":"https://provider.invalid/?token=secret"}),
            json!({"state":{"nodes":["ss://secret@server.invalid:1"]}}),
            json!({"error":"password=secret"}),
            json!({"name":"a private node"}),
            json!({"Bad-Key":1}),
        ] {
            assert!(public(&value, &BTreeSet::new()).is_err());
        }
        public(
            &json!({"status":"passed","sha256":"a".repeat(64),"bytes":123,"cpu_s":0.25}),
            &BTreeSet::new(),
        )
    }
    #[test]
    fn import_suffix_fragment_arguments_and_raw_source_inspection_preserve_configuration() -> Result
    {
        let original = json!({"name":"demo","displayName":"name_123","display-name":"old","process":[{"args":{"namespace":"private","max_checks":"150"}}]});
        let mut imported = original.clone();
        for (key, value) in [
            ("name", "demo_1792461600123"),
            ("displayName", "name_123_1792461600123"),
            ("display-name", "name_123_1792461600123"),
        ] {
            imported[key] = json!(value);
        }
        import_transform(&original, &imported)?;
        let mut broken = imported.clone();
        broken["process"][0]["args"]["max_checks"] = json!(150);
        assert!(import_transform(&original, &broken).is_err());
        broken = imported;
        broken["name"] = json!("demo_123");
        assert!(import_transform(&original, &broken).is_err());
        let mut config = json!({"name":"demo","url":"https://private.invalid","mergeSources":"remoteFirst","process":[{},{},{},{},{"type":"Script Operator","args":{"mode":"link","content":"https://script.invalid#max_checks=150","arguments":{"max_checks":"150","namespace":"private","refresh_mode":"always"}}},{},{}]});
        let saved = config.clone();
        let actual = configured(&config, "http://127.0.0.1:12345", "full")?;
        let fragment = actual["process"][4]["args"]["content"]
            .as_str()
            .unwrap()
            .split_once('#')
            .unwrap()
            .1;
        let decoded: Value = serde_json::from_str(&crate::fixture::unquote(fragment)?)?;
        assert_eq!(
            decoded,
            json!({"max_checks":"150","namespace":"private","refresh_mode":"always","http_meta_port":"12345"})
        );
        assert_eq!(actual["url"].as_str().unwrap().lines().count(), 22);
        assert_eq!(actual["process"][4]["args"]["mode"], "link");
        assert_eq!(config, saved);
        let mut inspected = inspection(&config);
        assert_eq!(inspected["process"].as_array().unwrap().len(), 1);
        assert!(
            inspected["process"][0]["args"]["content"]
                .as_str()
                .unwrap()
                .contains("return context.raw")
        );
        inspected["process"] = config["process"].take();
        config["process"] = saved["process"].clone();
        assert_eq!(inspected, config);
        Ok(())
    }
    #[test]
    fn paired_statistics_use_blocks_units_and_sufficient_tail_counts() -> Result {
        let result = paired_interval(&[(2.0, 1.0), (4.0, 2.0), (8.0, 4.0)])?;
        assert!((result["node_over_rust"].as_f64().unwrap() - 2.0).abs() < 1e-12);
        assert_eq!(result["pairs"], 3);
        assert!(distribution(&[0.01, 0.02])?.get("p95").is_none());
        assert_eq!(
            distribution(&(1..=100).map(|value| value as f64).collect::<Vec<_>>())?["p95"],
            95.0
        );
        assert!(paired_interval(&[(0.0, 1.0)]).is_err());
        assert_eq!(shards().len(), 35);
        Ok(())
    }
    #[test]
    fn pipeline_events_reject_private_incomplete_extra_and_unbalanced_data() -> Result {
        let row = |id, parent: Value, stage: &str, active, own| json!({"id":id,"parent":parent,"stage":stage,"index":null,"calls":1,"wall_ns":active+10,"active_ns":active,"self_ns":own,"max_wall_ns":active+10});
        let report = json!({"version":1,"job":1,"operation":"download_sub","success":true,"wall_ns":200,"dropped":0,"open_scopes":0,"unfinished_spans":0,"rows":[row(0,Value::Null,"job",100,30),row(1,json!(0),"processor",70,70)]});
        validate_profile(&report, "download_sub", true)?;
        for (key, value) in [
            ("dropped", json!(1)),
            ("unfinished_spans", json!(1)),
            ("success", json!(false)),
            ("rows", json!([{ "private":"sentinel"}])),
        ] {
            let mut changed = report.clone();
            changed[key] = value;
            assert!(validate_profile(&changed, "download_sub", true).is_err());
        }
        for (key, value) in [
            ("self_ns", json!(60)),
            ("stage", json!("private-sentinel")),
            ("parent", json!(1)),
            ("calls", json!(true)),
        ] {
            let mut changed = report.clone();
            changed["rows"][1][key] = value;
            assert!(validate_profile(&changed, "download_sub", true).is_err());
        }
        let root = crate::process::scratch("performance-profiles")?;
        let path = root.join("server.log");
        let prefix = b"private-prefix\nSUB_STORE_PROFILE invalid old event\n";
        let mut bytes = prefix.to_vec();
        bytes.extend_from_slice(b"private-log-line\nSUB_STORE_PROFILE ");
        bytes.extend_from_slice(&canonical(&report));
        bytes.push(b'\n');
        std::fs::write(&path, bytes)?;
        assert_eq!(
            pipeline_profiles(&path, prefix.len() as u64, "download_sub", true)?,
            vec![report]
        );
        std::fs::write(&path, b"SUB_STORE_PROFILE {broken\n")?;
        assert!(pipeline_profiles(&path, 0, "download_sub", true).is_err());
        Ok(())
    }
}
