//! Interactive release-only driver for the actual native parser, not a parser copy.
use serde_json::{Value, json};
use std::hint::black_box;
use std::io::{BufRead, Write};
use std::time::Instant;

#[test]
#[ignore = "single-node comparison in GitHub Actions"]
fn compare() {
    assert_eq!(std::env::var("GITHUB_ACTIONS").as_deref(), Ok("true"));
    assert!(
        !black_box(cfg!(debug_assertions)),
        "release benchmark required"
    );
    let fixtures: Vec<Value> = serde_json::from_str(
        &std::fs::read_to_string(std::env::var("SINGLE_NODE_FIXTURES").unwrap()).unwrap(),
    )
    .unwrap();
    let parser = crate::clash::Parser::new().ok().unwrap();
    for line in std::io::stdin().lock().lines() {
        let command: Value = serde_json::from_str(&line.unwrap()).unwrap();
        let original = fixtures[command["case"].as_u64().unwrap() as usize]["raw"].clone();
        // Alternate equivalent text so the last-input YAML cache cannot skip preprocessing.
        let raw = [
            original.clone(),
            Value::String(format!("{}\n", original.as_str().unwrap())),
        ];
        let iterations = command["iterations"].as_u64().unwrap();
        let fresh = command["fresh"].as_bool().unwrap_or(false);
        let parse = |index: u64| {
            crate::conversion::parse(
                black_box(&raw[index as usize % 2]),
                (!fresh).then_some(&parser),
            )
        };
        for index in 0..command["warmup"].as_u64().unwrap_or(0) {
            black_box(parse(index).ok().unwrap());
        }
        let started = Instant::now();
        let mut checksum = 0;
        for index in 0..iterations {
            checksum += black_box(parse(index).ok().unwrap()).len();
        }
        let elapsed_ns = started.elapsed().as_nanos() as u64;
        let outputs = [parse(0).ok().unwrap(), parse(1).ok().unwrap()];
        println!(
            "SINGLE_NODE {}",
            json!({"elapsed_ns": elapsed_ns, "iterations": iterations,
                "checksum": checksum, "outputs": outputs})
        );
        std::io::stdout().flush().unwrap();
    }
}
