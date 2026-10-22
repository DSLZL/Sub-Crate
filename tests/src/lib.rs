pub mod json;
pub mod http;
pub mod process;
pub mod fixture;
pub mod providers;
pub mod dns;
pub mod upstream;
pub mod contracts;
pub mod notices;
pub mod server;
pub mod reference;
pub mod mmdb;
pub mod probe;
pub mod distribution;
#[cfg(test)]
mod notices_tests;

use std::path::PathBuf;

pub type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

pub fn require(condition: bool, message: impl Into<String>) -> Result {
    if condition {
        Ok(())
    } else {
        Err(message.into().into())
    }
}

pub fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_owned()
}

pub fn compat() -> PathBuf {
    root().join("tests/compat")
}

pub fn actions_only() {
    assert_eq!(
        std::env::var("GITHUB_ACTIONS").as_deref(),
        Ok("true"),
        "Program tests run in GitHub Actions only"
    );
}
