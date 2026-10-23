# Tests

Tests run in GitHub Actions. Keep local checkouts free of build products,
installed test dependencies and downloaded runtime evidence.

The independent `tests/Cargo.toml` package drives service, browser, container and
performance checks with **cargo-nextest 0.9.149**, Rust **1.98.0** and a locked
dependency graph. The production crate stays independent of test dependencies.
The frozen Node application and unchanged official frontend remain test oracles.

| Workflow | Required execution |
|---|---|
| [Compatibility](../.github/workflows/rust-compat-native.yml) | Linux x64/arm64 and Windows x64; native Docker x64/arm64; 21 UI flows in each of three hosting modes |
| [Single-node comparison](../.github/workflows/rust-single-node.yml) | All 48 protocol fixtures, pure/setup/HTTP measurements, isolated load, lifecycle profiles and tail inspection |
| [Supplied subscription](../.github/workflows/rust-performance.yml) | Real file imports, one source capture per run, 35 independent paired jobs, diagnostics and strict aggregation |

Dispatch these workflows from the branch being reviewed. See
[compatibility](compat/README.md) and [performance](performance/README.md) for
their fixed inputs, selections and evidence boundaries. An older passing run or
a self-check-only result does not accept a new source revision.

Runner entry points, after the workflow's setup and build steps:

```sh
cargo +1.98.0 nextest run --manifest-path rust/Cargo.toml --locked --all-targets --config-file .config/nextest.toml --profile production
cargo +1.98.0 nextest run --manifest-path tests/Cargo.toml --locked --lib --config-file .config/nextest.toml --profile ci
cargo +1.98.0 nextest run --manifest-path tests/Cargo.toml --locked --test native --config-file .config/nextest.toml --profile acceptance
```

Use the workflow's narrower delivery filters before native acceptance. Frontend,
Docker and performance targets require their own prepared binaries, environment
and explicit selection; do not run every integration target as a generic unit
test command. Each workflow invocation has a separate JUnit profile.

| Path | Contents |
|---|---|
| `src/` | Rust fixtures, assertions, owned process cleanup and measurement code |
| `native.rs`, `frontend.rs`, `docker.rs` | Actual application and delivery test entries |
| `performance.rs`, `performance_fixture.rs` | Benchmark entries and the controlled detector child |
| `compat/` | Fixed protocol/state fixtures, certificates, license provenance and JavaScript reference probes |
| `performance/` | Public single-node fixtures, the parser worker and authenticated capsule helper |

Raw subscriptions, credentials, state and private benchmark logs stay on the
ephemeral runner. Only sealed private inputs and allowlisted metrics leave it.
Keep failed Actions attempts and unique historical evidence under their original
source/version labels; fetch only a specific small report when reviewing them.
