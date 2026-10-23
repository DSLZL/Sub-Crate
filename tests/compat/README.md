# Compatibility tests

[Rust service and compatibility](../../.github/workflows/rust-compat-native.yml)
runs the independent Rust harness through cargo-nextest. It requires all six
jobs on one source revision: three native platforms, two native Docker
architectures and the unchanged frontend in headless Chrome. The complete
contract is [rust/ACCEPTANCE.md](../../rust/ACCEPTANCE.md).

The native jobs call the reusable [Rust test workflow](../../.github/workflows/rust-tests.yml).
They run formatting, clippy, harness self-checks and production Rust units before
the actual service contracts. They also execute all 826 original Node unit tests,
the reference HTTP suites and the retained script probes. The negative checks
reject missing cases, typed state differences, corrupt payloads, altered wire
requests, incomplete cleanup and omitted observations on both implementations.

The reference bootstrap restores backend
`9cc6b5fac966993364ae33da8797f446bbf7f571` on the runner and permits only
`approved-reference-fixes.patch` (RF01/RF02). Node **24.16.0** and pnpm
**11.0.9** are development tooling. Rust production execution excludes Node
builtins, npm loading and Node-specific user scripts.

The browser job builds official frontend
`4bd7b0acbe65466b933ccea14fc61dbebe036ead` without source changes. It packages
and extracts the frontend and Rust backend, then runs all 21 declared flows
against Node, merged Rust and separate Rust. Import/upload, HAR requests, backend
switches, persistence, deletion and restart checks use the actual UI.

Docker jobs build on native x64/arm64 runners, verify license provenance, save
and extract each image, remove the original tag and load the saved image before
service acceptance. The contained probe checks its actual platform. Its cleanup
regression exercises success, watchdog timeout and launch failure while preserving
another container owned by the same test.

`native.rs`, `frontend.rs` and `docker.rs` contain the nextest entries;
[`../src/`](../src/) owns fixtures and assertions. Keep these fixed inputs here:

- `manifest.json`, `fixtures/http-cases.json`, `fixtures/script-cases.json` and
  `frontend-contract-map.json` define finite required inventories.
- JavaScript probes and `*-reference.cjs` exercise the frozen application's
  actual helpers. Build scripts generate test-only bundles on the runner.
- DNS certificates, MMDB databases/vectors and license files carry independent
  byte/hash/provenance checks.
- `package.json` and `pnpm-lock.yaml` pin reference-only JavaScript dependencies.

All commands are Actions-only; see [tests/README.md](../README.md). Review logs
remotely. Generated helpers, installed dependencies, evidence and compiler output
belong on the runner. Product downloads are the separate `m5-*` artifacts;
`m1-*` and nextest JUnit record execution and failures.

[HISTORY.md](HISTORY.md) preserves the earlier discovery, experiments and their
original commands byte-for-byte. Its pending statements and retired Python
commands describe those historical revisions. [HISTORY-CLEANUP.md](HISTORY-CLEANUP.md)
records the earlier removal of unrelated Web API experiments.
