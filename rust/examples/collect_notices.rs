#[path = "../../tests/src/notices.rs"]
mod notices;

fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let args: Vec<_> = std::env::args_os()
        .skip(1)
        .map(std::path::PathBuf::from)
        .collect();
    if args.len() != 3 {
        return Err("Expected Cargo metadata, delivery directory and Rust sysroot paths".into());
    }
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_owned();
    let metadata = serde_json::from_slice(&std::fs::read(&args[0])?)?;
    notices::collect(
        &metadata,
        &args[1],
        &args[2],
        &root,
        &root.join("tests/compat/fixtures/licenses/rquickjs-v0.14.0-LICENSE"),
    )?;
    Ok(())
}
