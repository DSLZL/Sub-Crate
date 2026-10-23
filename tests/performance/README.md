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

## README release chart

[Update README benchmark chart](../../.github/workflows/benchmark-chart.yml)
reuses the supplied Chart.js design. It renders a PNG because GitHub READMEs
cannot execute Chart.js. Chart code lives in `.github/benchmark/`; measurement,
aggregation, rendering, self-checks and publication all run in Actions.

The chart follows GitHub's latest published, non-prerelease `vMAJOR.MINOR.PATCH`
release. After a successful Release workflow, it publishes a waiting state and
dispatches the single-node comparison on `main` with `release_tag` set to the
latest tag. The workflow resolves that tag to a commit before checking out the
Rust application. The measurement harness has its own recorded source SHA;
adding a workload does not move a published tag or substitute development
application code. Its Node reference remains
`9cc6b5fac966993364ae33da8797f446bbf7f571`.

The coordinator waits for the exact dispatched run and explicitly dispatches
chart publication after success, without relying on a bot-triggered
`workflow_run` callback. To reuse a completed comparison, dispatch the chart
workflow with its `benchmark_run` ID and `benchmark_attempt`; the same latest-tag,
source, success and artifact checks still apply, without repeating measurements.

Only a successful, same-repository comparison dispatched on trusted `main` can
supply measurements. Aggregation verifies that the measured application is the
latest tag, and that the report's harness SHA matches the originating run.
Ordinary development runs, older tags, failed/partial reports, inspection runs
and three-fixture tail diagnostics are excluded. Publication rechecks the latest
release and peeled tag SHA; an older run/attempt or delayed waiting state cannot
overwrite a newer completed result. Chart-only commits do not trigger additional
benchmarks. A failed comparison leaves the waiting state visible instead of
retaining a previous version's measurements. Release measurements require
correctness, not a Rust speedup; slower results remain eligible for publication.

| Column | Aggregation | Unit | Better |
| --- | --- | --- | --- |
| Pure parsing | Median of 48 fixture medians, each from seven block averages with a reused parser | µs/call | Lower |
| Bulk conversion | Median of four format medians, each from seven paired block means | ms/10k input nodes | Lower |
| JavaScript pipeline | Median of seven paired block means | ms/10k input nodes | Lower |
| 8-client throughput | Median of three mixed-fixture blocks | Requests/second | Higher |
| CPU efficiency | Same four-format aggregation as bulk conversion, using whole-process CPU per completed request | CPU ms/10k input nodes | Lower |
| Settled RSS | Median of three 1-client blocks | MiB | Lower |

The batch contains 10,000 public SS/VMess/Trojan nodes, including duplicate pairs.
Both backends use an 8 MiB JSON body limit to import the batch before timing.
It uses local content; no subscription provider, DNS lookup or external script
download is part of these workloads. Built-in renaming precedes ClashMeta, Surge,
QX and sing-box output. Each exported result must retain all 10,000 node names
and match Node's output. The JavaScript pipeline uses the same general JS script
on both backends to filter ports, deduplicate and rename; exactly 4,000 output
nodes must match Node. Its duration includes normal parsing, script loading,
bridging and JSON production, not just a JavaScript function microbenchmark.

Each workload warms up five times on both backends, then alternates backend
order over seven blocks. Equivalent original/newline inputs alternate to avoid
last-input preprocessing cache hits; normal runtime and script reuse remain
enabled. Requests use persistent loopback connections. Their complete durations
include local HTTP transport and response encoding, but are not separate HTTP
latency scores. Every timed response is validated outside its wall-clock timer.

CPU comes from the owned backend process's Linux user/system counters, covering
all its threads and excluding the driver. Each block completes at least four
requests and accumulates at least one CPU second, bounded by 256 requests and
60 seconds. The score uses CPU milliseconds per completed 10,000-node request,
not CPU percentage, sampler overhead, or profiler stage duration. Raw blocks
retain sample counts, both input/output hashes and the JS script hash.

Each orange bar is 100 points. For blue bars, time and memory use
`100 × Node / Rust`; throughput uses `100 × Rust / Node`. Higher scores are better
for every metric: 200 points means twice the throughput, half the time or half
the memory use of the Node baseline. The original values and units appear below
each pair. Fixture-level medians summarize these 48 inputs; they are not a claim
about every protocol, large subscriptions, cold start or forwarding traffic.
Both backends run on the same hosted Linux x64 runner, whose hardware and
contention can still vary between runs.

The publisher commits only `assets/benchmark.png` and
`assets/benchmark-data.json`. The schema-2 JSON contains six whitelisted numeric summaries,
tag/source/harness identities and the originating run/attempt. Raw artifacts,
subscriptions, credentials and logs are never copied into these assets. Chart
dependencies run in a separate job without write permissions; the publisher
validates the PNG and JSON without executing downloaded code or installing npm
packages. The directly pinned chart dependencies are isolated from the backend.
