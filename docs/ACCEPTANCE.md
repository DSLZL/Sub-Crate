# Acceptance

A Sub-Crate revision is acceptable only when the [Rust service and compatibility](../.github/workflows/rust-compat-native.yml) workflow passes on that exact commit. A partial run, an older passing commit or a passing probe does not count.

## Baselines

| Reference | Revision |
| --- | --- |
| Sub-Store backend | `2888806251d90d45eabf2baf7a7fd4e8be2a4787` |
| Sub-Store frontend | `4bd7b0acbe65466b933ccea14fc61dbebe036ead` |

The frozen backend is restored on each runner from its commit, with the two approved fixes in `tests/compat/approved-reference-fixes.patch`. The frontend is built from its commit without changes.

## Required jobs

| Job | Passes when |
| --- | --- |
| Format and workflow syntax | `cargo fmt` and actionlint report nothing |
| Build and unit test, six targets | Clippy is clean and all unit tests pass on Linux, Windows and macOS, x64 and arm64 |
| Acceptance, Linux x64/arm64 and Windows x64 | The packaged binary passes every service contract below |
| Docker, Linux x64/arm64 | The saved, reloaded image passes the same contracts |
| Frontend | All 21 browser flows pass against Sub-Store, Sub-Crate serving the frontend, and Sub-Crate behind a separate frontend server |

## What the contracts cover

- All 890 original Sub-Store unit tests and its HTTP test suites.
- Request-by-request comparison with the reference backend: responses, stored data, outgoing requests and caches.
- Subscription parsing and conversion for every registered input format and output target.
- Built-in processors, user scripts and every host helper scripts can call.
- Data safety: unknown fields, stored scripts, plain/Base64/age backups, denied or interrupted writes, corrupt files, restarts and rolling back to Sub-Store.
- Delivery: tests run the extracted release archive and the reloaded image, not the build output.

## Intentional differences

- The frontend sees the backend as `Node`, version `2.44.6`, so all its features stay enabled. `/api/utils/env` reports the real implementation under `meta.rust`.
- Node.js-only scripts do not run; see [Scripts](scripts.md). They are kept intact and fail with a script error when invoked.
- Error traces and parser messages come from Rust and differ in wording from Node.js.
- Two reference bugs are fixed: Windows path handling (RF01), and a missing token during artifact restore now returns an error instead of stopping the process (RF02).

## Deliveries

Each successful run uploads `m5-*` artifacts: the three native archives, the two Docker image archives and the frontend archive, each with checksums and a manifest of its source, toolchain and licenses. `m1-*` artifacts keep test evidence, including from failed runs. Artifacts expire; releases are published only by the [Release](development.md#release) workflow.
