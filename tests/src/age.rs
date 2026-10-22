use crate::{Result, json, probe, process, reference, require, server};
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::path::Path;
use std::process::Command;
use std::time::Duration;

const TYPES: [&str; 2] = ["x25519", "mlkem768-x25519"];
const PAYLOADS: [(&str, usize, &str); 4] = [
    ("empty", 0, "d41d8cd98f00b204e9800998ecf8427e"),
    ("unicode", 17, "71345ef34d87f1f2200738a8af6fd5f2"),
    ("view", 13, "7b6acbd705745601c03e09bd53439a9e"),
    ("chunk-boundary", 65542, "26203b26926625632f97f95f64df380d"),
];
pub fn check_run(result: &Value, progress: &Value, full: bool, directions: &[&str]) -> Result {
    let ids = if full {
        probe::identifiers("age")?
    } else {
        vec![]
    };
    json::check(&result["passed"], &json!(ids), false)?;
    json::check(
        &json!(
            result["observations"]
                .as_object()
                .ok_or("Missing age observations")?
                .keys()
                .collect::<Vec<_>>()
        ),
        &json!(ids),
        false,
    )?;
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
    )?;
    let expected=directions.iter().flat_map(|direction| TYPES.iter().flat_map(move |kind| PAYLOADS.map(|(id,length,md5)| json!({"direction":direction,"type":kind,"id":id,"utf8Length":length,"md5":md5})))).collect::<Vec<_>>();
    json::check(&result["verified"], &json!(expected), false)
}

pub fn run(output: &Path, executable: &Path) -> Result {
    let stages = [
        ("node-age-generate", "node", "full", None, None, vec![]),
        (
            "rust-age-exchange",
            "rust",
            "full",
            Some(0),
            None,
            vec!["node-key-to-rust"],
        ),
        (
            "node-age-reply",
            "node",
            "reply",
            Some(1),
            Some(0),
            vec!["rust-key-to-node", "rust-to-node-key"],
        ),
        (
            "rust-age-finish",
            "rust",
            "finish",
            Some(2),
            Some(1),
            vec!["node-to-rust-key"],
        ),
    ];
    let mut exchanges = Vec::<Value>::new();
    let mut runs = Vec::new();
    let mut observations = None;
    for (name, kind, mode, peer, own, directions) in stages {
        let state = output.join(format!("{name}-state"));
        std::fs::create_dir(&state)?;
        for file in ["sub-store.json", "sub-store-probe.json", "root.json"] {
            json::write_new(state.join(file), &json!({}))?;
        }
        let mut input = json!({"mode":mode});
        if let Some(index) = peer {
            input["peer"] = exchanges[index].clone();
        }
        if let Some(index) = own {
            input["own"] = exchanges[index].clone();
        }
        json::write_new(state.join("age-input.json"), &input)?;
        let mut environment = server::environment(&state);
        server::set(&mut environment, "SUB_STORE_COMPAT_STATE_DIR", &state);
        server::set(
            &mut environment,
            "SUB_STORE_COMPAT_UPSTREAM",
            "http://127.0.0.1:1",
        );
        let mut command = if kind == "node" {
            let mut command = Command::new(reference::node());
            command
                .arg("--require")
                .arg(crate::compat().join("register.cjs"))
                .arg(crate::compat().join("age-reference.cjs"));
            command
        } else {
            let mut command = Command::new(executable);
            command.arg("--age");
            command
        };
        command
            .env_clear()
            .envs(environment)
            .current_dir(if kind == "node" {
                crate::root().join("backend")
            } else {
                crate::root()
            });
        let result = process::run_direct(
            &mut command,
            output,
            name,
            Duration::from_secs(60),
            "Direct age process ownership allows watchdog cleanup on Windows",
        )?;
        require(
            result.success(),
            format!("{name} failed: {}", result.stderr),
        )?;
        let progress = result
            .stderr
            .lines()
            .filter_map(|line| line.strip_prefix("COMPAT_AGE "))
            .map(json::parse)
            .collect::<Result<Vec<_>>>()?;
        let value = if kind == "node" {
            let lines = result
                .stdout
                .lines()
                .filter_map(|line| line.strip_prefix("COMPAT_AGE_RESULT "))
                .collect::<Vec<_>>();
            require(lines.len() == 1, "Missing or duplicate age result")?;
            json::parse(lines[0])?
        } else {
            json::parse(result.stdout.lines().last().ok_or("Missing age result")?)?
        };
        check_run(&value, &json!(progress), mode == "full", &directions)?;
        if mode == "full" {
            if let Some(previous) = &observations {
                json::check(&value["observations"], previous, false)?;
            } else {
                observations = Some(value["observations"].clone());
            }
        }
        let exchange = json::read(state.join("age-output.json"))?;
        json::check(&exchange["sender"], &json!(kind), false)?;
        let record = json!({"name":name,"exit_code":result.status.code(),"progress":progress,"result":value,"exchange_sha256":process::sha256(&std::fs::read(state.join("age-output.json"))?),"input_sha256":process::sha256(&std::fs::read(state.join("age-input.json"))?),"command":json::read(output.join(format!("{name}.command.json")))?});
        json::write_new(output.join(format!("{name}.json")), &record)?;
        runs.push(record);
        exchanges.push(exchange);
    }
    check_artifacts(output, &runs)?;
    json::write_new(
        output.join("age.json"),
        &json!({"runs":runs,"key_types":TYPES,"fresh_processes":4,"verified_plaintexts":32}),
    )
}

pub fn check_artifacts(output: &Path, runs: &[Value]) -> Result {
    let names = [
        "node-age-generate",
        "rust-age-exchange",
        "node-age-reply",
        "rust-age-finish",
    ];
    json::check(
        &json!(runs.iter().map(|run| &run["name"]).collect::<Vec<_>>()),
        &json!(names),
        false,
    )?;
    let directions = [
        vec![],
        vec!["node-key-to-rust"],
        vec!["rust-key-to-node", "rust-to-node-key"],
        vec!["node-to-rust-key"],
    ];
    let mut exchanges = Vec::new();
    let mut inputs = Vec::new();
    let mut logs = Vec::new();
    for (index, run) in runs.iter().enumerate() {
        let name = names[index];
        let state = output.join(format!("{name}-state"));
        let command = json::read(output.join(format!("{name}.command.json")))?;
        json::check(&command, &run["command"], false)?;
        json::check(
            &json!([command["exit_code"], command["timed_out"], run["exit_code"]]),
            &json!([0, false, 0]),
            false,
        )?;
        let stdout = std::fs::read_to_string(output.join(format!("{name}.stdout.log")))?;
        let stderr = std::fs::read_to_string(output.join(format!("{name}.stderr.log")))?;
        let progress = stderr
            .lines()
            .filter_map(|line| line.strip_prefix("COMPAT_AGE "))
            .map(json::parse)
            .collect::<Result<Vec<_>>>()?;
        json::check(&json!(progress), &run["progress"], false)?;
        let value = json::parse(if index % 2 == 0 {
            let lines = stdout
                .lines()
                .filter_map(|line| line.strip_prefix("COMPAT_AGE_RESULT "))
                .collect::<Vec<_>>();
            require(lines.len() == 1, "Missing or duplicate raw age result")?;
            lines[0]
        } else {
            stdout.lines().last().ok_or("Missing raw age result")?
        })?;
        json::check(&value, &run["result"], false)?;
        check_run(&value, &json!(progress), index < 2, &directions[index])?;
        for (file, field) in [
            ("age-input.json", "input_sha256"),
            ("age-output.json", "exchange_sha256"),
        ] {
            json::check(
                &json!(process::sha256(&std::fs::read(state.join(file))?)),
                &run[field],
                false,
            )?;
        }
        inputs.push(json::read(state.join("age-input.json"))?);
        let exchange = json::read(state.join("age-output.json"))?;
        json::check(
            &exchange["sender"],
            &json!(if index % 2 == 0 { "node" } else { "rust" }),
            false,
        )?;
        exchanges.push(exchange);
        logs.push(stdout + &stderr);
    }
    json::check(
        &runs[1]["result"]["observations"],
        &runs[0]["result"]["observations"],
        false,
    )?;
    json::check(
        &json!(inputs),
        &json!([{"mode":"full"},{"mode":"full","peer":exchanges[0]},{"mode":"reply","peer":exchanges[1],"own":exchanges[0]},{"mode":"finish","peer":exchanges[2],"own":exchanges[1]}]),
        false,
    )?;
    let mut secrets = Vec::new();
    let mut publics = Vec::new();
    let mut ciphertexts = Vec::new();
    for (index, exchange) in exchanges.iter().enumerate() {
        let expected = [
            vec!["sender", "pairs"],
            vec!["sender", "pairs", "replies"],
            vec!["sender", "replies"],
            vec!["sender"],
        ];
        let mut keys = exchange
            .as_object()
            .ok_or("Invalid age exchange")?
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>();
        keys.sort();
        let mut required = expected[index].clone();
        required.sort();
        require(keys == required, "Age exchange fields differ")?;
        if let Some(pairs) = exchange.get("pairs") {
            let pairs = pairs.as_array().ok_or("Invalid age pairs")?;
            json::check(
                &json!(pairs.iter().map(|pair| &pair["type"]).collect::<Vec<_>>()),
                &json!(TYPES),
                false,
            )?;
            for (kind, pair) in TYPES.iter().zip(pairs) {
                let secret = pair["secret"].as_str().ok_or("Missing age identity")?;
                let public = pair["public"].as_str().ok_or("Missing age recipient")?;
                require(
                    secret.starts_with(if *kind == TYPES[0] {
                        "AGE-SECRET-KEY-1"
                    } else {
                        "AGE-SECRET-KEY-PQ-1"
                    }) && public.starts_with(if *kind == TYPES[0] { "age1" } else { "age1pq1" }),
                    "Age identity/recipient type mismatch",
                )?;
                secrets.push(secret);
                publics.push(public);
            }
        }
        if let Some(replies) = exchange.get("replies") {
            let replies = replies.as_array().ok_or("Invalid age replies")?;
            json::check(
                &json!(replies.iter().map(|pair| &pair["type"]).collect::<Vec<_>>()),
                &json!(TYPES),
                false,
            )?;
            json::check(
                &json!(
                    replies
                        .iter()
                        .map(|pair| &pair["public"])
                        .collect::<Vec<_>>()
                ),
                &json!(
                    exchanges[1 - index % 2]["pairs"]
                        .as_array()
                        .ok_or("Missing peer key pairs")?
                        .iter()
                        .map(|pair| &pair["public"])
                        .collect::<Vec<_>>()
                ),
                false,
            )?;
        }
        for field in ["pairs", "replies"] {
            for pair in exchange
                .get(field)
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                let items = pair["items"].as_array().ok_or("Missing age ciphertexts")?;
                json::check(
                    &json!(items.iter().map(|item| &item["id"]).collect::<Vec<_>>()),
                    &json!(PAYLOADS.map(|(id, _, _)| id)),
                    false,
                )?;
                for item in items {
                    let armor = item["armor"].as_str().ok_or("Missing armor")?;
                    require(
                        armor.starts_with("-----BEGIN AGE ENCRYPTED FILE-----\n")
                            && armor
                                .trim_end()
                                .ends_with("-----END AGE ENCRYPTED FILE-----"),
                        "Invalid age ciphertext armor",
                    )?;
                    ciphertexts.push(armor);
                }
            }
        }
    }
    require(
        secrets.len() == 4
            && secrets.iter().collect::<BTreeSet<_>>().len() == 4
            && publics.iter().collect::<BTreeSet<_>>().len() == 4,
        "Age identities must be independent",
    )?;
    require(
        ciphertexts.len() == 32 && ciphertexts.iter().collect::<BTreeSet<_>>().len() == 32,
        "Age ciphertexts must be unique",
    )?;
    require(
        !secrets
            .iter()
            .any(|secret| logs.iter().any(|log| log.contains(secret))),
        "Fixture identities leaked into console logs",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn age_trace_and_interop_reject_omissions_order_directions_and_plaintext_changes() {
        let ids = probe::identifiers("age").unwrap();
        let progress = json!(
            ids.iter()
                .flat_map(|id| [
                    json!({"id":id,"event":"start"}),
                    json!({"id":id,"event":"end"})
                ])
                .collect::<Vec<_>>()
        );
        let mut result = json!({"passed":ids,"observations":{},"verified":[]});
        for id in &ids {
            result["observations"][id.as_str().unwrap()] = json!({});
        }
        check_run(&result, &progress, true, &[]).unwrap();
        let mut changed = result.clone();
        changed["passed"].as_array_mut().unwrap().pop();
        assert!(check_run(&changed, &progress, true, &[]).is_err());
        let mut changed = progress.clone();
        changed.as_array_mut().unwrap().swap(0, 1);
        assert!(check_run(&result, &changed, true, &[]).is_err());
        let mut changed = result.clone();
        changed["observations"]
            .as_object_mut()
            .unwrap()
            .shift_remove(ids[0].as_str().unwrap());
        assert!(check_run(&changed, &progress, true, &[]).is_err());
        assert!(check_run(&result, &progress, true, &["wrong-direction"]).is_err());
        result["verified"] =
            json!([{"direction":"x","type":"x25519","id":"empty","utf8Length":0,"md5":"bad"}]);
        assert!(check_run(&result, &progress, true, &["x"]).is_err());
        assert_eq!("订阅🙂\r\n\0尾\n".len(), PAYLOADS[1].1);
        assert_eq!("视图🙂\0\r\n".len(), PAYLOADS[2].1);
    }
}
