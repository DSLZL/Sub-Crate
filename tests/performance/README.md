# Performance comparisons

Run the two workflows on the branch being reviewed. Builds, services, browsers,
self-checks and measurements run only in GitHub Actions; the Rust harness rejects
local execution. cargo-nextest drives `tests/performance.rs`, with separate JUnit
profiles for preparation, pairs, aggregation, comparisons and inspection.

## Supplied subscription

[Supplied subscription performance](../../.github/workflows/rust-performance.yml)
uses the original file through Sub-Store's actual UI file-import feature, five
paired imports per backend. It checks the submitted fields, permitted timestamp
suffix, UI reload/editor state and persisted data after restart.

| Input | Fixed identity |
|---|---|
| Node backend | `9cc6b5fac966993364ae33da8797f446bbf7f571` |
| Rust candidate | `af08b0649c4d2f2814fe38ce6943b1397f5e1c14` |
| Diagnostic Rust baseline | `ee64ab9f7376a68c126950caee256bf3fb9c2b12` |
| Unchanged frontend | `4bd7b0acbe65466b933ccea14fc61dbebe036ead` |
| Subscription SHA-256 | `a6e6bb9cbe0587f33de0cf130ec391ecffd3483009888fed0ea59d39492ae890` |

`BENCHMARK_SUBSCRIPTION_B64` is the existing repository secret containing the
unchanged subscription file. Preparation captures 22 decoded remote source
strings once through Node before preprocessing and retains the local content as
the 23rd string. Every worker verifies exact ordered replay on both backends.
This excludes provider network/decryption variability while retaining normal
preprocessing, parsing, processing, scripts, production, caches and persistence.

Five guarded edits remove only maintenance scheduling/batching delays from the
pinned health script; `stress.patch` records them. The normal run contains 35
independent same-runner pairs (70 cells), with up to 20 jobs in parallel. Pair
ordering alternates. Cold/warm, pressure, no-publisher and the separate real
HTTP-META/Mihomo cell retain their original work and correctness checks.

- `diagnose_parse`: 44 direct-source observations and full-input parse/pre-health
  cells, with private-safe mismatch paths, types and hashes.
- `diagnose_health`: pre-health, cold, warm and no-publisher cells on four
  variants, including the frozen baseline and a repeated candidate.
- `profile_pipeline` with `diagnose_health`: the same candidate binary with
  profiling off/on. This uses the five-site stress script without the separate
  JavaScript timing wrappers. Profile-on samples remain diagnostic.
- `inspect_run` and `inspect_attempt`: read a specific existing sanitized result
  on the runner without fetching provider inputs again.

The controlled detector allocates a distinct port per node identity and checks
probe counts, retries, start/stop and cleanup. Work validation requires current,
nonempty snapshots and no skipped cold work or warm-cache miss. Semantic hashes
exclude only top-level `_latency`; parse-only hopping accepts a port variation
only within unchanged valid ranges. Credentials, node order and other metadata
remain part of the comparison. Failed cells keep metrics and cannot yield
speedup estimates. Aggregation rejects missing/duplicate cells, changed sources,
wrong attempts and mismatched work or output hashes.

CPU/RSS/IO measurements track the actual owned process. RSS samples include their
observed intervals and distinguish cell peak, process-lifetime peak, end and
settled memory. Driver/fixture/detector overhead is recorded separately. Paired
confidence intervals resample block log-ratios 2000 times; P95 requires at least
20 samples. Hosted runners may differ in CPU model or contention, so the two
backends share each measured worker.

Raw input, HAR, credentials, state and child logs remain in private ephemeral
scratch. Shared inputs use the existing AES-GCM/scrypt helper and run/input
authentication. This Rust harness uses a validated **version 2 JSON/base64 capsule**
payload instead of the retired tar payload; capsules are not interchangeable
across harness versions. Publication accepts code-owned labels, numeric fields
and hashes only. Reports retain the harness SHA separately from application SHAs.
The deterministic bootstrap generator also changed with the Rust harness; do
not present harness-only timing or interval differences as backend improvements.

## Single-node protocols

[Single-node parsing comparison](../../.github/workflows/rust-single-node.yml)
uses `single-node.json`: 48 public fixtures covering every pinned Clash node type
and Surge external. The candidate is the workflow source SHA; the separate Rust
baseline is `9c19111bae2af2654caa339151e4eabd12b586bb`.

It measures seven alternating blocks of pure parsing with reused/fresh parsers,
first conversion, full HTTP with new/persistent connections, isolated 1/8-client
load, retained RSS and separate lifecycle/stage profiles. Equivalent original
and newline inputs alternate to avoid last-input preprocessing cache hits.
Persistent sockets cannot silently reconnect. Every response must match Node.
Interactive parser helpers use build-only Cargo artifact discovery; all normal
Rust test execution uses nextest.

`diagnose_tail` runs Hy2 and SS controls in three fresh-server rounds, 200
requests per block and 1000 separate profile-on requests per fixture. Tail
inspection preserves round/block/sample identity and pairs client, server,
worker and stage observations. It waits for completed teardown events and
rejects missing, extra, corrupt or private profile fields. These timings measure
configuration conversion, not traffic through the protocols.

[HISTORY.md](HISTORY.md) preserves previous measurements and failures under their
original source/harness versions. They are historical results, not acceptance
of this migrated harness.
