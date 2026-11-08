# Tests

The `tests/` crate (`sub-crate-tests`) checks Sub-Crate against the original Sub-Store backend and frontend. It starts real processes, browsers and containers, and it refuses to run outside GitHub Actions.

| Entry | What it runs |
| --- | --- |
| `native.rs` | Service contracts against a packaged native binary |
| `docker.rs` | The same contracts against a reloaded Docker image |
| `frontend.rs` | Browser flows with the unchanged frontend |
| `performance.rs` | Benchmarks; see [performance/README.md](performance/README.md) |
| `src/` | Shared fixtures, assertions, process control and harness self-tests |
| `compat/` | Reference data and probes; see [compat/README.md](compat/README.md) |

CI first runs the harness self-tests:

```sh
cargo nextest run -p sub-crate-tests --lib --profile ci --config-file .config/nextest.toml
```

Each CI step has its own nextest profile in [.config/nextest.toml](../.config/nextest.toml), which also sets its JUnit report path under `target/nextest/`. For the exact commands and environment, read the workflow that runs the step: [rust-tests.yml](../.github/workflows/rust-tests.yml) for native contracts and [rust-compat-native.yml](../.github/workflows/rust-compat-native.yml) for Docker and the frontend.

Subscriptions, credentials and raw logs stay on the runner. Only reports and allowlisted metrics are uploaded.
