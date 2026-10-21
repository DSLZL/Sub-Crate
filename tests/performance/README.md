# Supplied-subscription comparison

Run **Supplied subscription performance** (`rust-performance.yml`) manually on
`codex/rust-server-parity`. All builds, self-checks, browsers, services and
measurements run in GitHub Actions; the harness refuses local execution.

Set `inspect_run` to an earlier run ID to print bounded, already sanitized
field/work diagnostics in Actions without building or downloading bundles locally.
Set `diagnose_health` for a separate short full-input diagnosis: pre-health,
one cold full scan, then three warm/no-publisher requests per backend. It retains
all sources and strict comparisons. Identical diagnostic wrappers count/time
MD5, producer and cache calls with millisecond clock resolution; their extra
write/measurement overhead means these are attribution observations, not the
unmodified full-run performance result. `health-profile.patch` records the
instrumentation separately from the unchanged five-site `stress.patch`.

Select the manual `diagnose_parse` input for a separate short diagnosis. It keeps
the same actual file imports and full captured inputs, measures all 22 sources
individually through local-content preview (no source HTTP), then three complete
download requests each for parse/pre-health. This is labelled diagnostic and
never marks the complete matrix finished. Per-source CPU/count/hash/mismatch
observations are sanitized; it can run alongside the unchanged full baseline.

Set repository Actions Secret `BENCHMARK_SUBSCRIPTION_B64` to base64 of the
original exported file bytes. The required SHA256 is
`a6e6bb9cbe0587f33de0cf130ec391ecffd3483009888fed0ea59d39492ae890`.
Do not commit this file or put its contents in workflow arguments. The secret
is injected only into the measurement step. It is removed from the environment
before application processes start. Private files stay in `$RUNNER_TEMP` and
are never uploaded, cached, or included in console logs.

The workflow freezes original `9cc6b5fa`, Rust `464241c8`, and unchanged frontend
`4bd7b0ac`, with full revisions in the harness and report. It builds the original
without compatibility-reference patches. The actual subscription file input on
`/subs` imports the original file five times per backend into fresh storage.
Browser POST data, frontend timestamp suffixes, every submitted field, all seven
processors, reload/editor observations and restart persistence are checked.
All repeated states originate from these real UI imports.

The retained full baseline at Actions `39402271707` and initial short diagnostic
at `39408480379` use Rust `948c665b`. Their sources and reports are immutable;
later diagnostics identify the candidate above and must not overwrite them.

Only after import acceptance does the ordinary PATCH API apply derived workload
settings. Original names are restored and all 22 source locations are mirrored
on loopback. The original backend's `produceType=raw` route captures decoded
parser inputs once. Empty/failed source bodies are counted explicitly; results
cannot establish performance for content the providers did not return.

The public pinned health script is modified at exactly five asserted sites:
workers, batch size and max checks become the full input length; scan budget is
unbounded; crash cleanup lease becomes 600 seconds, exceeding the 300-second
external request watchdog. `max_checks=0` is **not** used: it disables probing.
`stress.patch` records the full shared diff. Filters, connection deduplication,
protocol compatibility, per-probe timeout/retries and snapshot publication remain
intact. Both backends execute identical script bytes.

Ten alternating pairs probe all eligible identities with fresh health caches in
one batch. A separate asynchronous detector fixture supplies deterministic 204
responses through distinct proxy ports and checks unique completed work. Five
paired warm blocks provide 100 requests per backend. Client concurrency 1/2/4/8
uses three alternating paired 20-second windows. Warm paths still execute all
seven processors and publish snapshots; counters must prove no active probes.
Two unmeasured full warm-up requests precede each warm block; their output/work
counts are retained, including any first-request output loss. Measured requests
retain the original refresh settings and must perform zero new probes.
Parse/produce-only and six-stage no-publisher diagnostics are separate cases.
An additional ClashMeta export is parsed using the original's locked YAML
utility and compared across backends. Real pinned HTTP-META 1.3.0/Mihomo 1.19.32
runs a final full-probe pair with the same captured nodes, reported separately.
This separate test service's Node runtime is not a production Rust dependency.

`summary.md` and `metrics.json` contain allowlisted observations only: source and
output hashes, import proof, work counters, timings, paired block estimates,
server CPU deltas, sampled RSS/actual intervals, Linux VmHWM, I/O, and separate
driver/fixture/detector resource observations. Top-level node `_latency` is
excluded from fingerprints. Only parse-only diagnostics also canonicalize a
numeric selected port inside the unchanged declared hopping ranges, because
the original exporter chooses it randomly. Missing ports, ports outside the
range, changed ranges and other fields still differ; raw hashes are retained.
Snapshot `generatedAt` is not an equivalence
key. Snapshots are compared independently across runtimes. The real JSON producer
is invoked through the public preview API, outside timing, to check that the
pre-producer snapshot corresponds to final output (including legitimate SNI/port
normalization). No broad field removal or copied producer implementation is used.
Output must be nonempty, and cold selected / completed identities must match.
Repeated probe requests are counted separately: the original retry policy remains
active, while unique completed identities must still cover the entire batch.
Raw replay text is also checked on both backends as a diagnostic: preprocessing
can change its representation, so measured output semantics determine correctness.
Parse and pre-health cases run first
to locate processing differences. A product mismatch marks that cell failed but
allows independent cells to run; observed metrics and differing counts remain in
the report, while failed/non-equivalent pairs never enter speedup estimates.
Timeout, crash or incomplete work is a failed
sample; the harness never trims nodes or re-enables maintenance caps to pass.
The Actions deadline is 300 minutes: the retained 90-minute attempt completed
only 33/70 cells, so the full matrix needs a longer execution budget.

RSS is sampled at 50ms; VmHWM is the OS lifetime peak of each fresh process.
CPU seconds use cumulative user+system counters, which have clock granularity.
Paired bootstrap intervals describe observed block variability, with weak
confidence for three-block cells. No P99, cross-platform ranking or production
capacity claim follows from this hosted Linux x64 runner. Replay excludes
provider transport/decryption; real network availability can differ between
rounds and is not a backend speed ratio.

The retained short baseline [39408480379](https://github.com/DSLZL/Sub-Store/actions/runs/39408480379)
matched all individual sources, but Rust's first multi-source request returned
4,811 nodes against 5,785, then recovered. Its parse requests took about 29s.
The shared parser repeatedly used interpreted JSON5 and YAML, including a
second unchanged YAML parse. The final bounded release fixture measured 5,524
JSON5 parses/projections at 1.075s versus 0.182s using built-in JSON. One generic
YAML pass took 3.507s versus 0.023s for the canonical download representation.
Context creation for 23 parsers took 0.249s. These are component measurements,
not whole-request speedup factors.

Candidate `e5890c2a` uses built-in JSON before the existing fallbacks, reuses
unchanged YAML results, and recognizes canonical JSON entries already serialized
by the downloader. It validates exact parse/stringify equality and retains the
generic YAML path for duplicate/noncanonical keys, Unicode escape/separator
semantics and short-id rewriting. No download timeout or input limit changed.

Short diagnostic [39411714863](https://github.com/DSLZL/Sub-Store/actions/runs/39411714863)
passed 14 driver checks, ten actual frontend imports/restarts, all 22 direct
source comparisons, and three requests per backend/case. Capture obtained 20/22
nonempty sources; both backends used the same 5,654 nodes in every parse request.

| Diagnostic | Node median | Rust median | Node CPU/request | Rust CPU/request |
|---|---:|---:|---:|---:|
| Parse/produce | 1.556s | 1.153s | 2.137s | 1.390s |
| Four processors before health | 1.613s | 1.363s | 2.203s | 1.603s |

Peak sampled parse RSS was 403.5MiB for Node and 61.7MiB for Rust. The first
request was approximately 1.90s for both; this small sample does not establish
tail latency or full-scan/concurrent capacity. Different capture hashes and
runner CPU models prevent a controlled cross-run old/new speedup claim. The
complete stress matrix and real detector must still be measured for this
candidate; short diagnostics are not the full comparison.

[Native acceptance 39411699444](https://github.com/DSLZL/Sub-Store/actions/runs/39411699444)
passed all six jobs: Linux x64/arm64, Docker x64/arm64, Windows x64 and the
unchanged frontend. Its `aef3863e` checkout has the identical Rust production
tree to the diagnostic's pinned `e5890c2a` candidate. All product checks ran in
GitHub Actions.

The complete fixed-candidate run [39417720368](https://github.com/DSLZL/Sub-Store/actions/runs/39417720368)
finished all 70 cells. Imports and parse/pre-health pass, with median Node/Rust
latencies of 1.620/1.202s and 1.617/1.460s respectively. Thirty Rust full-pipeline
cells fail output equivalence. Warm observed latency is 1.838/2.662s, but those
failed pairs do not establish equivalent-work speed ratios. Cold Rust probes
retry more (3,663 requests for 2,901 identities in the first pair, versus 2,901
Node requests); measured warm requests perform no new probes.

Remote inspection [39426039773](https://github.com/DSLZL/Sub-Store/actions/runs/39426039773)
locates differences in ws-opts path/early-data fields. Frozen Node ClashMeta
production mutates the shared nested object when the script passes a shallow
proxy copy; Rust normalizes an independent projected object instead. Preserve
this producer-side contract rather than normalizing unrelated parse inputs.

Health attribution [39426859222](https://github.com/DSLZL/Sub-Store/actions/runs/39426859222)
passes all 15 harness checks and exact builds, but retains product correctness
failures. Its capture has 2,978 pre-health nodes on both backends, a different
capture from the full run. In one measured warm invocation:

| Instrumented health helper | Calls on each backend | Node total | Rust total |
|---|---:|---:|---:|
| MD5 | 2,980 | 43ms | 1,057ms |
| Single-node produce | 2,978 | 17ms | 623ms |
| Cache get / set | 1 / 0 | 0ms / 0ms | 0ms / 0ms |
| Entire health operator | 1 | 92ms | 1,866ms |

These coarse wrapper totals identify MD5 and per-node conversion as dominant
warm health costs, not a measured native-replacement speedup. At that revision,
MD5 built JS byte arrays and called the generic JS hash utility. Conversion
projected owned values and recompiled its projection function repeatedly; its
compilation and serialization costs had not been split. Cold observations also
lost verified nodes on Rust; transport initialization/scheduling and exact retry
causes remain unresolved. No production optimization is included in these
diagnostic commits, and no local product execution or artifact expansion occurs.

The subsequent production repair at `9bab7346` replaces the JS MD5 implementation
with pinned RustCrypto md-5 0.11.0, retaining exact UTF-8/isolated UTF-16 hashes and
cache keys. Projection/stringify/restore functions compile once per JS context,
and the shared native Clash-family WebSocket normalizer now updates the caller's
existing nested options, including early-data fields and metadata deletion.
Filtering, aliases, JSON behavior, timeouts and probe work remain unchanged.

Health diagnostic [39431798705](https://github.com/DSLZL/Sub-Store/actions/runs/39431798705)
passes driver checks, builds, actual imports/restarts and pre-health equivalence.
Both backends receive the same 3,096 pre-health nodes. One measured warm operator
records the following helper totals:

| Instrumented health helper | Calls on each backend | Node total | Rust total |
|---|---:|---:|---:|
| MD5 | 3,098 | 38ms | 17ms |
| Single-node produce | 3,096 | 13ms | 254ms |
| Entire health operator | 1 | 87ms | 428ms |

This establishes the paired helper observations, not a whole-pipeline speedup.
The capture differs from the earlier 2,978-node profile. Cold Rust still verifies
only 3,012/3,096 nodes and makes 4,827 probes, versus Node's complete 3,096 probes.
Warm cases inherit partial health caches and remain failed; exact cold retry
causes and the remaining conversion overhead require further attribution.
The complete accepted performance comparison is still unfinished.

[Native compatibility acceptance 39436003557](https://github.com/DSLZL/Sub-Store/actions/runs/39436003557)
passes all six jobs: Linux x64/arm64, Windows x64, Docker x64/arm64 and the
unchanged frontend. Its `18877689` checkout has the identical Rust production
tree to the diagnostic's pinned `9bab7346`. Extended MD5 and shared WebSocket
assertions run in both the actual delivered service and the JS probe; a runner
check prevents their embedded service scripts from drifting from the source
fixtures. All builds, test execution, browsers and containers run in Actions.

Candidate `312cd230` reduces the per-node value-bridge cost: ordinary scalar
projection avoids unused ancestry paths, plain JSON bypasses identity-only
UTF-16 preprocessing, and ordinary owned output avoids restoration copies and
numeric text normalization before engine JSON.parse. Cycles, nonfinite numbers,
negative zero, UTF-16/private keys and current getter/toJSON behavior retain
their existing handling. Shared `grpc-opts` now also receives the original
Clash-family metadata deletions, including internal production. PC97 covers
aliases, shallow identity, JSON/filter non-effects and both output kinds in the
probe and delivered service; the source-drift guard remains required.

[Diagnostic 39448619317](https://github.com/DSLZL/Sub-Store/actions/runs/39448619317)
at harness `9ec3f2ac` compares frozen `5b68cbb9` with this candidate on the same
runner. Each public fixture repeats 3,096 actual producer calls per block; the
table shows three-block medians for candidate, baseline, candidate again.
Context setup and assertions are outside the timed loops. The baseline has an
isolated Cargo target and is visibly recompiled from its own source checkout.

| Public fixture | Baseline total ms | Candidate first ms | Candidate repeat ms | Observed reduction |
|---|---:|---:|---:|---:|
| Flat SS | 88.260 | 69.445 | 71.471 | 19.0-21.3% |
| WebSocket | 144.968 | 109.510 | 107.355 | 24.5-25.9% |
| Nested metadata | 221.442 | 160.295 | 161.365 | 27.1-27.6% |

Projection medians fall about 29-32%, and return-to-JS medians about 54-58%.
Native business conversion remains essentially unchanged. These bounded,
repeated-context observations do not establish a whole-subscription speedup.
The earlier shared-target A/B in 39446530342 is invalid because Cargo reused
the candidate executable; its before/after rows are excluded. Failed/cancelled
evidence is retained, including superseded 39447710450.

The final diagnostic passes actual unchanged-file imports/restarts and all
eight pre-health/cold/warm/no-publisher cells, with 2,846 nodes on each backend.
Capture attempted all 22 sources and obtained 21 nonempty bodies. All 2,846
identities verify and output/snapshot checks pass; shared gRPC differences are
gone. In one warm health invocation, Node/Rust producer totals are 19/137ms
for 2,846 calls, MD5 totals 28/5ms for 2,848 calls, and operator totals 80/258ms.
This capture differs from earlier diagnostics; it is not an old/new helper ratio.

Overall Rust latency remains higher in this short block: cold is 19.717s versus
Node 6.306s (one request), and warm median is 4.068s versus 2.701s (three requests).
Rust makes 4,139 cold probes for the same 2,846 identities versus Node's 2,846;
measured warm probes remain zero. Cold transport/retry cost is unresolved and
unchanged. A successful short diagnostic does not replace the complete matrix,
concurrency or real-detector checks.

[Native compatibility 39448490215](https://github.com/DSLZL/Sub-Store/actions/runs/39448490215)
passes all six jobs at `312cd230`: Linux x64/arm64, Windows x64, Docker x64/arm64
and the unchanged frontend. The ordinary-value regression, extended gRPC source
assertions and delivered-service source guard are included. The current Rust
tree and contract fixtures at that checkpoint are identical to this tested revision.
[Complete matrix 39450897454](https://github.com/DSLZL/Sub-Store/actions/runs/39450897454)
ran at harness `9ec3f2ac` with that candidate and was cancelled when the newer
validated conversion candidate replaced it. Its retained partial observations
do not count as a completed matrix. Its pressure rows at clients4/8 retain
output mismatches (for example Rust2,806/2,759/2,830 versus Node2,842). A newer
short diagnostic cannot establish that those full-concurrency failures are fixed.

The next candidate, `cda93a24`, reduces live-value crossings: object reads use
the engine's property operation, truthiness uses its ToBool, and strict equality
uses its actual comparison API. Primitive property boxing, String conversion,
JS write/delete functions and getter/Proxy effects retain their existing paths.
Rust value-handle equality is not used as JS equality. Clash internal output now
moves the owned result list into its array value, eliminating a full tree copy.
No library, output rule, timeout, retry budget or workload setting is changed.

[Diagnostic 39457177406](https://github.com/DSLZL/Sub-Store/actions/runs/39457177406)
at harness `b42ed623` compares frozen `312cd230` with `cda93a24` using the same
three public fixtures, 3,096 calls per block, three-block medians and an isolated
baseline target. Candidate first/repeat bracket the baseline on the same runner.

| Public fixture | Baseline total ms | Candidate first ms | Candidate repeat ms | Observed reduction |
|---|---:|---:|---:|---:|
| Flat SS | 46.839 | 38.690 | 38.001 | 17.4-18.9% |
| WebSocket | 72.768 | 62.926 | 60.766 | 13.5-16.5% |
| Nested metadata | 114.237 | 96.849 | 96.710 | 15.2-15.3% |

The native stage, including its required owned input clone, falls about 31-45%.
Projection/restoration implementations are unchanged and remain a large cost.
The added JSON internal preparation observation runs after each fixture's
unchanged A/B sequence and takes about 12-13ms per 3,096 calls on this runner.
It excludes platform output normalization and is not an exact decomposition of
the Clash call. No earlier runner's absolute timings form an old/new speed ratio.

The full-input short diagnostic passes actual unchanged-file imports/restarts
and all eight cells. Both backends return 3,354 nodes; 21/22 captured source bodies
are nonempty. All 3,354 selected identities verify and publish the same snapshot.
[Inspection 39458413589](https://github.com/DSLZL/Sub-Store/actions/runs/39458413589)
confirms one complete cold detector batch and zero measured warm probes.
One warm invocation records producer Node13/Rust82ms for 3,354 calls, MD5 20/6ms
for 3,356 calls and whole health operator 61/184ms. This is a new capture and
runner, so it does not establish a 137-to82ms production improvement.

Rust still has higher overall latency: cold 14.930s versus Node5.263s (one sample)
and warm median 2.733s versus 1.852s (three). Rust cold makes 4,424 probes for the
same 3,354 identities; Node makes 3,354. Cold CPU is 12.50/4.49s for Rust/Node.
The excess retry/transport cost remains open and is unchanged by this iteration.
These observations do not establish full performance acceptance.

[Native compatibility 39457094305](https://github.com/DSLZL/Sub-Store/actions/runs/39457094305)
passes all six jobs at `cda93a24`: Linux x64/arm64, Windows x64, Docker
x64/arm64 and the unchanged frontend. Formatting, lint, unit, probe and delivered
service checks are included; this is conversion compatibility acceptance.
[Complete matrix 39458493395](https://github.com/DSLZL/Sub-Store/actions/runs/39458493395)
completed with failures at the same source. Rust cold block 4 verifies/publishes
3,219 of 3,256 identities after 4,964 probes; warm block 0 starts from an incomplete
3,232-node warmup and fails Clash output verification. This is an output check,
not evidence of a failed HTTP request. All twelve pressure blocks pass, including
clients 4/8, but the whole matrix fails. The broader task remains in progress.
All product execution and artifacts stay in Actions.

An isolated native projection-replacer experiment at `a4d5d725` is not merged.
[Diagnostic 39465006279](https://github.com/DSLZL/Sub-Store/actions/runs/39465006279)
shows only 2.0-7.9% lower complete-call medians against `cda93a24`, while actual
cold/warm/no-publisher checks still fail incomplete health work. Five focused
value-bridge tests pass. This does not qualify the experiment for acceptance;
its larger callback implementation stays outside the reconstruction branch.

The next retained candidate, `464241c8` (production change `52c844ef`), reuses
successful platform certificate-verifier initialization. Previously every
script HTTP request constructed a verifier, loading/parsing the Linux system
roots, including plain HTTP probes. Request TLS configs/session state,
strict/insecure policy, proxies, pooling, cancellations and retry/timeout
budgets retain their existing owners. Initialization errors remain retryable.
Linux system roots now have process lifetime; restart after trust-store changes.

[Initial diagnostic 39467750872](https://github.com/DSLZL/Sub-Store/actions/runs/39467750872)
passes unchanged-file imports/restarts and all eight cells at `52c844ef`, with
3,261 output nodes on both backends. Cold latency is Node5.226/Rust6.664s and
CPU4.43/4.44s (one sample); warm median is1.922/2.683s (three). Warm helper totals
remain Node5/Rust74ms for3,261 producer calls and whole health63/172ms.
These are paired backend observations on a new capture/runner, not a controlled
14.930-to6.664s Rust before/after ratio. Rust remains slower overall here.
The initial verifier microprofile used a nonexistent module filter and ran zero
tests; it supplies no initialization measurement. The corrected harness requires
explicit nonzero semantic/profile test summaries and uses `script_io::transport`.

[Final diagnostic 39469080522](https://github.com/DSLZL/Sub-Store/actions/runs/39469080522)
and [complete matrix 39469086270](https://github.com/DSLZL/Sub-Store/actions/runs/39469086270)
both PASS at exact candidate `464241c8` and harness `8ab5262d`. That source differs from
`52c844ef` only in three unit-assertion formatting changes. The full matrix is
uninstrumented. [Native compatibility 39468483155](https://github.com/DSLZL/Sub-Store/actions/runs/39468483155)
PASSES all six jobs at that final source: native Linux x64/arm64, Windows x64,
Docker x64/arm64 and the unchanged frontend. No local product execution occurs.

The corrected diagnostic executes two transport semantic tests (one ignored
profile), then exactly one ignored release profile; the workflow requires both
nonzero success summaries. Initial shared-verifier setup takes 3.506ms. Three
blocks of 100 fresh constructions take 292.765/294.234/292.752ms; 100 shared-owner
lookups report 0.001ms in each block at the printed precision. This measures
initialization only, excluding connection/request/TLS handshake costs. Its eight
full-input cells also pass with 3,280 output nodes on both backends. Its different
capture, instrumentation and runner timings are not the final performance table.
The conversion implementation is identical to baseline `cda93a24` in this second
profile; those conversion rows cannot establish another source improvement.

The completed uninstrumented matrix contains 70 passing cells. All ten cold
pairs select, verify and publish every one of 2,998 eligible identities in one
batch, with exactly 2,998 probes per backend and no budget skips. All warm and
pressure blocks pass the zero-measured-probe, output/snapshot and Clash checks.
Parse-only output has 5,479 nodes on both backends before the normal filters.
Both raw replays match all 23 source strings (22 remote plus local content).
Capture attempts all 22 remote sources and receives 18 nonempty bodies; four
empty/failed bodies are recorded and replayed equally. This does not measure
unavailable provider content. Both backends pass five unchanged-file browser
imports and five restart checks.

These are pooled request medians and process observations from that single
4-vCPU Intel Xeon Platinum 8370C Linux x64 runner (15.61GiB RAM). CPU excludes
the browser, driver, fixture, sampler and real detector processes. Peak RSS is
sampled over each case; the separate OS lifetime peaks remain in the report.

| Case | Node median s | Rust median s | Node CPU s/request | Rust CPU s/request | Node peak RSS MiB | Rust peak RSS MiB |
|---|---:|---:|---:|---:|---:|---:|
| Cold, 10 requests/backend | 8.125 | 10.553 | 5.945 | 6.346 | 515.5 | 227.9 |
| Warm, 100 requests/backend | 2.732 | 3.996 | 3.136 | 4.001 | 1125.1 | 218.0 |
| Parse, 60 requests/backend | 2.500 | 2.844 | 2.920 | 2.891 | 407.0 | 71.2 |
| Pre-health, 20 requests/backend | 2.549 | 3.130 | 2.994 | 3.189 | 482.6 | 65.3 |
| No publisher, 60 requests/backend | 2.664 | 3.658 | 3.076 | 3.664 | 1081.4 | 218.9 |

| Pressure clients | Node requests/s | Rust requests/s | Paired Node/Rust throughput ratio |
|---|---:|---:|---:|
| 1 | 0.37 | 0.25 | 1.470 |
| 2 | 0.37 | 0.47 | 0.776 |
| 4 | 0.37 | 0.49 | 0.748 |
| 8 | 0.37 | 0.50 | 0.741 |

Thus Rust still has about 30% higher cold and 46% higher warm single-request
latency, while paired throughput is about 29/34/35% higher at clients 2/4/8.
The 8-client Node/Rust pooled median latencies are 19.205/15.933s. Rust uses
more CPU under pressure (2.79 versus 1.15 average cores across those windows),
so the throughput result is not a CPU-efficiency claim. Three pressure blocks
per setting and ten cold pairs are limited samples; retain paired intervals,
per-block observations and P95 sample qualifications from the Actions report.

Import API medians are Node5.73/Rust5.52ms; startup readiness medians are
259.20/51.25ms, and UI import wall medians are 20.22/19.19ms. Readiness includes
the driver's polling interval. The separate real HTTP-META/Mihomo pair passes
each backend's work/snapshot checks, but verifies different reachable sets
(Node883/Rust881 of2,998). Its 14.972/17.111s timings and8.05/7.09s CPU are
observations, not an equal-output speed comparison.

This validates the retained conversion and verifier changes and the complete
comparison for this capture. It does not establish Rust as faster overall.
The remaining warm gap persists even with zero probes; further attribution must
cover the full pipeline, state handling and script execution rather than assume
the remaining small per-node producer cost explains total request time. No new
before/after wall-time ratio is inferred across different captures or runners.

The isolated whole-request diagnostic
[39521899136](https://github.com/DSLZL/Sub-Store/actions/runs/39521899136)
passes all eight full-input cells. One warm request creates65 parser contexts,
taking600.7ms of3242.1ms; preprocessing takes1014.4ms and native parse1350.6ms.
These are overlapping inclusive spans, not an additive breakdown. Diagnostic
timers remain on their separate branch. The candidate reuses one independent
parser per job, while continuing conversion, validation and external CA reads.

`--diagnose-health --rust-baseline PATH` compares Node, the candidate, frozen
464241c8 and the candidate again on one runner and one captured input. Each cell
starts with an independent copy of the same imported state and keeps complete
output/snapshot/probe checks. The frozen binary builds in a separate target;
provenance requires different source revisions and executable hashes. This
short comparison does not replace the full matrix or native/frontend gates.
