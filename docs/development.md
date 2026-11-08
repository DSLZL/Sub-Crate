# Development

## Repository layout

```text
Cargo.toml                 Workspace manifest; shared build profiles
Cargo.lock                 Single lockfile for every crate
Dockerfile                 Production image (build from the repository root)
crates/sub-crate/          The backend
  src/main.rs              HTTP server, routing and configuration
  src/api.rs               API handlers
  src/state.rs             Data files, migration, backup and restore
  src/conversion.rs        Proxy parsing and conversion entry points
  src/clash.rs, surge.rs, classic.rs   Input parsers
  src/output/              Output producers, one per client family
  src/script.rs, script/   JavaScript runtime and the helpers scripts call
  src/scheduler.rs, cron.rs            Scheduled jobs
  vendor/                  Pinned third-party JavaScript utilities and their licenses
  examples/                Script probe and license collector used by CI
tests/                     Acceptance and performance harness (crate sub-crate-tests)
  compat/                  Fixtures and the frozen Sub-Store reference
  performance/             Benchmark fixtures and records
docs/                      User and project documentation
.github/                   Workflows, shared actions, release notes, benchmark chart
```

## Build and test

CI runs every build and test, so a local toolchain is optional. To work locally, install Rust 1.98.0 and [cargo-nextest](https://nexte.st/):

```sh
cargo build -p sub-crate --release      # target/release/sub-crate
cargo nextest run -p sub-crate          # unit tests
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all
```

Unit tests live next to the code in `#[cfg(test)]` modules and are not compiled into release binaries. Most of the `tests/` harness needs prepared reference builds and only runs in GitHub Actions; see [tests/README.md](../tests/README.md).

## Continuous integration

| Workflow | Trigger | Jobs |
| --- | --- | --- |
| [Rust service and compatibility](../.github/workflows/rust-compat-native.yml) | Push to any branch that touches code, manual | Format and workflow lint; build and unit tests on six targets; acceptance on three; Docker on two; frontend browser flows |
| [Release](../.github/workflows/release.yml) | `v*` tag | Full acceptance, then GitHub release and GHCR images |
| [Single-node comparison](../.github/workflows/rust-single-node.yml) | Harness changes on `main`, manual | Per-protocol benchmark against Sub-Store |
| [Supplied subscription](../.github/workflows/rust-performance.yml) | Manual | Real-subscription benchmark |
| [Benchmark chart](../.github/workflows/benchmark-chart.yml) | After a release or benchmark | Redraws `assets/benchmark.png` |

Targets and runners:

| Target | Runner | Unit tests | Acceptance | Released |
| --- | --- | --- | --- | --- |
| Linux x64 | `ubuntu-26.04` | Yes | Native and Docker | Yes |
| Linux arm64 | `ubuntu-26.04-arm` | Yes | Native and Docker | Yes |
| Windows x64 | `windows-2025-vs2026` | Yes | Native | Yes |
| Windows arm64 | `windows-11-vs2026-arm` | Yes | No | No |
| macOS x64 | `macos-26-intel` | Yes | No | No |
| macOS arm64 | `macos-26` | Yes | No | No |

Every job runs on a native runner, so no target is cross-compiled. The shared [rust-setup](../.github/actions/rust-setup/action.yml) action installs the pinned toolchain and nextest, compiles through [sccache](https://github.com/mozilla/sccache) backed by the Actions cache, and restores dependency builds with [rust-cache](https://github.com/Swatinem/rust-cache). Docker builds keep their layers in the Actions cache, and the Dockerfile compiles dependencies in a separate layer so code changes do not rebuild them.

A new push to a branch cancels that branch's running CI. Runs on `main` and tags are never cancelled midway.

## Release

1. Update `version` in `crates/sub-crate/Cargo.toml`, then run `cargo update -p sub-crate` to refresh `Cargo.lock`.
2. Write the release notes in `.github/releases/vX.Y.Z.md`.
3. Merge to `main` and push the tag `vX.Y.Z`. The tag must point at a commit on `main`.

The release workflow reruns acceptance on the tagged commit. It then publishes the Linux x64, Linux arm64 and Windows x64 archives with `SHA256SUMS`, and pushes the exact images that passed acceptance to `ghcr.io/dslzl/sub-crate` as `vX.Y.Z`, `X.Y.Z` and `latest`. A failed upload leaves a draft release that a rerun completes. GHCR makes a new package private; set it to public once in the package settings.
