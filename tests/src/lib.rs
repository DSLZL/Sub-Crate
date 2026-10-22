pub mod age;
pub mod api;
pub mod backup;
pub mod browser;
pub mod business;
pub mod contracts;
pub mod distribution;
pub mod dns;
pub mod download;
pub mod edges;
pub mod fixture;
pub mod frontend;
pub mod http;
pub mod json;
pub mod media;
pub mod mmdb;
pub mod notices;
#[cfg(test)]
mod notices_tests;
pub mod notifications;
pub mod persistence;
pub mod probe;
pub mod process;
pub mod providers;
pub mod raw_http;
pub mod reference;
pub mod scheduling;
pub mod server;
pub mod service;
pub mod sharing;
pub mod static_files;
pub mod upstream;
pub mod unicode;

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
