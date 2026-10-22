# Supplied-subscription comparison

Run **Supplied subscription performance** (`rust-performance.yml`) manually on
`codex/rust-server-parity`. All builds, self-checks, browsers, services and
measurements run in GitHub Actions; the harness refuses local execution.

The current raw-input result is [39556665967](https://github.com/DSLZL/Sub-Store/actions/runs/39556665967):
all 35 paired jobs / 70 cells passed in 11m38s including preparation. See the
2026-10-21 parallel result below; older processed-input results remain historical.

Set `inspect_run` to an earlier run ID to print bounded, already sanitized
field/work diagnostics in Actions without building or downloading bundles locally.
Set `diagnose_health` for a separate short full-input diagnosis: pre-health,
one cold full scan, then three warm/no-publisher requests per backend. It retains
all sources and strict comparisons. Identical diagnostic wrappers count/time
MD5, producer and cache calls with millisecond clock resolution; their extra
write/measurement overhead means these are attribution observations, not the
unmodified full-run performance result. `health-profile.patch` records the
instrumentation separately from the unchanged five-site `stress.patch`.

Enable both `diagnose_health` and `profile_pipeline` for per-processor Rust
diagnostics. The ordinary candidate (`rust`) and its profiled repeat
(`rust_repeat`) use the **same binary**; the latter alone gets
`SUB_STORE_PROFILE=1`. Normal service runs emit no profile. Identical raw-source
replay, output/state/work checks, all processor work and existing test counts
remain required. The separate historical Rust baseline remains uninstrumented.
This mode runs the five-site stress script directly on all four variants, without
the old Date.now helper wrappers or their extra `benchmark-health-profile` state
write. Prepared/worker mode flags and script hashes must agree. Ordinary
`diagnose_health` without this flag retains its historical JS wrappers.

Each measured download emits one bounded numeric JSON event before replying.
`metrics.json` retains the complete tree, and the Actions summary shows every
processor's zero-based process/processor indices and the most expensive stages.
Counts, total/max wall nanoseconds, inclusive active nanoseconds and exclusive
self nanoseconds cover bootstrap/state, source input, parse, each processor,
script loading/compilation/binding/input cloning/call/await/fallback, final
production and output encoding. Producer stages split validation, filtering and
callbacks, preparation and callbacks, collection, node/options projection,
transport reconciliation, native output and JS decoding. Projection and decoding
are split again into their serialization, text extraction, parsing and restore
steps. Response transformers retain their original processor indices.

Wall time includes async waiting and overlapping siblings must not be added.
Active time measures elapsed time while a scope's Rust call/future poll runs;
it includes synchronous JS, blocking I/O and OS descheduling, so it is **not
CPU time**. Self time excludes nested active scopes. Async scopes re-enter only
while polled. JS jobs the engine drives outside a processor's future poll are
charged to the enclosing engine/job scope; the complete job and per-processor
wall totals remain available. Report writing and disabled/enabled timer overhead
are observable in the same-binary client-latency comparison, not subtracted from
measurements. These diagnostic samples are not uninstrumented throughput claims.

Only code-owned labels and numeric indices/timings leave the runner; no
subscription names, addresses, script bodies or arguments are used as labels.
The reader rejects missing/corrupt events, unknown fields, unbalanced times,
unfinished spans, or overflow beyond 2048 aggregate rows per job. Raw private
server logs remain in runner scratch. Profiling uses the standard library and
adds no tracing/profiling dependency.

### Verified processor profile (2026-10-21)

[Run 39622274257](https://github.com/DSLZL/Sub-Store/actions/runs/39622274257)
and its [reader 39623985503](https://github.com/DSLZL/Sub-Store/actions/runs/39623985503)
pass at product `887ddccb`, harness `c1b0e201`, previous product `ea16edca`,
original `9cc6b5fa` and unchanged frontend `4bd7b0ac`. All sixteen cells / forty
measured requests pass output, state and work checks. Ten profiled requests
produce complete, balanced events with no dropped rows or unfinished scopes.
All four variants return 3647 filtered nodes. The frozen capture has 21 nonempty
remote bodies and one explicitly retained empty/failed body; the 23 ordered raw
strings, including local content, replay identically before preprocessing.

The following are means over the three profiled warm requests. Processor indices
refer to the original imported configuration. Parent/child times overlap; each
processor's complete internal tree remains in `metrics.json`.

| Processor index | Type | Mean wall ms |
|---:|---|---:|
| 0 | Quick Setting Operator | 101.205 |
| 1 | Type Filter | 6.876 |
| 2 | Regex Filter | 78.835 |
| 3 | Script Operator | 129.618 |
| 4 | Script Operator (health) | 302.012 |
| 5 | Regex Delete Operator | 119.057 |
| 6 | Script Operator (publisher) | 354.447 |

Inside processor 4, each request makes 3647 actual producer calls. The producer's
mean inclusive active time is 132.052ms; these immediate child phases partition
that total, with approximately 8.85ms left for validation, collection and the
producer's own bookkeeping. The values include instrumentation overhead.

| Producer phase | Mean active ms |
|---|---:|
| Project live nodes into owned Rust values | 54.057 |
| Project options and coerce output kind | 11.648 |
| Decode output back into JS | 23.437 |
| Filter and prepare live objects, including callbacks | 22.767 |
| Reconcile shared WebSocket/gRPC options | 6.665 |
| Generate native target output | 4.625 |

Node/options projection and output decoding together account for approximately
89.14ms / **67.5%** of producer active time. Native target generation accounts for
**3.5%**. This points to the JS/owned-value boundary as the main producer target.
Within the complete health processor, input cloning contributes 29.377ms and
compilation 1.465ms. Other request costs remain visible: bootstrap averages
303.787ms, built-in application 227.032ms across four operators, serialization and
parsing of five input clones 180.141ms, and the parse entry 354.448ms across 44
invocations. These totals overlap
with processor and bridge phases and must not be added indiscriminately.
Engine work outside narrower polls remains in the enclosing job's self time.

| Case | Requests per variant | Node median s | Previous Rust s | Current, profile off s | Same binary, profile on s |
|---|---:|---:|---:|---:|---:|
| Pre-health | 3 | 4.190 | 1.326 | 1.398 | 1.403 |
| Cold | 1 | 8.780 | 8.431 | 8.262 | 8.978 |
| Warm | 3 | 4.447 | 2.253 | 2.273 | 2.282 |
| No publisher | 3 | 4.384 | 1.798 | 1.828 | 1.870 |

The observed enabled/disabled warm difference is 0.4%, and no-publisher is 2.3%.
Three sequential observations and a single cold request do not establish a
constant profiler overhead or a throughput/tail-latency result. The earlier
[39618663779](https://github.com/DSLZL/Sub-Store/actions/runs/39618663779) also passes
but includes the retained legacy JS timing wrappers and their extra state write.
Raw body hashes, node counts and runner conditions vary between runs; compare
variants within each frozen-input run and keep the earlier evidence separate.

[Native acceptance 39618445261](https://github.com/DSLZL/Sub-Store/actions/runs/39618445261)
passes all six jobs at exact product `887ddccb`: Linux x64/arm64, Windows x64,
Docker x64/arm64 and the unchanged frontend. Formatting, clippy, all 34 ordinary
Rust tests and the retained delivered-service/JS/state/restart/rollback contracts
pass. Performance self-checks pass all 24 tests in Actions. All execution and
report artifacts remain in Actions; the following report changes leave the tested
Rust source unchanged.

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
before application processes start. Plaintext private files stay in `$RUNNER_TEMP`
and are never uploaded, cached, or included in console logs. A one-day artifact
transfers an AES-256-GCM encrypted capsule of the captured original text and
UI-created seeds to workers. Its key derives from the private input bytes with
a domain prefix and random salt, not the published input hash; authentication
also binds it to the workflow run. The key is never uploaded.

The workflow freezes original `9cc6b5fa`, Rust `ea16edca`, and unchanged frontend
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
on loopback. An untimed original-backend preview with an inspection-only Script
Operator captures `context.raw` once, before downloader YAML preprocessing.
`produceType=raw` exposes already-preprocessed text and is unsuitable for this
boundary. The pinned input has no URL fragments or age secret arguments.
Empty/failed source bodies are counted explicitly; results
cannot establish performance for content the providers did not return.
Both backends must replay the exact same 23 ordered strings (22 remote sources
plus local content) before any timed cell starts. A mismatch fails preflight.
Only the fetched source text is reused: measured requests still enter the public
download route and execute source preprocessing, parsing, conversion, all configured processors,
scripts, normal cold/warm cache logic, snapshot publication and output production.
Parsed nodes and final output are not supplied as cached application results.
Reports through run `39526057421` used the earlier preprocessed-text capture.
Their results remain immutable and are not evidence for this stricter boundary;
new reports identify it with `capture.before_preprocessing=true`.
The passing raw-input diagnostic below validates this boundary. The complete
raw-input matrix is reported in the 2026-10-21 parallel result below.

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
Raw replay equality is a required input gate on both backends, in addition to
the measured output/work checks.
Parse and pre-health cases run first
to locate processing differences. A product mismatch marks that cell failed but
allows independent cells to run; observed metrics and differing counts remain in
the report, while failed/non-equivalent pairs never enter speedup estimates.
Timeout, crash or incomplete work is a failed
sample; the harness never trims nodes or re-enables maintenance caps to pass.
The full matrix now uses 35 independent jobs (up to 20 concurrently), each with
one Node/Rust pair on the same runner, preserving all 70 cells. Builds, five
actual imports per backend and original-text capture happen once. Each worker
validates identical binaries/inputs and all 23 ordered replay strings before
measurement. Different jobs can have different physical CPUs despite sharing
the `ubuntu-24.04` label; every runner's model, quotas and load are recorded.
Paired estimates compare servers on the same machine; pooled distributions span
these machines. Missing, duplicate, failed or unequal cells fail aggregation.
Failed-job reruns reuse successful pairs and select the latest attempt of each
rerun pair; original artifacts remain intact. Pair jobs have a 90-minute deadline.

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

`--diagnose-health --rust-baseline PATH` compares Node, the candidate, the pinned
Rust baseline and the candidate again on one runner and one captured input. Each cell
starts with an independent copy of the same imported state and keeps complete
output/snapshot/probe checks. The frozen binary builds in a separate target;
provenance requires different source revisions and executable hashes. This
short comparison does not replace the full matrix or native/frontend gates.

The first uninstrumented comparison,
[39522853939](https://github.com/DSLZL/Sub-Store/actions/runs/39522853939),
passes16/16 cells with3057 equal output nodes. Parser reuse e9da7c39 gives warm
medians3.2388/3.2602s versus frozen464241c8 at3.9682s, a reduction of18.4/17.8%.
Node is2.7716s. Pre-health medians are2.2502/2.2518s versus baseline3.0034s and
Node2.5905s. No-publisher medians are2.8108/2.8848s versus baseline3.6249s.
Warm/pre-health/no-publisher cells have only three requests each; cold has one
per cell, and its7.8383/9.6723s candidate timings are too variable for a stable
cold improvement claim. All16 harness checks and25 Rust tests execute and pass.

The retained diagnostic
[39523282917](https://github.com/DSLZL/Sub-Store/actions/runs/39523282917)
passes8/8 cells and confirms one parser context per request. One warm request's
three generic YAML normalizations take1564.8ms. Its state comparison/persistence
spans overlap: codec stringify324.0ms, delta185.7ms and persistence424.6ms.
The next candidate compares against e9da7c39: reuse only the last successful
generic normalization text within the request, clone only the requested state
key, and avoid serialization when ordered marker-free values are identical.
Final proxy conversion, validation, CA reads and writes remain active; no result
is reused across jobs. Equivalent-output latency results are still required.

The second same-runner comparison,
[39525101503](https://github.com/DSLZL/Sub-Store/actions/runs/39525101503),
passes16/16 cells with3102 equal nodes. Product4adef131 versus baselinee9da7c39:

| Case | Node s | Baseline Rust s | Candidate first/repeat s |
|---|---:|---:|---:|
| Pre-health | 1.8290 | 1.4801 | 0.8959 / 0.9059 |
| Warm | 1.9729 | 2.1077 | 1.4468 / 1.4244 |
| No publisher | 1.8851 | 1.8453 | 1.2378 / 1.2676 |
| Cold, one request/cell | 5.2789 | 8.3129 | 5.4101 / 8.8577 |

Warm latency is31.4/32.4% lower than the preceding Rust product and26.7/27.8%
lower than Node in this short capture; each warm cell has three requests. Cold
is still unstable. All16 harness checks and26 Rust semantic tests pass. Health
script instrumentation is identical on all backends; this is not the final
uninstrumented long-matrix result. Do not combine these ratios with the earlier
different capture/runner or claim a general cold speedup. Production has no
diagnostic timers. [Native compatibility39524979905](https://github.com/DSLZL/Sub-Store/actions/runs/39524979905)
passes all six jobs at exact4adef131. The completed uninstrumented full matrix
for that exact product is reported below.

The final isolated diagnostic
[39525638140](https://github.com/DSLZL/Sub-Store/actions/runs/39525638140)
passes8/8 cells. An actual warm request creates one parser context (12.98ms),
runs generic YAML normalization once (526.01ms), and reuses its normalized text
twice (0.010ms total). State delta takes1.29ms over four calls, while six root
reads still execute. These inclusive timings overlap and use a different
capture/runner from earlier diagnostics. They verify the retained paths; they
do not supply a cross-run total-speed ratio. Diagnostic source and timers stay
on their separate branch, and its clean local source-only worktree is removed.

The final uninstrumented matrix
[39526057421](https://github.com/DSLZL/Sub-Store/actions/runs/39526057421)
passes70/70 cells at product4adef131 and harness2b288ed2. Its
[remote report reader39530822738](https://github.com/DSLZL/Sub-Store/actions/runs/39530822738)
also passes. Original9cc6b5fa and frontend4bd7b0ac remain fixed. All five actual
unchanged-file browser imports and five restarts per backend pass. Capture gets
20/22 nonempty source bodies; two empty/failed sources are recorded and replayed
equally. Both backends replay all23 source strings, including local content.
Parse output is6266 on both; full filtered output is3255 on both. All20 cold
rows (ten pairs) select, verify and publish3255 identities, with exactly3255
probes per backend, one full batch and no skips. Warm and pressure retain
zero measured probes and their complete output/snapshot/Clash assertions.

These results use one4-vCPU AMD EPYC7763 Linux x64 hosted runner with15.61GiB
RAM and no reported cgroup CPU quota. The table gives pooled request medians,
backend CPU per measured request and sampled backend peak RSS; auxiliary
driver/browser/fixture/detector processes are excluded from backend resources.

| Case | Requests/backend | Node median s | Rust median s | Node CPU s/request | Rust CPU s/request | Node peak RSS MiB | Rust peak RSS MiB |
|---|---:|---:|---:|---:|---:|---:|---:|
| Cold | 10 | 10.747 | 10.018 | 7.437 | 5.602 | 520.2 | 249.5 |
| Warm | 100 | 3.276 | 2.548 | 3.852 | 2.554 | 1230.5 | 231.6 |
| Parse | 60 | 3.063 | 1.293 | 3.653 | 1.357 | 455.8 | 80.6 |
| Pre-health | 20 | 3.128 | 1.656 | 3.743 | 1.729 | 440.1 | 73.2 |
| No publisher | 60 | 3.263 | 2.258 | 3.851 | 2.258 | 1213.6 | 223.5 |

Warm single-request latency is22.2% lower, with33.7% lower backend CPU/request
and81.2% lower sampled peak RSS in this case. Warm P95 is Node3.344/Rust2.578s
over100 requests each. Parse latency is57.8% lower and pre-health47.0% lower.
Cold is still inconclusive: although the pooled median is lower, mean latency
is Node10.509/Rust10.884s. The ten paired observations give a Node/Rust ratio
0.972 with bootstrap interval[0.900,1.039], crossing1. Samples range Node7.65-
15.35s and Rust7.94-15.51s. Do not claim stable cold superiority or a cold P95
from ten requests. The short A/B ratios and older complete matrix use different
captures/runners; they cannot be combined into a new before/after speed ratio.

Reading all20 original cold resource records inside Actions
([39531336358](https://github.com/DSLZL/Sub-Store/actions/runs/39531336358))
shows stable backend CPU: Node7.21-7.64s and Rust5.57-5.65s. Fixture CPU ranges
from0.83-8.49s for Node cells and0.75-8.30s for Rust cells, rising with the slow
wall-time samples; driver CPU stays0.11-0.16s. Thus the public detector fixture
is a substantial source of cold variability. It creates3255 ephemeral listening
sockets per batch, then serves3255 probes. Listener allocation, port lifetime
and probe handling are not separately timed, so the exact fixture cause remains
unconfirmed. These existing resource records do not justify subtracting CPU from
wall time or publishing an adjusted cold speedup. The read-only inspection and
its instrumentation remain off the production branch.

| Pressure clients | Node requests/s | Rust requests/s | Paired Node/Rust throughput ratio |
|---|---:|---:|---:|
| 1 | 0.302 | 0.393 | 0.768 |
| 2 | 0.309 | 0.721 | 0.428 |
| 4 | 0.299 | 0.787 | 0.380 |
| 8 | 0.304 | 0.759 | 0.401 |

Each setting has three timed blocks. The8-client pooled medians are Node23.800/
Rust10.180s. Rust uses about3.31 average cores versus Node1.18 in those windows
and4.36 versus3.88 backend CPU seconds/request, so this pressure result does not
imply lower CPU per request at every concurrency. The single-request warm result
above is independently measured.

Import API medians are Node4.35/Rust4.49ms; startup readiness260.56/51.45ms and
UI wall63.77/19.23ms, including polling/browser variability. The separate real
HTTP-META/Mihomo pair returns1305/1094 reachable nodes of3255, at16.544/16.392s
and10.16/6.93 backend CPU seconds. Its unequal reachable sets and single pair
cannot establish equal-work performance or explain the reachability difference.
Keep cold variability and real-network limits open; the measured warm/parse
single-request gap is resolved for this supplied-input capture. All builds,
tests, browser flows, measurements and artifact inspection execute in Actions.

## Untouched-source replay acceptance (2026-10-21)

Actions39540875021 PASSES at harness70f0eb9d and productd32f652a; reader
39541591690 confirms both backends replay23/23 ordered strings with zero timeout
log entries. The20/22 nonempty remote inputs and two recorded empty/failed
bodies are captured once before preprocessing. All44 direct-source rows and
all4 complete-input parse/pre-health diagnostic cells pass exact comparisons.
Each parse request returns6429 nodes and pre-health3478 on this capture.

Moving owned HTTP I/O to the parent service runtime allows collection to
continue while synchronous source preprocessing runs on the script worker.
Only owned strings/results cross runtimes; the JoinSet aborts its owned task
when dropped. This fixes the observed first-request source loss without changing
8-second source timers, retries, parsers, caches or measured processing. The
internal ordinary-text bridge also avoids per-byte JSON arrays and follows
Undici BOM/UTF8 behavior; binary responses retain their exact bytes.

This diagnostic exposes the cost hidden by historical preprocessed capture:
three measured parse requests take17.48s Node versus58.33s Rust in total;
pre-health17.70s versus59.60s. These are cell totals, not per-request medians.
Several raw YAML direct-source rows are CPU-bound on Rust (source2:0.487s/0.860
CPU Node versus2.938s/2.920 CPU Rust; source3:0.417s versus2.924s). Parser::yaml
still executes the pinned generic YAML JavaScript library inside QuickJS. This
locates further work; no new whole-product speedup or full-matrix pass is claimed.

Product7022849b changes only formatting requested by the retained native CI
rustfmt diff. No YAML dependency/implementation or test-workload shortcut is
included. Earlier reports remain immutable under their narrower input boundary.

## Per-node preparation and projection (2026-10-21)

Product `ea16edca` lets the script producer dispatch its already prepared owned
values without a second pass through common filtering and normalization. The
duplicate owned preparation has no remaining production callers and is removed;
the live script producer retains target validation and preparation. Projection
returns its existing JSON string directly when there are no exceptional paths,
avoiding a result tuple and serialization/parsing of an empty path list. Cycles,
special numbers and unsupported root values keep their previous representation.
Focused regressions cover ordinary repeated getter/toJSON reads, sibling aliases,
mixed object/array cycles, special numbers, private/UTF-16 keys and recovery after
an exception.

The health diagnostic now compares against the immediately preceding production
version `9c659cef`, using the existing same-input Node/candidate/baseline/repeated
candidate requests. Its preparation job also runs the unchanged public 3,096-call
component profiles in candidate/baseline/candidate order on one runner. Source
pins and build targets remain separate. Source capture, real file import,
correctness checks, request counts and cold/warm work checks retain their prior
behavior; all product execution remains in Actions.

Performance and native acceptance: **PASSED**. The first candidate's native run
[39605952808](https://github.com/DSLZL/Sub-Store/actions/runs/39605952808)
found a newly unused preparation entry and a formatting diff. It and the
superseded diagnostic [39606000665](https://github.com/DSLZL/Sub-Store/actions/runs/39606000665)
were stopped before accepting a full result. Its completed public component
profiles show the active-stack experiment made projection roughly 3-7% slower
than the immediately preceding baseline on the same runner, so that experiment
is removed. Runs 39606917279, 39606961853 and 39606968623 were superseded before
acceptance to avoid continuing that version. The replacement removes the unused
implementation, applies the CI-generated formatting and omits empty projection
metadata. Historical results below retain their stated earlier source versions.

The completed preparation job in [diagnostic39608454627](https://github.com/DSLZL/Sub-Store/actions/runs/39608454627)
passes all 31 release regressions and all public component assertions. It executes
candidate/baseline/candidate again on one runner, in separate Cargo targets. Each
cell below is the median of three blocks of 3,096 actual complete script-producer
calls; setup and output assertions are outside the timed loops.

| Public fixture | Baseline `9c659cef` ms | Candidate first ms | Candidate again ms | Time reduction |
|---|---:|---:|---:|---:|
| Flat SS | 52.834 | 48.430 | 48.320 | 8.3-8.5% |
| WebSocket | 85.789 | 80.894 | 81.231 | 5.3-5.7% |
| Nested metadata | 130.362 | 124.268 | 124.515 | 4.5-4.7% |

Projection alone is also lower in both candidate measurements: 15.957/15.818ms
versus 17.886ms for flat SS, 35.337/35.242 versus 36.224ms for WebSocket, and
67.720/66.935 versus 69.592ms for nested metadata. These are Rust before/after
component measurements, separate from Node comparisons and complete-request
acceptance.

The supplied-input diagnostic passes all sixteen cells with 3,938 equivalent
output nodes per variant. Warm complete-request medians over three requests are
Node 5.791s, candidate 3.223s, baseline 3.291s and candidate again 3.232s. This
observes a 1.8-2.1% reduction from the previous Rust version in this small sample.
Cold observations span 10.717-11.234s across the Rust variants and do not establish
a cold-request improvement. The health script's millisecond-resolution warm
produce counters are Node 16ms, candidate 161ms, baseline 166ms and candidate
again 175ms for 3,938 calls. These rounded counters do not establish a stable
improvement for this real helper or parity with Node; the public component
profiles above provide the controlled before/after measurements.

[Full matrix39608465038](https://github.com/DSLZL/Sub-Store/actions/runs/39608465038)
passes all seventy cells in thirty-five independent paired worker jobs.
[Full report39610594741](https://github.com/DSLZL/Sub-Store/actions/runs/39610594741)
and [diagnostic report39610701917](https://github.com/DSLZL/Sub-Store/actions/runs/39610701917)
also pass. The capture contains 20/22 nonempty sources and two explicit
empty/failed sources, before preprocessing; both backends replay all 23 ordered
raw strings exactly. Parse output is 6,623 nodes and complete controlled output
is 3,688 on both backends. All cold work audits, warm no-new-probe checks,
snapshots and ClashMeta output comparisons pass.

| Complete controlled requests | Node median s | Rust median s | Requests per backend |
|---|---:|---:|---:|
| Parse | 3.634 | 0.850 | 60 |
| Pre-health | 3.525 | 1.029 | 20 |
| Cold | 9.838 | 9.855 | 10 |
| Warm, all processors | 5.970 | 2.956 | 100 |
| Without publisher | 3.683 | 1.443 | 60 |

Warm CPU is Node/Rust 7.532/2.722s per request and sampled peak RSS is
1,373.7/365.4 MiB. Rust pressure throughput is 0.371/0.790/0.765/0.599 requests
per second at 1/2/4/8 clients; eight clients still have lower throughput than
two or four clients on the four-CPU workers. These
new complete-run numbers compare the two engines within this capture, not the
previous report's different subscription bodies. The separate real-network
detector returns Node/Rust 1,061/1,396 nodes, so it does not establish an
equivalent-work speed ratio.

[Native acceptance39608386858](https://github.com/DSLZL/Sub-Store/actions/runs/39608386858)
passes all six jobs: Linux x64/arm64, Windows x64, Docker x64/arm64 and the
unchanged frontend. Its harness checkout `3b1b2d54` has the identical Rust source
tree to the measured product `ea16edca`. Formatting, lint, unit regressions,
delivered-service contracts and general-JS probes pass on all native targets.
All compilation, tests, browsers and containers execute in Actions.

## Parallel raw-input result (2026-10-21)

[Full run39556665967](https://github.com/DSLZL/Sub-Store/actions/runs/39556665967)
at harness `1bc1a72a` and product `9c659cef` passes all 70 cells in 35 paired worker jobs,
with at most 20 workers concurrently. Including builds, five actual unchanged-file
frontend imports/restarts per backend and once-only raw capture, workflow wall
time is 11m38s. [Report reader39558045248](https://github.com/DSLZL/Sub-Store/actions/runs/39558045248)
passes. Capture records 20/22 nonempty remote bodies and two empty/failed bodies;
both backends replay all 23 ordered strings exactly on each worker. These results
cover returned content, not unavailable provider content. Parse output is 6368
nodes and complete filtered output 3556 on both backends. All ten cold pairs
select, probe, verify and publish all 3556 identities in one batch, without
budget skips. Measured warm/pressure requests perform no new probes. All
controlled output, snapshot and ClashMeta comparisons pass. The separate real
detector pair is qualified below.

Across five fresh states per backend, startup readiness medians are Node
258.91ms / Rust 51.35ms, actual file-import API medians 5.65/3.26ms, and browser
UI wall medians 19.12/17.66ms. All five restarts per backend pass. These small
startup/UI samples include polling/browser variability and do not establish
tail latency.

The table contains pooled request medians, process user+system CPU per measured
request and sampled backend peak RSS. Each Node/Rust pair runs sequentially on
one Linux x64 worker, alternating order by block; CPU models, quotas and load
are recorded per worker. Same runner labels do not guarantee identical hardware.
Pooled distributions span workers; same-worker block estimates are separate.

| Case | Requests/backend | Node median s | Rust median s | Node CPU s/request | Rust CPU s/request | Node peak RSS MiB | Rust peak RSS MiB |
|---|---:|---:|---:|---:|---:|---:|---:|
| Parse/produce | 60 | 5.266 | 1.337 | 6.590 | 1.270 | 671.8 | 134.2 |
| Four processors before health | 20 | 5.088 | 1.701 | 6.920 | 1.813 | 645.3 | 134.4 |
| Cold full scan | 10 | 9.969 | 8.905 | 10.206 | 5.723 | 625.7 | 315.8 |
| Warm, all seven processors | 100 | 5.597 | 2.736 | 6.764 | 2.292 | 1368.8 | 356.9 |
| Six stages, no publisher | 60 | 5.634 | 2.372 | 7.602 | 2.376 | 1327.6 | 250.9 |

Pooled parse latency is 74.6% lower and warm latency 51.1% lower in this capture.
Warm P95 is Node 5.859 / Rust 2.811s over 100 requests each. Geometric means of paired
block Node/Rust latency ratios are parse 4.103 (bootstrap interval 3.926-4.466,
three pairs), warm 2.178 (2.072-2.303, five pairs), and cold 1.100 (1.034-1.158,
ten pairs). Three-block intervals have weak confidence; pre-health has just one
block. Ten cold requests do not establish tail latency or general cold capacity.

| Pressure clients | Node requests/s | Rust requests/s | Node median s | Rust median s |
|---|---:|---:|---:|---:|
| 1 | 0.198 | 0.414 | 5.150 | 2.479 |
| 2 | 0.177 | 0.643 | 11.229 | 3.097 |
| 4 | 0.242 | 1.049 | 13.907 | 3.228 |
| 8 | 0.177 | 0.661 | 35.353 | 12.065 |

All settings have three paired windows and complete equivalent work. On these
four-CPU workers Rust throughput falls from 4 to 8 clients, CPU/request rises
from 3.048 to 5.135s and sampled peak RSS from 976.5 to 1368.5MiB. This identifies
contention under pressure; the records do not isolate a specific responsible
function. Higher concurrency is not automatically higher capacity.

The raw-source bottleneck was the generic YAML JavaScript library interpreted
inside QuickJS. Product `9c659cef` uses pinned serde-saphyr 2.0.0 for ordinary
subscription YAML while retaining the existing resolver for aliases, tags and
schema-sensitive cases. An initial guard wrongly treated identifier prefixes
as numbers, forcing entire documents back onto the slow path; full-token guards
fix that. Native-path selection and rendered-value equality are independently
checked against the locked original JS utility. No final proxies/output cache,
source deadline, retry, probe cap or script change supplies the improvement.

[Same-input diagnosis39556670388](https://github.com/DSLZL/Sub-Store/actions/runs/39556670388)
passes 16/16 cells on one 4-vCPU AMD EPYC 7763 worker; its
[reader39559054309](https://github.com/DSLZL/Sub-Store/actions/runs/39559054309)
passes. Node, candidate `9c659cef`, pre-native baseline `7022849b` and candidate again
receive identical captured text and 3376 filtered nodes. Warm medians are Node
5.483s, baseline 19.793s, candidate 2.780s and repeat 2.781s: an 86.0% reduction
against the old Rust baseline in this short sample. Pre-health is baseline
19.215s versus candidate 1.685/1.701s. Each of these cells has three requests;
cold has only one. The same coarse diagnostic wrapper runs on all four variants,
so use the uninstrumented full matrix above for current performance claims.
This diagnostic capture has 18/22 nonempty remote bodies and four recorded
empty/failed bodies; do not combine its timings with the full run's capture.

One measured warm health invocation still spends 187ms in 3376 Rust producer
calls versus 21ms Node; total health-operator time is 360/105ms Rust/Node.
Rust MD5 is 8ms versus 48ms Node, and warm cache get/set totals are 0/0ms on both
at millisecond clock resolution. Thus per-node production remains a measured
local disadvantage although the complete Rust request is now faster. Cold
health operator time is 4579/3734ms and cache publication 158/46ms Rust/Node in
this one instrumented invocation. These overlapping wrapper totals and the
shared detector fixture do not establish a new whole-request optimization ratio.

The full run's separate real HTTP-META/Mihomo pair completes independently:
Node 1042 reachable outputs at 17.953s, Rust 1349 at 17.122s, with each backend's
full-probe/work checks passing. Different real reachability sets are not equal
work and must not enter speed ratios. No architecture ranking, tail-capacity
claim or estimate for unavailable provider inputs follows from these runs.
All execution and report inspection remain in Actions; no local product build,
test, dependency generation or evidence download was performed.

[Native acceptance39556563589](https://github.com/DSLZL/Sub-Store/actions/runs/39556563589)
at exact product `9c659cef` passes all six jobs: Linux x64/arm64, Windows x64,
Docker x64/arm64 and the unchanged official frontend. Formatting, clippy with
warnings denied, all 28 ordinary Rust tests, delivered-service/restart/rollback
contracts, general-JS oracle checks and browser/container acceptance pass.
The following report-only commit does not alter that tested production source.

### Value-boundary optimization experiments (2026-10-22)

The starting product for these experiments is `887ddccb`. The official frontend,
supplied file import, pre-preprocessing source capture, full script work and
strict output/state comparisons remain unchanged. Timings from different captures
must not be combined: each new run freezes its own raw bodies for all variants.

The following changes were measured and then removed:

- Lazy linked projection paths (`da7b705c`): the same-run component observations in
  [39630568439](https://github.com/DSLZL/Sub-Store/actions/runs/39630568439)
  make ordinary-node projection 5.5–7.0% slower and WebSocket projection about
  2.8% slower, while metadata-rich projection improves only 1.1–1.5%. The private
  comparison and remaining native jobs were cancelled after rejecting this path.
  These component observations remain valid; the cancelled run is not full
  acceptance evidence.
- Direct construction of whole ordinary JS trees (`dd87e307`):
  [39632094342](https://github.com/DSLZL/Sub-Store/actions/runs/39632094342) and
  [reader 39634071073](https://github.com/DSLZL/Sub-Store/actions/runs/39634071073)
  pass all sixteen cells / forty requests with 3768 equal outputs. Same-run
  component decoding improves 28–32%, but warm request median improves only from
  2.969s to 2.917s, while observed peak RSS grows from 260.8 to 348.1 MiB.
  No-publisher peak RSS grows from 204.2 to 278.3 MiB. This tradeoff was rejected;
  these observations do not prove a particular engine allocation cause.
  [39628419401](https://github.com/DSLZL/Sub-Store/actions/runs/39628419401)
  passes all six native/frontend/container jobs for that experimental source.
- Scalar-only direct decoding (`74e4f7e1`):
  [39635228659](https://github.com/DSLZL/Sub-Store/actions/runs/39635228659) and
  [reader 39639245816](https://github.com/DSLZL/Sub-Store/actions/runs/39639245816)
  pass sixteen cells / forty requests with 3754 equal outputs. Warm median is
  2.762s versus the same-run baseline's 2.750s, with peak RSS 297.1 versus
  299.7 MiB. No-publisher median is essentially equal, while peak RSS rises from
  224.8 to 237.9 MiB. This does not establish an end-to-end benefit, so the
  scalar fast path was removed. The added scalar/prototype semantic regression
  checks remain. [39634884099](https://github.com/DSLZL/Sub-Store/actions/runs/39634884099)
  passes all six native/frontend/container jobs for that experimental source.
- Compiled RegExp templates (`37c6d9c4`, together with scalar-only decoding):
  [39639235850](https://github.com/DSLZL/Sub-Store/actions/runs/39639235850) and
  [reader 39641291499](https://github.com/DSLZL/Sub-Store/actions/runs/39641291499)
  pass sixteen cells / forty requests with 3790 equal outputs. Warm median rises
  from 1.583 to 1.662s, CPU/request from 1.557 to 1.627s, and peak RSS from
  274.1 to 307.0 MiB. Both the default-off and profiled candidates are slower
  than baseline. Fresh-instance/constructor/prototype regression checks pass,
  but the additional guarded cache was removed after this measured regression.
  The remaining native jobs for the rejected source were cancelled; this is
  not full native acceptance or an isolated attribution to regex compilation.

An earlier `dd87e307` comparison,
[39628489744](https://github.com/DSLZL/Sub-Store/actions/runs/39628489744),
was rejected before measurement because the worker checkout revisions did not
match the prepared builds. The workflow now shares candidate/baseline revision
variables between preparation, workers and cache keys. The comparison regression
check verifies both checkout pairs against the harness pins. This keeps source
verification strict and prevents treating a mismatched binary as a timing sample.

### Embedded-library bytecode reuse (2026-10-22)

Product `ee64ab9f7376a68c126950caee256bf3fb9c2b12` retains process-local bytecode
reuse for fixed embedded libraries. Each request still creates an independent
runtime/context and executes every initializer; mutable JS values and host state
are not shared. Only privately compiled bytes enter the cache, without disk or
external bytecode loading. Loading copies the bytes into the current runtime.
Global strict mode, the original filename, debug stacks and function source are
preserved. The rejected projection/direct-decoding/RegExp changes are absent.

The following observations use the same twenty-three ordered raw source strings
(twenty-two remote sources: nineteen nonempty and three empty/failed, plus the
local source). The file is imported unchanged through the official frontend;
normal preprocessing, scripts, node conversion, cache behavior and publication
still execute. [Attempt 1](https://github.com/DSLZL/Sub-Store/actions/runs/39641575510/attempts/1)
and [attempt 2](https://github.com/DSLZL/Sub-Store/actions/runs/39641575510/attempts/2)
each pass all sixteen cells / forty measured requests with 3520 equal outputs.
Attempt 2 reruns only the measurement job and its aggregate, reusing attempt 1's
sealed input/build artifacts and frontend-created seeds. It does not recapture
subscription bodies. Each attempt pairs the original, baseline Rust, default-off
candidate and the same profiled candidate on one runner.

| Attempt | Case | Node median s | Baseline Rust median s | Candidate median s | Rust reduction |
|---|---|---:|---:|---:|---:|
| 1 | pre-health | 5.530 | 1.771 | 1.689 | 4.6% |
| 2 | pre-health | 5.235 | 1.732 | 1.649 | 4.8% |
| 1 | cold | 10.850 | 9.655 | 9.517 | 1.4% |
| 2 | cold | 10.646 | 9.624 | 9.495 | 1.3% |
| 1 | warm | 5.804 | 3.160 | 2.682 | 15.1% |
| 2 | warm | 5.649 | 2.641 | 2.605 | 1.4% |
| 1 | no publisher | 5.511 | 2.333 | 2.270 | 2.7% |
| 2 | no publisher | 5.324 | 2.302 | 2.220 | 3.6% |

Cold has only one measured request per variant/attempt; other cells have three.
The first warm 15.1% reduction does not reproduce as a stable effect. Warm
CPU/request similarly changes from 3.167 to 2.683s in attempt 1, but only from
2.640 to 2.607s in attempt 2. These observations support a modest initialization
benefit; they do not establish P95, long-run throughput or a general 15% speedup.

| Warm observation | Baseline peak RSS MiB | Candidate peak RSS MiB | Baseline end/settled MiB | Candidate end/settled MiB |
|---|---:|---:|---:|---:|
| Attempt 1 | 256.3 | 347.4 | 256.3 | 307.0 |
| Attempt 2 | 296.4 | 265.8 | 296.4 | 265.8 |

The profiled candidate's warm peaks are 287.4 and 238.8 MiB respectively. The RSS
direction reverses across the two attempts, so neither a stable memory reduction
nor a stable memory regression/leak is established. The 250ms settled sample is
not an indefinitely idle/leak check; post-response work retains its normal
lifetime. Preserve both attempts instead of selecting the lower peak. Bounded
readers [39644881058](https://github.com/DSLZL/Sub-Store/actions/runs/39644881058)
and [39645417695](https://github.com/DSLZL/Sub-Store/actions/runs/39645417695) expose
the immutable attempts' CPU, peak/end/settled RSS and numeric pipeline profiles.
The reader accepts `inspect_attempt` (default `1`), alongside `inspect_run`.

Attempt 1's three profiled warm requests locate the remaining work. Medians of
same-stage cumulative active time are: all twelve bytecode loads 6.36ms,
evaluations 7.31ms, and zero repeated compilations; total application bootstrap
232.90ms; five input clones 232.68ms. The 3520 per-node native conversions take
8.34ms, versus 79.14ms for node projection, 16.99ms for option projection and
33.62ms for result decoding. The full `produce` stage also includes a batch
output call and totals 245.09ms. Global projection stringify/parse totals are
174.29/39.64ms. Parent/child stages overlap; do not add these into a CPU total or
treat unclassified bootstrap time as compilation. Reducing full-list copies and
live JS/native value round trips is the next measured direction.

[39641498364](https://github.com/DSLZL/Sub-Store/actions/runs/39641498364) passes
all six native Windows/Linux x64/Linux arm64, Docker x64/arm64 and unchanged
frontend jobs for the exact retained product. Formatting, clippy with warnings
denied, all 38 ordinary Rust tests, delivered-service/state/restart/rollback and
general-JS contracts pass. Three component/CPU probes remain opt-in ignored tests;
the paired workflow explicitly runs its release component probe. All compilation,
product execution and testing occurred in Actions. Later report/reader changes
do not alter that tested Rust source.

### Bulk JSON state binding (2026-10-22)

Candidate `bfae5be4ff67a7bfc9be70ea838e28dce5c0bf02` builds on retained bytecode
baseline `ee64ab9f7376a68c126950caee256bf3fb9c2b12`. The shared wire serializer
copies whole quoted JSON strings instead of appending every character, avoids
three marker-replacement copies when markers are absent, and returns its owned
output directly when UTF-16 restoration is unnecessary. Numeric normalization,
marker escaping, lone surrogates, property order and compact/pretty bytes retain
their old behavior. The scanner is private and accepts only serde-generated
JSON. A regression corpus compares both wire formats against the old serializer;
the existing exhaustive UTF-16, numeric and state-order tests remain unchanged.

Application initialization also serializes its already-owned baseline through
an immutable borrow, eliminating the extra full `Value` clone. The borrow ends
before JS evaluation and both strings still parse into independent context-local
objects. Processor clones, live getters/toJSON, script execution, publication,
state refresh/merge and post-response work retain their existing behavior.
`bootstrap_state_encode` and `bootstrap_state_parse` expose numeric phase timing.
No dependency, node-result cache or runtime/context pool was added.

[Attempt 1](https://github.com/DSLZL/Sub-Store/actions/runs/39649627281/attempts/1)
and [attempt 2](https://github.com/DSLZL/Sub-Store/actions/runs/39649627281/attempts/2)
reuse the same twenty-three ordered raw source strings, frozen before normal
preprocessing. The original JSON is imported unchanged through the official
frontend. Both backends' raw-source replay hashes match. Each attempt passes all
sixteen cells / forty measured requests with 3611 equal outputs, including the
same candidate with profiling enabled. Attempt 2 reruns only measurement and
aggregation; the sole preparation, encrypted-input and build artifacts remain
attempt 1's. No subscriptions are recaptured or binaries rebuilt for attempt 2.

| Attempt | Case | Node median s | Previous Rust median s | Candidate median s | Rust reduction |
|---|---|---:|---:|---:|---:|
| 1 | pre-health | 4.291 | 1.297 | 1.228 | 5.3% |
| 2 | pre-health | 5.498 | 1.694 | 1.597 | 5.7% |
| 1 | cold | 8.696 | 9.354 | 7.817 | 16.4% |
| 2 | cold | 10.665 | 10.050 | 8.804 | 12.4% |
| 1 | warm | 4.461 | 2.045 | 1.924 | 5.9% |
| 2 | warm | 5.710 | 2.752 | 2.536 | 7.9% |
| 1 | no publisher | 4.291 | 1.722 | 1.643 | 4.6% |
| 2 | no publisher | 5.482 | 2.316 | 2.263 | 2.3% |

Cold has one measured request per variant/attempt; the other cells have three.
Warm OS CPU/request changes from 2.017 to 1.900s and from 2.760 to 2.537s.
The paired warm latency/CPU improvement reproduces in these two short diagnostic
attempts. Absolute times differ across runners, so compare variants within each
attempt; do not compare them directly with the preceding 3520-node experiment.
These are not the full pressure matrix, P95 or long-run memory acceptance.

| Warm observation | Previous peak RSS MiB | Candidate peak MiB | Previous end/settled MiB | Candidate end/settled MiB |
|---|---:|---:|---:|---:|
| Attempt 1 | 301.6 | 299.1 | 278.3 | 299.1 |
| Attempt 2 | 282.9 | 290.8 | 279.7 | 290.8 |

End/250ms-settled RSS is higher in both candidate observations, by about 21 and
11 MiB. No-publisher peaks are 242.5 -> 266.9 and 261.7 -> 262.2 MiB. Retain these
costs alongside the CPU/latency gain; no memory improvement is established and
these short observations do not identify an allocation cause or prove a leak.
The bounded readers [39651793207](https://github.com/DSLZL/Sub-Store/actions/runs/39651793207)
and [39652964075](https://github.com/DSLZL/Sub-Store/actions/runs/39652964075) expose
both immutable attempts' numeric metrics and profiler trees without private
input/state/log downloads to the workstation.

The profiled candidate's three warm requests have cumulative-stage medians of
132.52/167.94ms for bootstrap, including 23.63/29.77ms of state encoding and
17.46/21.45ms of state parsing (attempts 1/2). Input clones remain 182.47/239.81ms
and produce remains 176.34/259.08ms. Stages overlap; these are active elapsed
timings, not independent OS CPU measurements. The previous product is measured
with profiling off, so these phase numbers alone are not a before/after claim.

[39647919148](https://github.com/DSLZL/Sub-Store/actions/runs/39647919148) passes
all six Windows/Linux x64/Linux arm64, Docker x64/arm64 and unchanged-frontend
jobs. Its Rust source is identical to the pinned candidate. Formatting, clippy
with warnings denied, all 39 ordinary Rust tests, delivered HTTP/state/restart/
rollback, generic JS and original backend contracts pass. The three component
probes remain opt-in ignored tests. Benchmark self-checks pass all 25 tests.
All builds and product tests execute in Actions.

Pre-measurement failures remain visible: [39647712535](https://github.com/DSLZL/Sub-Store/actions/runs/39647712535)
has the initial formatting diff; [39647927619](https://github.com/DSLZL/Sub-Store/actions/runs/39647927619)
fails at the fixed-script GitHub API fetch before file import; and
[39649552071](https://github.com/DSLZL/Sub-Store/actions/runs/39649552071) exposes
GitHub blob base64 line wrapping in the self-check. None provides request
performance evidence. Setup now authenticates that fixed-host fetch with the
Actions token, retries only setup I/O failures, retains base64 line wrapping and
still verifies the pinned SHA-256. The token is excluded from child environments.
The reader can inspect sanitized preparation failure codes/stack line numbers
when no aggregate exists. Raw failure messages and private data remain on runners.

### Single-node parsing versus request overhead (2026-10-22)

Two Actions comparisons pass: [39701402820](https://github.com/DSLZL/Sub-Store/actions/runs/39701402820)
at `30219770797934df8622de2eceb7071d43e71618`, and
[39702044025](https://github.com/DSLZL/Sub-Store/actions/runs/39702044025)
at `cbf50eb5e56b1c42a2efdf43ed04d56a11253cb3`. Their production Rust parser and
service code remain `bfae5be4ff67a7bfc9be70ea838e28dce5c0bf02`; the only Rust
source addition is a `cfg(test)` entry for the standalone measurement driver.
The original remains `9cc6b5fac966993364ae33da8797f446bbf7f571`. Each comparison
runs both engines on the same Ubuntu 24.04 x64 runner with Node 24.16.0 and Rust
1.98.0 release builds. Absolute differences between the two runners are not an
optimization gain.

`rust-single-node.yml` runs this focused comparison. `single-node.json` contains
nine public, synthetic one-node fixtures, including Unicode names. The Node
worker imports the original build's public `ProxyUtils.parse` export from
`dist/proxy-utils.esm.mjs`; its console output and installed log-persistence hook
are disabled only for pure parsing. Rust calls the real `conversion::parse`.
Each invocation receives one node, rather than dividing a subscription's bulk
time by its node count. Both workers alternate the original text and an
equivalent trailing-newline variant, so the last-input YAML preprocessing cache
cannot bypass work. Parsed values and counts are checked against the original
outside the timed loop.

Pure measurements use persistent workers, warm-up, adaptive block sizes targeting
250 ms, seven blocks and alternating variant order. Values below are medians of
the seven block means. The reused Rust parser retains its context while parsing
each input again; the fresh-parser column constructs and drops a parser on every
call. Fresh-parser measurements retain the process's existing library-bytecode
cache, so they are not cold process startup measurements.

The real endpoint is `POST /backend/api/proxy/parse` with `client=JSON` and
`type=internal`. It includes parsing, production, application setup, request
encoding and localhost HTTP transport. Both services start with empty application
state and default logging, warm up for 20 requests per format, and execute seven
blocks of 30 individual requests per format/backend. Each full response is
checked against the original; all 7,560 timed HTTP requests across the two runs
match. These empty-state single-node requests do not represent the imported
subscription's full-state workload or its throughput under concurrency. The
timed endpoints keep profiling disabled.

Latest run, in microseconds per pure parse and milliseconds per HTTP request:

| Input | Node pure us | Rust reused parser us | Rust new parser us | Node HTTP median ms | Rust HTTP median ms |
|---|---:|---:|---:|---:|---:|
| SS URI | 7.982 | 4.129 | 1272.326 | 0.482 | 10.054 |
| VMess WS | 21.861 | 8.162 | 1283.053 | 0.541 | 10.352 |
| VLESS WS | 19.503 | 6.797 | 1310.752 | 0.359 | 9.967 |
| Trojan | 31.957 | 4.139 | 1298.415 | 0.464 | 9.930 |
| Hysteria2 | 29.805 | 4.471 | 1309.676 | 0.455 | 10.164 |
| Surge SS | 78.871 | 72.950 | 1396.436 | 0.633 | 11.072 |
| QX SS | 164.350 | 71.875 | 1392.769 | 0.787 | 10.792 |
| Clash JSON | 59.403 | 8.001 | 1287.859 | 0.506 | 10.447 |
| Single-node Clash YAML | 125.048 | 10.790 | 1283.154 | 0.717 | 10.290 |

The first run confirms the same direction on every format. Representative
Node/Rust reused-parser means were SS `10.337 / 5.344 us`, VMess
`29.349 / 10.895 us`, Surge SS `110.822 / 93.032 us`, and YAML
`169.964 / 14.218 us`; its HTTP medians span `0.434–0.832 ms` for Node and
`12.216–12.662 ms` for Rust. Across both runs, reused-parser medians favor Rust
by approximately `1.08–11.95x`, with a small advantage for Surge SS. In the
latest run, the seven Surge SS block means range `75.266–84.768 us` for Node
and `72.674–73.695 us` for Rust. HTTP remains roughly `14–28x` slower on Rust
in the latest comparison; the SS endpoint P95 is `0.642 ms` for Node versus
`11.108 ms` for Rust. Raw blocks, iterations and all request latencies remain
in each run's `metrics.json`; the second run also prints bounded block/P95 data.

The second run then starts a separate instance of the same Rust binary with the
existing profiler enabled. All nine formats retain exact endpoint output and
each contributes seven complete, balanced profiles after warm-up, with no lost
or unfinished scopes. SS URI median active stage durations are:

| Stage | Active ms |
|---|---:|
| Tokio executor setup | 0.025 |
| State snapshot | 0.018 |
| QuickJS runtime setup | 0.013 |
| QuickJS context setup | 0.096 |
| Script job | 9.781 |
| Application bootstrap | 8.577 |
| Bootstrap bytecode load, nested | 4.059 |
| Bootstrap evaluation, nested | 4.641 |
| State JSON encode, nested | 0.004 |
| State JSON parse, nested | 0.004 |
| Parser initialization | 1.041 |
| Native parsing | 0.036 |
| Production | 0.020 |
| Output encoding | 0.011 |

These are instrumented elapsed active times, not OS CPU samples. Parent/child
durations overlap, and independently computed medians must not be added. They
do not enter the uninstrumented request table. The `script::run` path creates a
job, context and application bindings per request; `script::parser` lazily
constructs the additional parser in that job. Their fixed initialization cost
dominates one-node HTTP latency. Faster bulk conversion does not remove it.
Reducing per-request application/bootstrap and parser setup is the next target;
the measured hot parser already wins on these fixtures. Surge/QX's roughly
`72–73 us` reused-parser cost remains a separate format-specific opportunity.

Both runs pass formatting, clippy with warnings denied and 39 ordinary Rust
tests. Four ignored probes include the new interactive driver, which this
workflow explicitly executes in release mode; the second run also passes all
25 existing benchmark self-checks. The full frontend/native/Docker acceptance
matrix is outside this focused measurement. Its earlier product-source evidence
is retained above. Cancelled setup runs `39700896530` and `39701048608`, and
formatting failure `39701116768`, supply no accepted performance numbers.
Artifacts `single-node-39701402820-1` and `single-node-39702044025-1` retain the
public samples and runner logs for 30 days. No local product execution or
artifact extraction was used.

### Direct parse bootstrap specialization (2026-10-22)

The accepted same-runner comparison is
[39717747810](https://github.com/DSLZL/Sub-Store/actions/runs/39717747810),
source `87a105287588c4dc9526a9313889615d2856fbbe`. It compares the pinned
original, Rust `bfae5be4ff67a7bfc9be70ea838e28dce5c0bf02`, and the candidate
on one Ubuntu x64 runner. Baseline and candidate have separate Cargo output
directories and distinct binary hashes, recorded in `metrics.json` and the log.
The earlier shared-output comparison `39713194124` is not accepted: its final
candidate service build reused existing release output. It supplies no speedup
evidence. The isolated intermediate comparison `39713856286` measured a
36.0–38.1% reduction before the final specialization below.

Only direct `parse` jobs with a JavaScript-falsy `download` use the smaller
environment. They still execute the existing value bridge, language operations,
native parser and producer, IP predicates and random-port helper. Each request
retains its own engine/context/parser and normal state handling. Downloads and
every script-capable operation retain complete initialization. No transformed
node, live engine, callback or response is shared between requests.

The final driver also checks the existing public Surge fixture's 16 lines
individually and as one mixed input on all three backends: 51 untimed
correctness requests. This caught a missing `isIP` binding for AnyTLS with
`sni=off`; the final candidate reuses the original predicate. The native service
suite additionally compares `download=null/false/0/""` with the original.
Failed setup runs `39715021242` and `39716332827`, the Surge failure in
`39715004018`, and its focused reproduction `39716867675` remain historical
evidence; none contributes accepted candidate latency samples.

Timed HTTP methodology remains seven blocks of 30 requests for each of nine
formats and three backends, alternating equivalent raw/newline inputs. All
5,670 timed responses match the original. Profiling stays disabled for these
requests. Median milliseconds per request:

| Input | Node | Previous Rust | Candidate Rust |
|---|---:|---:|---:|
| SS URI | 0.609 | 15.303 | 2.267 |
| VMess WS | 0.635 | 15.358 | 2.301 |
| VLESS WS | 0.602 | 15.261 | 2.292 |
| Trojan | 0.569 | 15.432 | 2.270 |
| Hysteria2 | 0.616 | 15.224 | 2.269 |
| Surge SS | 0.798 | 15.035 | 2.440 |
| QX SS | 1.085 | 15.045 | 2.436 |
| Clash JSON | 0.693 | 14.942 | 2.277 |
| Single-node Clash YAML | 0.954 | 15.054 | 2.287 |

The candidate reduces these HTTP medians by 83.77–85.29%, or 6.16–6.80x.
It remains 2.25–3.99x slower than Node on this endpoint. SS P95 is
`0.883 / 20.003 / 2.455 ms` for Node/previous Rust/candidate. Pure parsing
algorithms are unchanged; the candidate's reused-parser medians still favor
Rust by 1.14–13.04x on these fixtures.

The separate candidate SS profile reports median active `bootstrap=0.192 ms`,
`context_setup=0.118 ms`, `parse_runtime=1.418 ms`, `parse_native=0.020 ms`,
`produce=0.019 ms` and `output_encode=0.012 ms`. Parser initialization now
dominates the remaining cost. These are instrumented elapsed times, not OS CPU
measurements. Library-load/evaluation spans also occur inside the independent
parser; they are not exclusively children of the application bootstrap. Do not
add overlapping stages or compare their independently computed medians as sums.

The focused run passes formatting, clippy with warnings denied, 39 ordinary
Rust tests, 25 harness self-checks and all 63 balanced profiles. Artifact
`single-node-39717747810-1` preserves samples and public logs for 30 days.
All six jobs pass in
[39717742648](https://github.com/DSLZL/Sub-Store/actions/runs/39717742648):
native Linux x64/arm64 and Windows x64, Docker Linux x64/arm64, and the
unchanged official frontend in headless Chrome.
The earlier intermediate native run `39713194216` retained a Windows export
timestamp boundary failure of about 0.6 ms; this optimization does not change
export clocks or relax that assertion.

These empty-state, localhost single-node observations do not establish full
imported-subscription throughput, memory savings or release readiness. The
supplied subscription long test was not repeated. All builds and product tests
ran in Actions, with no local product execution or artifact extraction.

### Lazy generic-parser initialization (2026-10-22)

[39728781380](https://github.com/DSLZL/Sub-Store/actions/runs/39728781380)
passes at `ef44869c214ee669ec0af9a3cada7a54b5a7e6c7`, comparing the pinned
Node original, previous Rust `87a105287588c4dc9526a9313889615d2856fbbe`, and
the candidate on one Ubuntu x64 runner. Use this run's paired values; absolute
latencies differ from the earlier runner. Separate output directories and
recorded distinct binary hashes prevent baseline/candidate build reuse.

The private generic parser now creates its context on first use and loads the
existing JSON5/YAML bundle only when a fallback or YAML dump needs it. The five
URI fixtures create neither. The existing fast paths for strict JSON and
ordinary subscription YAML retain actual QuickJS JSON semantics without eagerly
loading the generic bundle. Fallback precedence, typed values, error recovery and
per-job isolation are unchanged.
No live runtime, context or transformed result is shared between requests.

Median HTTP milliseconds per request, with profiling disabled:

| Input | Node | Previous Rust | Candidate Rust |
|---|---:|---:|---:|
| SS URI | 0.484 | 1.472 | 0.435 |
| VMess WS | 0.506 | 1.500 | 0.443 |
| VLESS WS | 0.311 | 1.450 | 0.423 |
| Trojan | 0.321 | 1.472 | 0.504 |
| Hysteria2 | 0.306 | 1.479 | 0.429 |
| Surge SS | 0.576 | 1.596 | 1.606 |
| QX SS | 0.728 | 1.576 | 1.566 |
| Clash JSON | 0.460 | 1.495 | 0.555 |
| Single-node Clash YAML | 0.643 | 1.509 | 0.540 |

Seven formats improve by about 63–71% against previous Rust. SS, VMess and
Clash YAML have lower medians than Node in this run; the other formats do not.
Surge/QX medians are essentially unchanged. Their profiles still show one
generic context initialization (0.140–0.141 ms) and one bundle evaluation
(0.748–0.751 ms) per request. On the first classic-format line,
`conversion::parse` still attempts `clash::Parser::line` before `classic::parse`;
preserving that precedence retains bundle loading on this path. The five URI
profiles assert zero generic contexts; those five plus Clash JSON/YAML assert
zero generic bundle initializations.

Fresh-parser URI calls take 3.603–7.723 us; their parser objects no longer create
an unused engine. Reused-parser medians remain 3.574–69.108 us. The candidate
SS profile reports `context_setup=0.062 ms`, `bootstrap=0.106 ms`,
`parse_native=0.012 ms` and `job=0.157 ms`; overlapping active elapsed stages
are not OS CPU measurements. Tail latency does not improve uniformly: Surge
HTTP P95 is 1.948 ms for previous Rust and 2.563 ms for the candidate, while
SS is 1.533 ms and 0.548 ms. These samples do not establish a tail-latency win.

All 5,670 timed responses, 51 public Surge preflight responses and 63 separate
balanced profiles pass, along with formatting, clippy, 40 ordinary Rust tests
and 25 harness self-checks. The earlier formatting failures at `57ee5a57`
(runs `39722969899` and `39722969874`) remain evidence and supply no accepted
performance samples. Artifacts remain on Actions for 30 days; none was
extracted locally. The supplied-subscription long test was not repeated, and
this single-node measurement does not establish memory savings or release
readiness.

All six jobs pass in the separate compatibility run
[39728781382](https://github.com/DSLZL/Sub-Store/actions/runs/39728781382),
at the same candidate source. Native Linux x64/arm64, Docker Linux x64/arm64,
Windows x64 and the unchanged official frontend in headless Chrome passed.
The final Windows result was confirmed after the earlier GitHub connection EOF
errors cleared; its HTTP/state/restart/rollback checks passed.

### Surge/QX scalar probes before generic initialization (2026-10-22)

[39735798119](https://github.com/DSLZL/Sub-Store/actions/runs/39735798119)
passes at `1cf6c2b175cee2773b25ce700b52d7ef7b2ed05e`, comparing the pinned
Node original, Rust `ef44869c214ee669ec0af9a3cada7a54b5a7e6c7`, and the
candidate on one Ubuntu x64 runner. Baseline and candidate use separate build
directories and have distinct binary hashes. Compare these paired observations;
absolute times from the earlier run came from a different runner.

Before starting the generic engine, `clash::Parser::line` reuses the existing
conservative YAML reader to reject confirmed scalar strings without an object
delimiter. Ordinary Surge/QX lines then reach the existing classic parser
without creating a generic context or loading its libraries. JSON/JSON5 objects,
tagged YAML and ambiguous inputs retain the previous fallback order. Nothing
changes in the five URI parser routes. Every request retains its private
application context; no live engine or converted result is reused.

Median uninstrumented HTTP milliseconds per request:

| Input | Node | Previous Rust | Candidate Rust |
|---|---:|---:|---:|
| SS URI | 0.569 | 0.767 | 0.815 |
| VMess WS | 0.672 | 0.877 | 0.873 |
| VLESS WS | 0.673 | 0.864 | 0.773 |
| Trojan | 0.708 | 0.749 | 0.710 |
| Hysteria2 | 0.453 | 0.731 | 0.836 |
| Surge SS | 0.768 | 2.573 | 0.712 |
| QX SS | 1.164 | 2.570 | 0.843 |
| Clash JSON | 0.592 | 1.154 | 1.015 |
| Single-node Clash YAML | 1.054 | 1.171 | 1.013 |

Surge/QX HTTP medians fall by 72.3%/67.2% against previous Rust and by
7.3%/27.6% against Node. Their HTTP P95 values are respectively
`1.024 / 2.762 / 0.870 ms` and `1.477 / 2.795 / 0.944 ms` for
Node/previous Rust/candidate. These focused gains do not establish a speedup
for every input: several other formats still have higher HTTP medians than
Node, and unchanged URI routes fluctuate against previous Rust in this run.

Pure parsing remains separate from HTTP. Fresh-parser Surge/QX calls take
`6.972 / 6.858 us`, against Node's `126.092 / 267.710 us`, or 18.09x/39.04x
faster. Reused-parser candidate medians are `6.977 / 6.906 us`. Those ratios
must not be presented as request-level speedups.

The separate Surge/QX profiles report median active `context_setup=0.107 /
0.112 ms`, `bootstrap=0.176 / 0.176 ms`, `parse_scalar_probe=0.014 /
0.013 ms`, `parse_native=0.031 / 0.029 ms` and `produce=0.019 / 0.019 ms`.
The probe is nested within parsing. Parent/child stages overlap; independent
medians cannot be added, and these are elapsed active spans, not OS CPU time.
Application initialization remains a larger cost than native parsing. The
native producer still requires the script layer's filtering, preparation and
JavaScript value semantics; bypassing that layer is not a verified substitute.

All 5,670 timed responses, 66 untimed public correctness responses and 63
balanced profiles pass. The additional preflight cases cover ignored strings,
minimal YAML, a comment-prefixed JSON5 object and a QX name resembling YAML.
All seven native-format profiles assert zero generic contexts; all nine
fixtures assert zero generic library initializations. Formatting, clippy,
41 ordinary Rust tests and 25 harness self-checks also pass. Samples and logs
remain in Actions; no artifacts were downloaded or extracted locally. The
supplied-subscription long test was not repeated, and this localhost
single-node comparison does not establish memory savings or release readiness.

All six jobs also pass in
[39735798237](https://github.com/DSLZL/Sub-Store/actions/runs/39735798237)
at the same source: native Linux x64/arm64 and Windows x64, Docker Linux
x64/arm64, and the unchanged official frontend in headless Chrome.

### Reuse compilation of the JS value bridge (2026-10-22)

[39740799145](https://github.com/DSLZL/Sub-Store/actions/runs/39740799145)
passes at `7d034cd522778241f49b902dec61f18634f00862`, comparing the pinned
Node original and Rust `1cf6c2b175cee2773b25ce700b52d7ef7b2ed05e` with the
candidate on one Ubuntu x64 runner. Separate build directories and distinct
binary hashes are recorded as before. Use these paired observations, rather
than comparing absolute times between runners.

Application bootstrap already reused compiled fixed libraries, but the value
bridge still compiled its inline expression on every initialization. Its
unchanged source now uses the existing private, immutable bytecode cache.
`Script::eval_value` returns the newly evaluated object in the current context;
`js_values::initialize` installs it at the same global property. Each engine
still constructs fresh functions and objects. The first use in a process still
compiles and serializes the source. No user script, live context, converted node
or mutable bridge is cached.

Median uninstrumented HTTP milliseconds per request:

| Input | Node | Previous Rust | Candidate Rust |
|---|---:|---:|---:|
| SS URI | 0.590 | 0.801 | 0.630 |
| VMess WS | 0.737 | 0.817 | 0.670 |
| VLESS WS | 0.622 | 0.825 | 0.677 |
| Trojan | 0.558 | 0.770 | 0.645 |
| Hysteria2 | 0.575 | 0.790 | 0.658 |
| Surge SS | 0.874 | 0.806 | 0.651 |
| QX SS | 1.206 | 0.811 | 0.668 |
| Clash JSON | 0.655 | 1.069 | 0.796 |
| Single-node Clash YAML | 1.044 | 1.080 | 0.806 |

All nine HTTP medians fall by 16.2–25.5% against previous Rust. Surge/QX fall
by 19.2%/17.6% against previous Rust and 25.5%/44.6% against Node. VMess and
Clash YAML also have lower medians than Node; the other five fixtures do not.
These are fixture medians, not a universal throughput or tail-latency claim.
Pure parser algorithms are unchanged. Candidate fresh-parser Surge/QX medians
are `7.711 / 7.676 us`; Clash JSON/YAML are `133.873 / 138.895 us`.

Separate Surge/QX profiles report median active `bootstrap_values=0.015 /
0.014 ms`, `bootstrap=0.064 / 0.063 ms`, `context_setup=0.111 / 0.108 ms`,
`job=0.170 / 0.164 ms`, `parse_native=0.035 / 0.033 ms` and
`produce=0.018 / 0.017 ms`. Context creation is now the largest reported
fixed setup stage. These active elapsed spans do not isolate all HTTP transport
or scheduling costs, and are not OS CPU time. Nested stages overlap and their
independently computed medians must not be added.

Formatting, clippy, 42 ordinary Rust tests, 25 harness self-checks, all 66 public
preflight responses, 5,670 timed responses and 63 balanced profiles pass. The
new returned-object test verifies independent mutable objects and methods
across runtimes after bytecode loads; the existing number, cycle, getter,
exception and concurrent-context checks remain in place. All seven native
fixtures still initialize no generic context, and all nine load no generic
library bundle. Builds, product execution and measurements ran only in Actions;
no artifacts were downloaded or extracted locally. The supplied-subscription
long test was not repeated; cold-process latency was not measured in this run.

All six jobs pass in
[39740799201](https://github.com/DSLZL/Sub-Store/actions/runs/39740799201)
at the same source: native Linux x64/arm64 and Windows x64, Docker Linux
x64/arm64, and the unchanged official frontend in headless Chrome. The Windows
HTTP/state/restart/rollback, Rust JS and original Node regression steps passed.

### Initialize conversion and parser intrinsics on demand (2026-10-22)

The first accepted measurement is
[39747263963](https://github.com/DSLZL/Sub-Store/actions/runs/39747263963)
at `81707dbc696b22c5197657f9c18e21a88364eea4`. It compares the pinned Node
original, Rust `7d034cd522778241f49b902dec61f18634f00862` and the candidate
on one runner, with separate build directories and distinct binary hashes.
The earlier `74475486` attempt
[39746997402](https://github.com/DSLZL/Sub-Store/actions/runs/39746997402)
stopped at two rustfmt differences before measurements; the format-only fix is
included in `81707dbc`. Its superseded compatibility run was cancelled.

Direct HTTP conversion now creates only base objects, Eval, RegExpCompiler,
RegExp, Json, MapSet and Promise. Downloading and all other application jobs
retain the full context. The separate generic parser starts with base objects,
Eval, Json and MapSet, then adds all omitted intrinsics before loading its fixed
JSON5/YAML library. Strict JSON still uses actual QuickJS parsing and the existing
lossless value bridge. No user script can mutate the private parser's globals;
existing JSON/MapSet identities survive fallback initialization. Fresh contexts,
numeric semantics, ordering and fallback precedence remain unchanged.

In this first paired run all nine HTTP medians improve by approximately 5–14%
against previous Rust. Fresh-parser Clash JSON is `95.566 us` against Node's
`104.844 us`; all nine candidate fresh-parser fixture medians are below Node.
Separate SS/Surge/QX profiles report application context active medians
`0.076 / 0.068 / 0.069 ms`; generic Clash JSON/YAML context medians are
`0.077 / 0.078 ms`. These are active elapsed spans, not CPU measurements, and
do not isolate all transport or scheduling costs. They are not paired component
speedup estimates against profiles recorded on a different runner.

Formatting, clippy, 43 ordinary Rust tests, 25 harness self-checks, 66 public
preflight responses, 5,670 timed responses and 63 balanced profiles pass.
The strict JSON/JSON5/YAML differential check now uses a separate full-context
oracle; the new application-context test checks language operations and isolation
in fresh reduced/full engines. Five compatibility jobs passed at `81707dbc` in
[39747198498](https://github.com/DSLZL/Sub-Store/actions/runs/39747198498);
the remaining Windows job was cancelled when the next candidate started, so
this run is not a complete six-job acceptance.

`401595abaa1001f88162f9e7f2305f849435b913` additionally avoids constructing
another Tokio executor for the same direct-conversion route. The existing
Tokio handle polls its future on the blocking worker; the per-job QuickJS
runtime/context, parser, state snapshot, response delivery and runtime drain
are retained. Downloading and other operations keep their per-job executor.
If this route gains I/O or user callbacks, restore the full executor and
intrinsics before exposing the new behavior.

[39748733206](https://github.com/DSLZL/Sub-Store/actions/runs/39748733206)
passes at that source with the same pinned Node and previous Rust sources.
The following are same-run medians; do not use the earlier runner's absolute
values to estimate the executor change alone.

| Input | Node pure us | Rust reused-parser us | Rust fresh-parser us | Node HTTP ms | Previous Rust HTTP ms | Candidate HTTP ms |
|---|---:|---:|---:|---:|---:|---:|
| SS URI | 15.310 | 6.629 | 6.655 | 0.673 | 0.674 | 0.606 |
| VMess WS | 42.942 | 13.442 | 13.551 | 0.667 | 0.713 | 0.641 |
| VLESS WS | 33.889 | 11.076 | 11.174 | 0.557 | 0.710 | 0.641 |
| Trojan | 54.560 | 6.360 | 6.353 | 0.640 | 0.677 | 0.616 |
| Hysteria2 | 52.011 | 6.930 | 6.940 | 0.616 | 0.682 | 0.611 |
| Surge SS | 166.868 | 8.224 | 8.197 | 0.837 | 0.683 | 0.621 |
| QX SS | 337.841 | 8.274 | 8.283 | 1.081 | 0.684 | 0.621 |
| Clash JSON | 105.659 | 15.191 | 94.119 | 0.649 | 0.828 | 0.719 |
| Single-node Clash YAML | 258.222 | 20.416 | 100.276 | 1.027 | 0.841 | 0.751 |

All nine candidate HTTP medians are 9.0–13.2% below previous Rust. Seven are
below Node; VLESS WS and Clash JSON remain 15.1% and 10.8% above Node. All nine
fresh-parser medians are below Node. This resolves the measured fresh-parser
Clash JSON disadvantage, while leaving two complete HTTP median gaps explicit.
The finite 210-request candidate P95 for each fixture is below both other
backends in this run; this does not establish general tail-latency performance.

Separate SS/Surge/QX profiles round executor setup to `0.000 ms` and report
application context active medians `0.069 / 0.069 / 0.067 ms`, bootstrap
`0.058 / 0.059 / 0.058 ms`, native parsing `0.017 / 0.028 / 0.028 ms` and
production `0.016 / 0.017 / 0.017 ms`. Generic Clash JSON/YAML context medians
are both `0.079 ms`. Every native-format profile has zero generic contexts,
and all nine fixtures load no generic library bundle. The profile does not
fully attribute HTTP transport, worker-queue delay or post-response teardown.
Nested spans overlap; active elapsed time is not OS CPU time.

Formatting, clippy, 43 ordinary Rust tests, 25 harness self-checks, all 66 public
preflight responses, 5,670 timed responses and 63 balanced profiles pass again.
The measured candidate/baseline SHA256 values are distinct. All compilation,
product execution, tests and measurements ran in Actions; no artifacts were
downloaded or extracted locally. Neither the supplied-subscription long test
nor cold-process latency was measured in these two runs. No memory, universal
protocol, throughput or release-readiness claim follows from these fixtures.


The compatibility matrix at the final source is
[39748732183](https://github.com/DSLZL/Sub-Store/actions/runs/39748732183).
Attempt 1 passed Linux x64/arm64, both Docker architectures and the unchanged
frontend. Windows passed formatting, lint, 43 Rust tests, packaging and its
native JavaScript checks, but the service contract runner timed out at step 7
of `T02-merged-share-count-and-origin`: standalone frontend
`POST /share/file/merged-share?token=limited`. Subsequent ordered share cases
were not run. This non-GET error path does not execute the conversion job.
The original five-second timeout and failed evidence remain unchanged.

The separate, temporary-branch diagnostic
[39752607561](https://github.com/DSLZL/Sub-Store/actions/runs/39752607561)
inspected the retained failure and ran the exact delivered Windows executable
(SHA256 `1eaa389a38158d5a4c0ec6cf74b2bf02b030924355c199e9defb896aa1332cc2`)
on a fresh Windows Actions runner. Two fresh non-production processes returned
the first non-GET sharing error in approximately 87 ms and 43 ms. Neither
reproduced the five-second timeout. This does not establish the original cause;
no production error handling, stack formatting or contract timeout was changed.
The diagnostic downloaded artifacts only inside Actions, with no local
artifact download, extraction, build or product execution.

Attempt 2 reran only the Windows job at the unchanged `401595ab` source.
[Windows job 119736564310](https://github.com/DSLZL/Sub-Store/actions/runs/39748732183/job/119736564310)
passed its actual-binary HTTP/state/restart/rollback contracts, native Rust JS
probe, Node HTTP contracts and Node regression suite. With the original five
successful jobs retained, the matrix now has six successful jobs. The first
Windows timeout remains a retained, unreproduced failure with an unconfirmed
cause; this rerun is not evidence that an error-path defect was fixed. The
temporary diagnostic branch and local plaintext workflow drafts were removed.

### All accepted proxy node types and URI input paths (2026-10-22)

[39756957657](https://github.com/DSLZL/Sub-Store/actions/runs/39756957657)
passes at `6cbeea75081cc0b8405e02bd5613b782af34eea9`. This change extends
the public fixtures and coverage assertions; production Rust remains unchanged
from `401595ab`. Its delivered binary SHA256 is again
`efd5abf9c5a1cad25e0efe565e409bdcd785871ed2f5001403ce76a21258f420`.
The same-run previous Rust binary remains
`6de97e4e8c6eb8a38117b1de45c745633533c3d752851dbef13001f833d07c30`
at `7d034cd522778241f49b902dec61f18634f00862`; Node remains pinned to
`9cc6b5fac966993364ae33da8797f446bbf7f571`.

The 48 public fixtures cover every one of the pinned original's 29 accepted
Clash node types, including its `masque-surge` type, plus Surge `external`.
An assertion extracts the original `Clash_All` allowlist and rejects missing
or extra types. Fifteen URI fixtures exercise HTTP/HTTPS, SOCKS5 with and
without TLS, Base64 SOCKS, SS, SSR, VMess WS, VLESS WS, Trojan, Hysteria 1/2,
TUIC, WireGuard and AnyTLS. Surge SS, QX SS and single-node Clash YAML remain
separate format samples. All fields are public fixture data. External command
configuration is parsed only; no proxy connection or external command is run.

The same runner alternates all three backends over seven blocks. Pure parsing
uses the median of seven block means in microseconds per call; HTTP uses the
median of 210 requests per input/backend in milliseconds. Equivalent original
and newline inputs alternate. Timed requests retain profiling-off behavior and
include JSON production and HTTP transport. Rust reused/fresh parser results
remain separate. Node/Rust function outputs and complete endpoint responses
must match before a sample is accepted.

Formatting, clippy, 43 ordinary Rust tests, 25 harness self-checks, 66 public
preflight responses, all 48 pure cases, 30,240 timed HTTP responses and
336 separate balanced profiles pass in Actions. No local product execution,
build, test or artifact download/extraction was used.

| Case | Protocol | Input | Node pure us | Rust reused parser us | Rust fresh parser us | Node HTTP ms | Previous Rust HTTP ms | Rust HTTP ms |
|---|---|---|---:|---:|---:|---:|---:|---:|
| ss_uri | ss | URI | 15.418 | 6.621 | 6.648 | 0.671 | 0.662 | 0.609 |
| vmess_ws | vmess | URI | 41.599 | 13.610 | 13.529 | 0.622 | 0.679 | 0.643 |
| vless_ws | vless | URI | 33.651 | 11.571 | 11.703 | 0.549 | 0.682 | 0.639 |
| trojan | trojan | URI | 54.246 | 6.790 | 6.779 | 0.561 | 0.666 | 0.608 |
| hysteria2 | hysteria2 | URI | 51.601 | 7.232 | 7.275 | 0.546 | 0.656 | 0.617 |
| surge_ss | ss | Surge | 164.402 | 8.167 | 8.213 | 0.820 | 0.661 | 0.616 |
| qx_ss | ss | QX | 331.432 | 8.027 | 8.094 | 1.031 | 0.669 | 0.618 |
| clash_json | ss | Clash JSON | 103.535 | 15.152 | 93.928 | 0.672 | 0.838 | 0.706 |
| clash_yaml | ss | Clash YAML | 255.367 | 20.341 | 99.889 | 1.025 | 0.827 | 0.712 |
| clash_zerotier | zerotier | Clash JSON | 93.461 | 11.960 | 89.841 | 0.603 | 0.837 | 0.689 |
| clash_shadowquic | shadowquic | Clash JSON | 97.343 | 13.253 | 91.293 | 0.607 | 0.815 | 0.715 |
| clash_gost_relay | gost-relay | Clash JSON | 100.439 | 13.921 | 92.101 | 0.629 | 0.804 | 0.720 |
| clash_openvpn | openvpn | Clash JSON | 100.309 | 14.248 | 92.395 | 0.625 | 0.848 | 0.705 |
| clash_tailscale | tailscale | Clash JSON | 98.648 | 13.305 | 91.152 | 0.596 | 0.796 | 0.730 |
| clash_easytier | easytier | Clash JSON | 107.121 | 16.527 | 95.675 | 0.626 | 0.843 | 0.733 |
| clash_trusttunnel | trusttunnel | Clash JSON | 101.691 | 14.583 | 92.555 | 0.619 | 0.809 | 0.702 |
| clash_h2_connect | h2-connect | Clash JSON | 101.154 | 14.579 | 92.350 | 0.697 | 0.804 | 0.715 |
| clash_naive | naive | Clash JSON | 94.507 | 13.198 | 91.247 | 0.688 | 0.837 | 0.731 |
| clash_anytls | anytls | Clash JSON | 94.985 | 13.024 | 90.592 | 0.683 | 0.807 | 0.702 |
| clash_mieru | mieru | Clash JSON | 98.882 | 13.921 | 91.904 | 0.689 | 0.840 | 0.715 |
| clash_masque | masque | Clash JSON | 99.557 | 14.290 | 91.682 | 0.612 | 0.830 | 0.722 |
| clash_masque_surge | masque-surge | Clash JSON | 101.912 | 14.461 | 92.181 | 0.612 | 0.830 | 0.736 |
| clash_sudoku | sudoku | Clash JSON | 89.641 | 11.689 | 89.315 | 0.690 | 0.828 | 0.701 |
| clash_juicity | juicity | Clash JSON | 104.264 | 15.118 | 92.624 | 0.635 | 0.812 | 0.705 |
| clash_ssr | ssr | Clash JSON | 106.583 | 15.486 | 93.550 | 0.613 | 0.851 | 0.717 |
| clash_vmess | vmess | Clash JSON | 128.401 | 24.358 | 105.932 | 0.644 | 0.890 | 0.752 |
| clash_socks5 | socks5 | Clash JSON | 101.634 | 14.606 | 92.639 | 0.650 | 0.845 | 0.707 |
| clash_http | http | Clash JSON | 98.226 | 13.865 | 92.353 | 0.628 | 0.814 | 0.703 |
| clash_snell | snell | Clash JSON | 96.930 | 12.554 | 91.063 | 0.653 | 0.824 | 0.712 |
| clash_trojan | trojan | Clash JSON | 98.490 | 13.509 | 91.636 | 0.679 | 0.844 | 0.706 |
| clash_tuic | tuic | Clash JSON | 109.681 | 15.507 | 94.264 | 0.695 | 0.855 | 0.725 |
| clash_vless | vless | Clash JSON | 124.259 | 21.748 | 103.146 | 0.724 | 0.879 | 0.756 |
| clash_hysteria | hysteria | Clash JSON | 107.727 | 15.637 | 94.523 | 0.618 | 0.816 | 0.738 |
| clash_hysteria2 | hysteria2 | Clash JSON | 107.202 | 15.089 | 94.169 | 0.603 | 0.821 | 0.739 |
| clash_wireguard | wireguard | Clash JSON | 131.056 | 19.941 | 99.342 | 0.655 | 0.857 | 0.747 |
| clash_ssh | ssh | Clash JSON | 96.040 | 12.430 | 90.515 | 0.680 | 0.835 | 0.714 |
| clash_direct | direct | Clash JSON | 75.921 | 7.322 | 82.990 | 0.563 | 0.782 | 0.670 |
| surge_external | external | Surge | 189.483 | 7.364 | 7.394 | 0.838 | 0.665 | 0.614 |
| ssr_uri | ssr | URI | 24.451 | 6.793 | 6.824 | 0.475 | 0.658 | 0.604 |
| tuic_uri | tuic | URI | 38.385 | 5.877 | 5.876 | 0.573 | 0.659 | 0.615 |
| wireguard_uri | wireguard | URI | 43.977 | 7.547 | 7.612 | 0.534 | 0.698 | 0.630 |
| hysteria_uri | hysteria | URI | 46.893 | 5.707 | 5.698 | 0.583 | 0.660 | 0.595 |
| anytls_uri | anytls | URI | 67.317 | 9.515 | 9.536 | 0.658 | 0.674 | 0.617 |
| http_uri | http | URI | 3.116 | 3.954 | 3.953 | 0.512 | 0.639 | 0.599 |
| https_uri | http | URI | 3.452 | 4.215 | 4.212 | 0.492 | 0.646 | 0.599 |
| socks5_uri | socks5 | URI | 3.251 | 4.033 | 4.039 | 0.527 | 0.653 | 0.590 |
| socks5_tls_uri | socks5 | URI | 3.558 | 4.335 | 4.371 | 0.477 | 0.657 | 0.591 |
| socks_uri | socks5 | URI | 9.896 | 3.931 | 3.938 | 0.472 | 0.646 | 0.590 |

In this run the observed Rust reused-parser median is below Node in 44/48
fixtures, and the fresh-parser median in 43/48. The five fresh-parser exceptions
are Direct JSON (+9.3%) and HTTP, HTTPS, SOCKS5 and SOCKS5+TLS URI (+22.0–26.9%).
The latter four also trail Node with a reused parser.

Complete Rust HTTP medians are below Node in 6/48 fixtures: SS URI, AnyTLS URI,
Surge SS, QX SS, Surge external and single-node Clash YAML. The other 42 are
above Node, including every one of the 29 Clash JSON fixtures. All 48 Rust HTTP
medians improve against previous Rust by approximately 5.3–17.7%. The earlier
seven-of-nine observation belongs only to its named run and fixture set.

These are finite measurements of configuration parsing/conversion with JSON
output. They do not measure proxy-protocol connection throughput, every
transport/option combination, cold-process latency or the imported full
subscription. The added cases do not establish new platform or release
acceptance; the unchanged production code retains its separately recorded
six-job compatibility result.


## Per-request state and parser allocations (2026-10-22)

[39763519388](https://github.com/DSLZL/Sub-Store/actions/runs/39763519388)
passes at `9c19111bae2af2654caa339151e4eabd12b586bb`, against original Node
`9cc6b5fac966993364ae33da8797f446bbf7f571` and previous Rust
`274a4ff1f63fdbe997dda0e0dcb99daf68135d06`. Rust 1.98.0 and Node
24.16.0 run on the same Ubuntu 24.04 x64 runner. Candidate SHA256 is
`7a54a67dd08697d540c800a9d7f6d834a3347ef1b237b0091d7e1a4b8e25fca7`;
previous Rust is
`906c03638cb1d9f36ce66400769adb82510d2d2ff1c623c25f74fbe520781fad`.

Conversion-only jobs no longer lock, read or clone the complete application
store, or encode its unused baseline. Downloading and script jobs keep their
state handling. Parsing borrows ordinary UTF-8 input, retains the checked
UTF-16-marker path, skips impossible Base64 probes when a colon is present,
and copies only option keys that need normalization. Percent-free components
avoid a redundant decoding pass. The common parser and producer still execute;
no live engine pool, converted-node cache or result cache is introduced.

The previous and current release parser drivers now both measure reused and
fresh parsers, with equivalent original/newline inputs alternating. Actual
HTTP measurements cover both new connections and reused connections; the latter
check socket identity after every response. All 48 fixtures, including the 29
pinned Clash node types, must retain exactly equal Node/Rust outputs.

Same-run observations, using the reported rounded medians:

- All 48 reused-parser medians improve by 12.9–39.9% over previous Rust;
  the median per-fixture reduction is 19.9%. All 48 fresh-parser medians
  improve by 2.3–39.5%, with a median reduction of 6.0%.
- Reused Rust parsing is below Node in all 48 cases; fresh Rust is below
  Node in 47. Direct Clash JSON remains 82.651 us versus Node's 76.344 us,
  8.3% higher. The four HTTP/HTTPS/SOCKS URI disadvantages are removed.
- All 48 new-connection HTTP medians improve over previous Rust by
  0.6–6.9%, with a median reduction of 4.0%. Rust is below Node in 12/48.
- Reused-connection medians improve in 45 cases, have one rounded tie, and
  increase by less than 0.33% in OpenVPN/WireGuard. The median reduction
  is 4.8%; Rust is below Node in 8/48. These finite samples do not establish
  a universal advantage or a statistically established change for tiny gaps.
- Finite new-connection P95 values fall in all 48 cases, with a median
  per-fixture reduction of 5.0%. Keep-alive P95 values fall in 46/48, with
  a median reduction of 5.9%; this does not establish general tail latency.

Pure columns are medians of seven block means. Each HTTP column is the median
of 210 requests. Connections are measured separately; request decoding, normal
production and complete response checking remain enabled.

| Case | Node pure us | Previous Rust reused / fresh us | Rust reused / fresh us | New connection: Node / previous / Rust ms | Keep-alive: Node / previous / Rust ms |
|---|---:|---:|---:|---:|---:|
| ss_uri | 15.074 | 6.659 / 6.704 | 4.609 / 4.609 | 0.652 / 0.619 / 0.592 | 0.460 / 0.502 / 0.481 |
| vmess_ws | 41.327 | 13.395 / 13.494 | 10.690 / 10.702 | 0.624 / 0.651 / 0.614 | 0.571 / 0.551 / 0.524 |
| vless_ws | 34.022 | 11.225 / 11.297 | 8.272 / 8.302 | 0.577 / 0.649 / 0.629 | 0.471 / 0.543 / 0.502 |
| trojan | 55.145 | 6.974 / 7.008 | 4.678 / 4.678 | 0.558 / 0.625 / 0.603 | 0.509 / 0.516 / 0.490 |
| hysteria2 | 52.060 | 7.324 / 7.351 | 4.827 / 4.826 | 0.627 / 0.625 / 0.594 | 0.545 / 0.517 / 0.489 |
| surge_ss | 166.079 | 8.876 / 8.890 | 6.870 / 6.848 | 0.812 / 0.628 / 0.613 | 0.698 / 0.523 / 0.493 |
| qx_ss | 335.015 | 8.740 / 8.764 | 6.081 / 6.098 | 1.015 / 0.633 / 0.615 | 0.886 / 0.522 / 0.497 |
| clash_json | 105.130 | 15.480 / 96.132 | 12.452 / 91.494 | 0.705 / 0.716 / 0.693 | 0.531 / 0.616 / 0.587 |
| clash_yaml | 258.391 | 20.977 / 102.663 | 18.280 / 98.705 | 1.029 / 0.736 / 0.700 | 0.842 / 0.621 / 0.592 |
| clash_zerotier | 93.917 | 12.313 / 92.863 | 10.059 / 88.745 | 0.699 / 0.717 / 0.683 | 0.566 / 0.599 / 0.576 |
| clash_shadowquic | 98.409 | 13.633 / 94.216 | 10.954 / 89.735 | 0.630 / 0.732 / 0.703 | 0.493 / 0.633 / 0.584 |
| clash_gost_relay | 101.461 | 14.114 / 94.782 | 11.183 / 90.079 | 0.611 / 0.711 / 0.681 | 0.591 / 0.614 / 0.592 |
| clash_openvpn | 101.361 | 14.507 / 95.463 | 11.749 / 90.177 | 0.742 / 0.732 / 0.697 | 0.600 / 0.614 / 0.616 |
| clash_tailscale | 100.358 | 13.547 / 93.688 | 10.745 / 89.208 | 0.607 / 0.737 / 0.715 | 0.543 / 0.628 / 0.602 |
| clash_easytier | 108.666 | 16.930 / 99.639 | 13.573 / 93.515 | 0.755 / 0.737 / 0.704 | 0.580 / 0.625 / 0.597 |
| clash_trusttunnel | 103.164 | 14.821 / 95.873 | 11.899 / 90.246 | 0.610 / 0.737 / 0.690 | 0.578 / 0.623 / 0.591 |
| clash_h2_connect | 106.172 | 14.779 / 96.038 | 11.961 / 90.362 | 0.605 / 0.747 / 0.720 | 0.519 / 0.630 / 0.603 |
| clash_naive | 97.720 | 13.495 / 94.109 | 11.086 / 89.146 | 0.698 / 0.742 / 0.715 | 0.494 / 0.613 / 0.583 |
| clash_anytls | 96.168 | 13.303 / 93.795 | 10.838 / 88.709 | 0.621 / 0.733 / 0.693 | 0.559 / 0.633 / 0.591 |
| clash_mieru | 100.333 | 14.212 / 94.823 | 11.654 / 89.947 | 0.609 / 0.718 / 0.698 | 0.506 / 0.618 / 0.585 |
| clash_masque | 101.292 | 14.365 / 94.634 | 11.636 / 89.616 | 0.608 / 0.727 / 0.691 | 0.499 / 0.641 / 0.617 |
| clash_masque_surge | 103.978 | 14.681 / 95.460 | 11.770 / 89.841 | 0.607 / 0.741 / 0.700 | 0.563 / 0.618 / 0.604 |
| clash_sudoku | 90.684 | 11.967 / 92.043 | 9.842 / 87.844 | 0.690 / 0.728 / 0.716 | 0.581 / 0.629 / 0.580 |
| clash_juicity | 105.869 | 15.414 / 96.749 | 12.142 / 90.155 | 0.631 / 0.729 / 0.698 | 0.493 / 0.642 / 0.592 |
| clash_ssr | 107.784 | 15.943 / 96.850 | 12.595 / 90.846 | 0.672 / 0.733 / 0.692 | 0.500 / 0.619 / 0.595 |
| clash_vmess | 132.667 | 24.980 / 109.044 | 20.755 / 102.238 | 0.637 / 0.760 / 0.732 | 0.527 / 0.677 / 0.625 |
| clash_socks5 | 103.885 | 14.982 / 95.491 | 12.112 / 90.758 | 0.605 / 0.734 / 0.703 | 0.549 / 0.625 / 0.587 |
| clash_http | 100.980 | 14.272 / 94.972 | 11.702 / 90.172 | 0.599 / 0.739 / 0.688 | 0.480 / 0.625 / 0.605 |
| clash_snell | 94.658 | 12.900 / 92.723 | 10.656 / 89.049 | 0.652 / 0.721 / 0.710 | 0.478 / 0.621 / 0.588 |
| clash_trojan | 97.284 | 13.960 / 94.579 | 11.450 / 90.392 | 0.588 / 0.728 / 0.697 | 0.486 / 0.626 / 0.594 |
| clash_tuic | 108.008 | 15.810 / 96.772 | 12.649 / 91.196 | 0.606 / 0.752 / 0.713 | 0.496 / 0.642 / 0.601 |
| clash_vless | 124.448 | 22.451 / 106.924 | 18.649 / 99.530 | 0.616 / 0.751 / 0.723 | 0.512 / 0.645 / 0.617 |
| clash_hysteria | 108.297 | 16.129 / 97.389 | 13.126 / 92.284 | 0.597 / 0.727 / 0.702 | 0.497 / 0.638 / 0.598 |
| clash_hysteria2 | 107.653 | 15.472 / 96.507 | 12.501 / 91.709 | 0.639 / 0.731 / 0.716 | 0.491 / 0.620 / 0.620 |
| clash_wireguard | 132.359 | 20.125 / 101.773 | 15.877 / 95.242 | 0.659 / 0.779 / 0.774 | 0.604 / 0.664 / 0.665 |
| clash_ssh | 96.656 | 12.750 / 92.485 | 10.527 / 88.689 | 0.658 / 0.729 / 0.711 | 0.566 / 0.614 / 0.583 |
| clash_direct | 76.344 | 7.504 / 84.628 | 6.297 / 82.651 | 0.630 / 0.712 / 0.686 | 0.445 / 0.582 / 0.556 |
| surge_external | 190.013 | 7.994 / 8.007 | 6.270 / 6.295 | 0.754 / 0.631 / 0.608 | 0.657 / 0.517 / 0.493 |
| ssr_uri | 24.121 | 6.866 / 6.898 | 4.665 / 4.658 | 0.472 / 0.628 / 0.594 | 0.358 / 0.517 / 0.490 |
| tuic_uri | 38.301 | 5.929 / 5.945 | 3.653 / 3.667 | 0.512 / 0.632 / 0.601 | 0.389 / 0.522 / 0.492 |
| wireguard_uri | 44.185 | 7.763 / 7.752 | 4.662 / 4.691 | 0.524 / 0.669 / 0.646 | 0.439 / 0.559 / 0.528 |
| hysteria_uri | 47.928 | 5.855 / 5.835 | 3.693 / 3.707 | 0.520 / 0.630 / 0.604 | 0.399 / 0.518 / 0.495 |
| anytls_uri | 68.744 | 10.072 / 10.103 | 7.134 / 7.126 | 0.658 / 0.633 / 0.613 | 0.454 / 0.526 / 0.491 |
| http_uri | 3.152 | 4.036 / 4.021 | 2.782 / 2.785 | 0.494 / 0.620 / 0.587 | 0.369 / 0.503 / 0.487 |
| https_uri | 3.525 | 4.285 / 4.291 | 3.028 / 3.015 | 0.494 / 0.622 / 0.595 | 0.380 / 0.504 / 0.490 |
| socks5_uri | 3.293 | 4.115 / 4.088 | 2.803 / 2.805 | 0.513 / 0.622 / 0.600 | 0.371 / 0.510 / 0.479 |
| socks5_tls_uri | 3.653 | 4.399 / 4.402 | 3.047 / 3.051 | 0.509 / 0.616 / 0.603 | 0.374 / 0.499 / 0.489 |
| socks_uri | 10.395 | 4.025 / 3.997 | 2.713 / 2.699 | 0.449 / 0.633 / 0.601 | 0.360 / 0.494 / 0.478 |

The profiler now measures the interval from enqueueing to worker start,
and a separate HTTP service timer stops when the response is ready. For direct
conversion, the job report is emitted after the runtime/context teardown;
HTTP delivery still occurs before draining and teardown, as before. Other
operations retain their earlier report/delivery behavior. The collector waits
for complete worker and HTTP records, validates the public numeric schema,
and rejects extra records or incomplete/unbalanced spans.

Separate profiler-on medians below include instrumentation overhead. Worker
lifetime and response-ready intervals overlap; teardown can finish after the
response. Client measurements also include HTTP transport and client work.
Do not add these columns or subtract independent medians to infer exact costs.
The full 48-case component table and raw profiles are in the run summary/artifact.

| Case | Queue us | Worker through teardown us | Bootstrap us | Native parse us | Produce us | Teardown us | Server response ready us | Client HTTP us |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| http_uri | 20.120 | 382.549 | 54.542 | 10.149 | 16.150 | 65.372 | 323.895 | 644.745 |
| surge_ss | 17.938 | 358.426 | 53.961 | 25.338 | 17.262 | 47.149 | 341.820 | 678.549 |
| qx_ss | 18.730 | 349.648 | 53.270 | 24.285 | 17.302 | 45.725 | 335.347 | 680.412 |
| clash_json | 18.008 | 503.737 | 54.452 | 106.720 | 16.832 | 78.507 | 454.700 | 772.314 |
| clash_wireguard | 20.109 | 607.027 | 56.225 | 119.584 | 41.818 | 113.222 | 496.539 | 812.489 |

For SS/Surge/QX, application-context setup still costs approximately
74/71/70 us and bootstrap 58/54/53 us in these profiles. Native URI parsing
itself is a few microseconds in the separate profiler-off measurements.
Generic JSON adds another private context, and its runtime/context teardown
also increases work per request. These measured fixed costs explain why faster
pure parsing has not made most complete requests faster than Node. The state
snapshot and state-encoding costs removed here round to zero or are absent.

Formatting, clippy, 45 ordinary Rust tests, 27 harness self-checks, 66 public
preflight responses, all 48 pure cases, 60,480 timed HTTP responses and 336
balanced measured worker/HTTP profile pairs pass. Four release-only Rust drivers
remain ignored by the ordinary unit command; the single-node driver is executed
explicitly for the comparison. All compilation, tests, product execution and
measurements ran in Actions; no artifacts were downloaded/extracted locally.
This is a single-node parse/produce comparison with empty application state,
not a supplied-subscription long run, cold-process test, protocol traffic test,
memory assessment or release-readiness claim.

The compatibility matrix at the same final source is
[39763519199](https://github.com/DSLZL/Sub-Store/actions/runs/39763519199):
all six jobs pass, covering native Linux x64/arm64 and Windows x64,
Docker x64/arm64, and the unchanged frontend's headless browser flows.
Native packaging, delivered-binary HTTP/state/restart/rollback checks and
JavaScript/reference checks pass. This does not establish the cause of the
previous unreproduced Windows sharing timeout; its failed evidence and the
original timeouts remain unchanged.


## 2026-10-22: conversion environment reuse and the direct bridge

[Performance run 39774769592](https://github.com/DSLZL/Sub-Store/actions/runs/39774769592)
passes at candidate `af08b0649c4d2f2814fe38ce6943b1397f5e1c14`.
This compares original Node `9cc6b5fac966993364ae33da8797f446bbf7f571`,
previous Rust `9c19111bae2af2654caa339151e4eabd12b586bb` and the candidate
on the same ubuntu-24.04 runner with the existing pinned tools and public fixtures.
The release binary SHA-256 values in that run are:

- Candidate: `3b8bf15faded89b92ee274427a117e0b17f271f14a51dc5a5ac5c8bc229236bd`.
- Previous Rust: `93b345d05cbbd13f0fa6ba70055c39fa2e6fc5a111f0a2c7366f5e5a6f21499b`.

Only direct parse requests with JS-falsy download reuse a worker-local synchronous
QuickJS context. Idle retention is bounded by available CPU parallelism and one
context per worker. Each request still decodes, parses, runs the common producer
and serializes its own result. The private parser clears its last normalized text;
errors, panic, pending jobs and worker exit destroy/release retained environments.
Full scripts/downloads keep fresh contexts and their existing background drain.
The direct path removes dynamic method lookup, argument arrays and Promise wrapping;
it still uses the existing native parser, value codec and common producer.
There is no result cache, new dependency, production option or frontend change.

All 48 fixture P50 values improve against previous Rust in both connection modes.
The median of the 48 candidate/previous ratios is 0.6230 for new connections
(37.7% lower) and 0.5315 for keep-alive (46.9% lower).
New-connection P50 is below Node in 48/48; keep-alive is below Node in 47/48.
The exception is socks5 TLS URI: 0.110 ms versus Node 0.106 ms, still below
previous Rust 0.230 ms. Reused pure parsing is below Node in 48/48 and fresh
parsing in 47/48. This task changes environment/bridge overhead; small changes
in the unchanged pure parser measurements are not attributed to a new parsing algorithm.

These are same-run comparisons. Absolute values from earlier runners are not
interchangeable with this run. Pure values are medians of seven block means;
HTTP P50/P95 use 210 checked responses per fixture/backend/connection mode.
Every row converts one node to JSON/internal. Original/newline inputs alternate.
Coverage includes all 29 pinned Clash types and Surge external, not all possible
protocol fields, output formats or traffic behavior.

In the HTTP columns, values are Node / previous Rust / candidate, in ms.
Pure reused/fresh pairs are in us per call.

| Case | Protocol / input | Node pure | Previous Rust reused / fresh | Candidate reused / fresh | New HTTP P50 N/P/R | Keep-alive P50 N/P/R |
|---|---|---:|---:|---:|---:|---:|
| ss_uri | ss / URI | 7.415 | 2.722 / 2.750 | 2.785 / 2.808 | 0.402 / 0.306 / 0.232 | 0.305 / 0.250 / 0.116 |
| vmess_ws | vmess / URI | 21.539 | 5.662 / 5.579 | 5.700 / 5.658 | 0.515 / 0.326 / 0.228 | 0.366 / 0.277 / 0.153 |
| vless_ws | vless / URI | 17.761 | 4.681 / 4.673 | 4.567 / 4.632 | 0.441 / 0.379 / 0.256 | 0.211 / 0.255 / 0.154 |
| trojan | trojan / URI | 29.242 | 2.655 / 2.640 | 2.710 / 2.741 | 0.302 / 0.295 / 0.245 | 0.202 / 0.213 / 0.116 |
| hysteria2 | hysteria2 / URI | 27.266 | 2.830 / 2.823 | 2.844 / 2.827 | 0.434 / 0.356 / 0.279 | 0.329 / 0.213 / 0.135 |
| surge_ss | ss / Surge | 74.741 | 4.066 / 4.060 | 4.026 / 3.965 | 0.553 / 0.316 / 0.231 | 0.406 / 0.232 / 0.133 |
| qx_ss | ss / QX | 156.289 | 3.601 / 3.605 | 3.638 / 3.633 | 0.663 / 0.299 / 0.189 | 0.582 / 0.214 / 0.116 |
| clash_json | ss / Clash JSON | 55.179 | 6.710 / 43.815 | 6.330 / 44.493 | 0.341 / 0.409 / 0.189 | 0.249 / 0.308 / 0.140 |
| clash_yaml | ss / Clash YAML | 119.187 | 9.829 / 47.177 | 9.606 / 48.488 | 0.672 / 0.415 / 0.311 | 0.459 / 0.309 / 0.138 |
| clash_zerotier | zerotier / Clash JSON | 49.197 | 5.512 / 42.682 | 5.268 / 43.478 | 0.348 / 0.393 / 0.231 | 0.343 / 0.252 / 0.132 |
| clash_shadowquic | shadowquic / Clash JSON | 53.732 | 6.199 / 44.880 | 5.918 / 45.394 | 0.454 / 0.418 / 0.213 | 0.243 / 0.297 / 0.148 |
| clash_gost_relay | gost-relay / Clash JSON | 52.968 | 6.209 / 43.524 | 5.821 / 44.706 | 0.355 / 0.398 / 0.220 | 0.245 / 0.258 / 0.121 |
| clash_openvpn | openvpn / Clash JSON | 54.146 | 6.504 / 45.171 | 6.271 / 45.265 | 0.456 / 0.422 / 0.247 | 0.257 / 0.247 / 0.136 |
| clash_tailscale | tailscale / Clash JSON | 52.888 | 6.138 / 43.622 | 5.767 / 44.297 | 0.366 / 0.362 / 0.233 | 0.362 / 0.300 / 0.142 |
| clash_easytier | easytier / Clash JSON | 57.137 | 7.441 / 45.042 | 7.082 / 47.214 | 0.343 / 0.429 / 0.240 | 0.352 / 0.248 / 0.147 |
| clash_trusttunnel | trusttunnel / Clash JSON | 54.788 | 6.653 / 43.935 | 6.304 / 44.165 | 0.450 / 0.413 / 0.242 | 0.234 / 0.251 / 0.132 |
| clash_h2_connect | h2-connect / Clash JSON | 53.965 | 6.479 / 43.769 | 6.325 / 44.848 | 0.311 / 0.327 / 0.185 | 0.229 / 0.252 / 0.118 |
| clash_naive | naive / Clash JSON | 50.970 | 6.045 / 43.354 | 5.830 / 44.028 | 0.323 / 0.383 / 0.260 | 0.220 / 0.266 / 0.142 |
| clash_anytls | anytls / Clash JSON | 50.547 | 5.930 / 43.054 | 5.677 / 44.209 | 0.316 / 0.414 / 0.243 | 0.320 / 0.323 / 0.137 |
| clash_mieru | mieru / Clash JSON | 52.498 | 6.314 / 43.391 | 6.109 / 44.595 | 0.446 / 0.436 / 0.248 | 0.343 / 0.248 / 0.141 |
| clash_masque | masque / Clash JSON | 53.616 | 6.310 / 43.254 | 6.142 / 44.078 | 0.327 / 0.341 / 0.207 | 0.221 / 0.270 / 0.136 |
| clash_masque_surge | masque-surge / Clash JSON | 54.762 | 6.429 / 43.502 | 6.187 / 44.221 | 0.323 / 0.413 / 0.249 | 0.325 / 0.321 / 0.139 |
| clash_sudoku | sudoku / Clash JSON | 48.178 | 5.370 / 42.522 | 5.123 / 43.331 | 0.305 / 0.321 / 0.243 | 0.329 / 0.261 / 0.132 |
| clash_juicity | juicity / Clash JSON | 56.738 | 6.669 / 43.917 | 6.334 / 44.459 | 0.430 / 0.398 / 0.245 | 0.221 / 0.321 / 0.184 |
| clash_ssr | ssr / Clash JSON | 57.974 | 6.909 / 44.012 | 6.547 / 45.583 | 0.333 / 0.431 / 0.198 | 0.330 / 0.348 / 0.194 |
| clash_vmess | vmess / Clash JSON | 68.033 | 11.179 / 48.929 | 10.566 / 48.659 | 0.473 / 0.445 / 0.325 | 0.341 / 0.327 / 0.166 |
| clash_socks5 | socks5 / Clash JSON | 54.241 | 6.760 / 44.443 | 6.438 / 45.301 | 0.308 / 0.414 / 0.299 | 0.320 / 0.298 / 0.184 |
| clash_http | http / Clash JSON | 52.349 | 6.376 / 43.569 | 6.108 / 43.946 | 0.428 / 0.360 / 0.199 | 0.308 / 0.299 / 0.130 |
| clash_snell | snell / Clash JSON | 49.522 | 5.820 / 43.466 | 5.523 / 43.620 | 0.319 / 0.422 / 0.236 | 0.220 / 0.295 / 0.139 |
| clash_trojan | trojan / Clash JSON | 50.525 | 6.288 / 43.854 | 6.011 / 43.853 | 0.447 / 0.351 / 0.313 | 0.314 / 0.239 / 0.137 |
| clash_tuic | tuic / Clash JSON | 55.057 | 7.150 / 43.958 | 6.575 / 44.446 | 0.437 / 0.356 / 0.233 | 0.323 / 0.248 / 0.132 |
| clash_vless | vless / Clash JSON | 65.881 | 10.386 / 48.209 | 9.597 / 47.835 | 0.326 / 0.433 / 0.221 | 0.228 / 0.314 / 0.129 |
| clash_hysteria | hysteria / Clash JSON | 55.421 | 7.112 / 44.779 | 6.948 / 45.620 | 0.301 / 0.391 / 0.188 | 0.323 / 0.241 / 0.150 |
| clash_hysteria2 | hysteria2 / Clash JSON | 57.574 | 7.033 / 46.161 | 6.868 / 46.769 | 0.312 / 0.343 / 0.309 | 0.330 / 0.247 / 0.144 |
| clash_wireguard | wireguard / Clash JSON | 66.988 | 8.813 / 46.431 | 8.178 / 46.320 | 0.345 / 0.446 / 0.279 | 0.343 / 0.262 / 0.145 |
| clash_ssh | ssh / Clash JSON | 49.923 | 5.762 / 42.873 | 5.455 / 43.810 | 0.316 / 0.418 / 0.240 | 0.209 / 0.309 / 0.141 |
| clash_direct | direct / Clash JSON | 39.216 | 3.615 / 40.418 | 3.460 / 41.353 | 0.280 / 0.403 / 0.185 | 0.302 / 0.287 / 0.126 |
| surge_external | external / Surge | 90.994 | 3.737 / 3.746 | 3.751 / 3.742 | 0.542 / 0.388 / 0.247 | 0.316 / 0.203 / 0.194 |
| ssr_uri | ssr / URI | 12.053 | 2.721 / 2.714 | 2.846 / 2.854 | 0.246 / 0.305 / 0.186 | 0.245 / 0.242 / 0.131 |
| tuic_uri | tuic / URI | 19.125 | 2.254 / 2.268 | 2.304 / 2.346 | 0.264 / 0.310 / 0.187 | 0.158 / 0.251 / 0.117 |
| wireguard_uri | wireguard / URI | 22.231 | 2.720 / 2.720 | 2.723 / 2.731 | 0.279 / 0.414 / 0.274 | 0.179 / 0.248 / 0.157 |
| hysteria_uri | hysteria / URI | 24.193 | 2.168 / 2.161 | 2.191 / 2.211 | 0.275 / 0.361 / 0.246 | 0.172 / 0.247 / 0.139 |
| anytls_uri | anytls / URI | 34.419 | 3.920 / 3.935 | 3.866 / 3.909 | 0.442 / 0.322 / 0.306 | 0.194 / 0.311 / 0.123 |
| http_uri | http / URI | 1.708 | 1.632 / 1.637 | 1.691 / 1.635 | 0.212 / 0.293 / 0.185 | 0.141 / 0.199 / 0.127 |
| https_uri | http / URI | 1.898 | 1.808 / 1.819 | 1.794 / 1.812 | 0.342 / 0.355 / 0.289 | 0.119 / 0.205 / 0.114 |
| socks5_uri | socks5 / URI | 1.783 | 1.675 / 1.688 | 1.644 / 1.695 | 0.231 / 0.311 / 0.190 | 0.206 / 0.224 / 0.126 |
| socks5_tls_uri | socks5 / URI | 1.913 | 1.839 / 1.826 | 1.795 / 1.822 | 0.334 / 0.345 / 0.237 | 0.106 / 0.230 / 0.110 |
| socks_uri | socks5 / URI | 4.941 | 1.550 / 1.564 | 1.587 / 1.590 | 0.348 / 0.356 / 0.221 | 0.129 / 0.274 / 0.123 |

P95 uses the same Node / previous Rust / candidate order (ms).

| Case | New HTTP P95 N/P/R | Keep-alive P95 N/P/R |
|---|---:|---:|
| ss_uri | 0.556 / 0.363 / 0.262 | 0.476 / 0.309 / 0.140 |
| vmess_ws | 0.702 / 0.448 / 0.294 | 0.966 / 1.045 / 0.190 |
| vless_ws | 0.507 / 0.440 / 0.287 | 0.347 / 0.322 / 0.223 |
| trojan | 0.448 / 0.380 / 0.309 | 0.331 / 0.288 / 0.191 |
| hysteria2 | 0.474 / 0.397 / 0.313 | 0.374 / 0.267 / 0.166 |
| surge_ss | 0.684 / 0.405 / 0.347 | 0.543 / 0.316 / 0.212 |
| qx_ss | 0.797 / 0.388 / 0.250 | 0.688 / 0.299 / 0.164 |
| clash_json | 0.536 / 0.495 / 0.314 | 0.396 / 0.391 / 0.220 |
| clash_yaml | 0.869 / 0.514 / 0.339 | 0.666 / 0.412 / 0.168 |
| clash_zerotier | 0.487 / 0.474 / 0.263 | 0.403 / 0.322 / 0.169 |
| clash_shadowquic | 0.483 / 0.474 / 0.330 | 0.400 / 0.381 / 0.203 |
| clash_gost_relay | 0.487 / 0.480 / 0.281 | 0.331 / 0.320 / 0.145 |
| clash_openvpn | 0.495 / 0.483 / 0.283 | 0.308 / 0.303 / 0.163 |
| clash_tailscale | 0.517 / 0.465 / 0.275 | 0.410 / 0.373 / 0.178 |
| clash_easytier | 0.503 / 0.491 / 0.280 | 0.387 / 0.296 / 0.177 |
| clash_trusttunnel | 0.493 / 0.466 / 0.299 | 0.361 / 0.420 / 0.167 |
| clash_h2_connect | 0.375 / 0.376 / 0.214 | 0.351 / 0.356 / 0.146 |
| clash_naive | 0.414 / 0.482 / 0.290 | 0.263 / 0.339 / 0.169 |
| clash_anytls | 0.405 / 0.495 / 0.274 | 0.358 / 0.392 / 0.173 |
| clash_mieru | 0.479 / 0.497 / 0.278 | 0.380 / 0.298 / 0.170 |
| clash_masque | 0.378 / 0.402 / 0.280 | 0.264 / 0.371 / 0.168 |
| clash_masque_surge | 0.469 / 0.494 / 0.278 | 0.380 / 0.397 / 0.177 |
| clash_sudoku | 0.351 / 0.377 / 0.269 | 0.528 / 0.357 / 0.159 |
| clash_juicity | 0.527 / 0.488 / 0.313 | 0.366 / 0.411 / 0.215 |
| clash_ssr | 0.465 / 0.531 / 0.331 | 0.370 / 0.396 / 0.223 |
| clash_vmess | 0.553 / 0.546 / 0.356 | 0.381 / 0.411 / 0.230 |
| clash_socks5 | 0.463 / 0.476 / 0.324 | 0.342 / 0.354 / 0.210 |
| clash_http | 0.486 / 0.462 / 0.320 | 0.341 / 0.395 / 0.152 |
| clash_snell | 0.462 / 0.479 / 0.267 | 0.339 / 0.352 / 0.206 |
| clash_trojan | 0.485 / 0.397 / 0.347 | 0.353 / 0.285 / 0.196 |
| clash_tuic | 0.466 / 0.495 / 0.262 | 0.350 / 0.370 / 0.155 |
| clash_vless | 0.469 / 0.522 / 0.309 | 0.290 / 0.377 / 0.161 |
| clash_hysteria | 0.338 / 0.458 / 0.225 | 0.345 / 0.320 / 0.215 |
| clash_hysteria2 | 0.447 / 0.408 / 0.341 | 0.951 / 0.535 / 0.705 |
| clash_wireguard | 0.499 / 0.503 / 0.318 | 0.379 / 0.344 / 0.175 |
| clash_ssh | 0.497 / 0.516 / 0.281 | 0.399 / 0.428 / 0.180 |
| clash_direct | 0.374 / 0.486 / 0.260 | 0.339 / 0.362 / 0.159 |
| surge_external | 0.579 / 0.475 / 0.286 | 0.853 / 0.870 / 0.626 |
| ssr_uri | 0.397 / 0.399 / 0.220 | 0.296 / 0.312 / 0.207 |
| tuic_uri | 0.304 / 0.400 / 0.227 | 0.221 / 0.313 / 0.146 |
| wireguard_uri | 0.427 / 0.492 / 0.311 | 0.300 / 0.341 / 0.184 |
| hysteria_uri | 0.420 / 0.456 / 0.278 | 0.289 / 0.359 / 0.205 |
| anytls_uri | 0.473 / 0.418 / 0.340 | 0.271 / 0.368 / 0.160 |
| http_uri | 0.310 / 0.347 / 0.242 | 0.247 / 0.262 / 0.202 |
| https_uri | 0.372 / 0.421 / 0.318 | 0.232 / 0.290 / 0.140 |
| socks5_uri | 0.375 / 0.426 / 0.257 | 0.238 / 0.300 / 0.164 |
| socks5_tls_uri | 0.380 / 0.401 / 0.304 | 0.212 / 0.288 / 0.147 |
| socks_uri | 0.376 / 0.425 / 0.294 | 0.253 / 0.328 / 0.149 |

The pooled keep-alive P95 for Clash Hysteria2 is a reviewed exception:
0.705 ms versus previous Rust 0.535 ms (Node 0.951 ms). Its P50 improves
from 0.247 to 0.144 ms. This run does not establish the cause or persistence
of the tail increase; it is retained as an unresolved timing limitation,
not described as a P95 improvement. Other 47 keep-alive P95 rows and all
48 new-connection P95 rows are below previous Rust. No timing assertion was relaxed.

First conversion after server readiness, one observation per backend; not a cold-start distribution.
| Backend | Case | First HTTP ms |
|---|---|---:|
| node | ss_uri | 5.646 |
| rust_baseline | ss_uri | 1.335 |
| rust | ss_uri | 1.230 |

Separate load cells: 384 changing requests over all 48 fixtures per block, three blocks.
New HTTP connections; load/allocator/worker churn is included. RSS is retained process memory, not a JS heap leak measurement.
| Backend | Clients | Block | P50 ms | P95 ms | Requests/s | CPU ms/request | End RSS MiB | Settled RSS MiB |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| node | 1 | 0 | 0.462 | 0.615 | 2087.3 | 0.312 | 252.56 | 252.56 |
| rust_baseline | 1 | 0 | 0.378 | 0.519 | 2397.3 | 0.260 | 13.66 | 13.66 |
| rust | 1 | 0 | 0.231 | 0.267 | 3931.5 | 0.130 | 13.05 | 13.05 |
| rust | 1 | 1 | 0.236 | 0.304 | 3773.1 | 0.130 | 13.05 | 13.05 |
| rust_baseline | 1 | 1 | 0.415 | 0.458 | 2296.0 | 0.312 | 13.66 | 13.66 |
| node | 1 | 1 | 0.344 | 0.525 | 2542.4 | 0.286 | 252.94 | 252.94 |
| node | 1 | 2 | 0.448 | 0.606 | 2104.3 | 0.313 | 254.33 | 254.33 |
| rust_baseline | 1 | 2 | 0.391 | 0.515 | 2346.1 | 0.313 | 13.68 | 13.68 |
| rust | 1 | 2 | 0.263 | 0.302 | 3457.4 | 0.130 | 13.05 | 13.05 |
| node | 8 | 0 | 2.245 | 3.130 | 3335.1 | 0.286 | 254.74 | 254.74 |
| rust_baseline | 8 | 0 | 2.358 | 4.174 | 2952.0 | 0.365 | 16.37 | 16.37 |
| rust | 8 | 0 | 1.973 | 3.249 | 3791.8 | 0.208 | 15.13 | 15.13 |
| rust | 8 | 1 | 2.081 | 3.564 | 3593.2 | 0.234 | 15.23 | 15.23 |
| rust_baseline | 8 | 1 | 2.388 | 4.095 | 3094.4 | 0.391 | 16.47 | 16.47 |
| node | 8 | 1 | 2.128 | 2.731 | 3655.9 | 0.286 | 255.15 | 255.15 |
| node | 8 | 2 | 2.184 | 2.970 | 3480.5 | 0.286 | 256.58 | 256.58 |
| rust_baseline | 8 | 2 | 2.769 | 8.311 | 2276.8 | 0.443 | 16.55 | 16.55 |
| rust | 8 | 2 | 2.147 | 3.491 | 3479.6 | 0.234 | 16.18 | 16.18 |

Across the separate changing-input load cells, median requests/s are
3773.1 / 2346.1 / 2104.3 for candidate / previous Rust / Node with one client,
and 3593.2 / 2952.0 / 3480.5 with eight clients. With eight clients the median
server CPU is 0.234 / 0.391 / 0.286 ms per request. These cells include the
Python HTTP driver and worker/allocator churn; they are not single-client
latency samples or a claim that server capacity is saturated. CPU counters
have OS sampling precision.

Candidate settled RSS stays 13.05 MiB through the three one-client load blocks
and reaches 15.13, 15.23, 16.18 MiB in the eight-client blocks. Previous Rust
reaches 16.55 MiB and Node 256.58 MiB. The finite RSS increase under concurrency
does not identify a JS leak or establish long-uptime memory stability. The
focused test separately bounds outer-engine post-GC growth to 64 KiB after
2048 changing conversions; parser source-text cleanup and idle-count bounds
are checked independently.

Separate profiler-on results prove 336/336 measured requests reused and retained
an environment, with zero repeated application/bootstrap or generic-parser setup
and zero environment destruction in those warm cells. Each request still records
one parse, native parse and producer call. The two warmup requests per fixture
are separate; the first conversion records an actual miss. Its instrumented
runtime/context/bootstrap costs are 60.020 / 138.859 / 398.008 us; this single
cold observation is not a startup distribution. Profiled times never enter the
ordinary latency tables and are elapsed time, not OS CPU.

| Case | Queue us | Worker lifetime us | Bootstrap us | Native parse us | Produce us | Teardown us | Response ready us | Client HTTP us |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| ss_uri | 10.567 | 76.235 | 0.000 | 7.982 | 9.875 | 0.000 | 155.083 | 311.779 |
| surge_ss | 10.086 | 73.641 | 0.000 | 14.532 | 9.625 | 0.000 | 155.865 | 308.253 |
| qx_ss | 10.146 | 69.655 | 0.000 | 13.070 | 9.454 | 0.000 | 151.467 | 296.556 |
| clash_json | 9.826 | 74.303 | 0.000 | 14.061 | 9.164 | 0.000 | 161.653 | 310.487 |
| clash_wireguard | 10.056 | 105.339 | 0.000 | 17.827 | 23.625 | 0.000 | 203.445 | 351.568 |
| http_uri | 9.084 | 62.124 | 0.000 | 4.176 | 8.102 | 0.000 | 139.680 | 274.423 |

Worker lifetime includes queueing and cleanup; the pure route returns after that
cleanup/retention decision. Response-ready time ends before socket transmission.
Intervals overlap and must not be added or subtracted as independent CPU costs.
Projection/decode remains present in the common value bridge: for SS/Surge/QX,
profiled project/decode active times are approximately 9/4, 5/3, 5/3 us.
This change does not claim to remove every JS representation or bridge operation.

Formatting, Clippy, 48 ordinary release Rust tests (four ignored drivers),
28 performance self-checks, 66 parser preflight responses, 60,480 timed HTTP
responses and the separate 6,912 load responses pass. Correctness is checked
for every measured response. The 336 warm profile pairs and first-use/warmup
pairs also pass strict lifecycle/privacy/completion checks. All compilation,
formatting, tests and measurements ran in Actions. Only Git/GitHub metadata and
plaintext logs were inspected locally; no test binary or artifact was downloaded.
This task does not repeat the private subscription long test or claim release readiness.

[Compatibility run 39774769529](https://github.com/DSLZL/Sub-Store/actions/runs/39774769529)
also passes at the exact same candidate source: all six jobs succeed (native
Linux x64/arm64, native Windows x64, Docker x64/arm64 and unchanged frontend).
The native jobs pass 43 harness self-checks, 48 binary unit tests plus three
library tests, the delivered-package HTTP/state/restart/rollback suite, JS probe
and all 826 Node reference unit tests. Existing timeouts remain unchanged.
The supplied-subscription long test, macOS/Android and long-uptime memory behavior
are outside this acceptance; a pass does not establish the cause of the earlier
unreproduced Windows sharing timeout.

## 2026-10-22: supplied-subscription long run and Hy2 tail diagnosis

[Long run 39781070691](https://github.com/DSLZL/Sub-Store/actions/runs/39781070691)
passes all 35 same-runner pairs and 70 cells. The aggregation rejects missing
pairs and unequal controlled output/work. It includes 812 timed controlled requests
and two separately reported real-detector requests, excluding warmups/imports.
[Runner-side report inspection 39782788702](https://github.com/DSLZL/Sub-Store/actions/runs/39782788702)
reads the immutable sanitized metrics without repeating preparation.

Candidate Rust is the accepted `af08b0649c4d2f2814fe38ce6943b1397f5e1c14`,
original Node remains `9cc6b5fac966993364ae33da8797f446bbf7f571`,
and the unmodified frontend remains `4bd7b0acbe65466b933ccea14fc61dbebe036ead`.
The driver dispatch source is `56132f012ad345866b899dd1259490c4cdae29b5`.
Candidate binary SHA-256 is
`3b8bf15faded89b92ee274427a117e0b17f271f14a51dc5a5ac5c8bc229236bd`.
The workflow and driver's old Rust pin were updated together; no product code
changed in this follow-up.

The unchanged supplied file is imported through the official frontend file picker
five times per backend, with reload/editor/restart validation. Sources are fetched
once through Node before preprocessing, then authenticated encrypted inputs and
UI-created seeds are shared to independent runners. Of 22 remote source slots,
21 are nonempty and one is empty/failed; this observation is preserved equally
on both backends, alongside the local source. Per-runner replay checks all 23
ordered source texts. The existing five-site stress script removes maintenance
batch caps while retaining filters, processing, probe retries/timeouts and
publication. It adds no conversion-result cache or processor bypass.

Each block runs Node and Rust on the same runner, alternating backend order.
The following medians pool requests; the ratio is Node median / Rust median,
which differs from the paired block geometric estimate in metrics.json.
The `pre_health` group has one block; cold has ten one-request blocks, so no
cold P95 is claimed.

| Case | Clients | Requests Node / Rust | Node P50 s | Rust P50 s | P50 ratio | Node / Rust CPU s per request | Node / Rust peak RSS MiB |
|---|---:|---:|---:|---:|---:|---:|---:|
| parse | 1 | 60 / 60 | 3.944 | 0.874 | 4.51 | 5.641 / 0.970 | 634.8 / 154.6 |
| pre_health | 1 | 20 / 20 | 4.893 | 1.456 | 3.36 | 6.786 / 1.543 | 641.9 / 147.3 |
| cold | 1 | 10 / 10 | 9.817 | 8.016 | 1.22 | 9.818 / 5.048 | 677.2 / 288.7 |
| warm | 1 | 100 / 100 | 5.192 | 2.324 | 2.23 | 6.870 / 2.234 | 1215.9 / 309.3 |
| no_publisher | 1 | 60 / 60 | 4.218 | 1.513 | 2.79 | 6.454 / 1.662 | 1224.3 / 243.1 |
| pressure | 1 | 13 / 29 | 5.124 | 2.282 | 2.25 | 6.336 / 2.133 | 1120.0 / 422.8 |
| pressure | 2 | 16 / 54 | 8.254 | 2.541 | 3.25 | 6.264 / 2.096 | 1222.0 / 648.1 |
| pressure | 4 | 22 / 64 | 17.196 | 4.364 | 3.94 | 6.652 / 3.484 | 1116.4 / 784.7 |
| pressure | 8 | 34 / 80 | 27.965 | 8.743 | 3.20 | 6.246 / 3.431 | 1264.5 / 1336.8 |

Controlled parse output is 5,960 nodes; processed/full output is 2,986 nodes
on both backends. Full warm P95 is 5.456 s for Node and 2.409 s for Rust.
At eight pressure clients, throughput is 0.218 versus 1.006 requests/s
(Rust / Node = 4.62), while peak Rust RSS is 1,336.8 versus 1,264.5 MiB:
5.7% higher despite lower CPU/request. Retained RSS after settling is
1,103.6 versus 1,222.5 MiB. This finite run does not establish a long-uptime
memory bound; the concurrent peak remains an optimization target.

The separate real HTTP-META/Mihomo smoke pair returns 1,021 and 1,159 nodes
for Node and Rust. Those live probe outcomes differ and each backend has only
one observation, so their 18.986 s and 16.694 s are not a valid speedup claim.
No controlled estimates include that pair. Remote source contents also differ
from older runs; comparisons with historical absolute times are invalid.

### Hy2: retained old evidence and enlarged repeat

[Inspection 39782204253](https://github.com/DSLZL/Sub-Store/actions/runs/39782204253)
reads the original 39774769592 artifact on an Actions runner. Its 210-request
Clash Hy2 keep-alive P95 increase (0.535 -> 0.705 ms) is real in that sample,
but concentrated in local batches. All original samples remain included:

| Block (one based) | Node P50 / P95 ms | Previous Rust P50 / P95 ms | Candidate P50 / P95 ms |
|---:|---:|---:|---:|
| 1 | 0.334 / 0.374 | 0.248 / 0.286 | 0.189 / 0.224 |
| 2 | 0.225 / 0.394 | 0.254 / 0.389 | 0.189 / 0.214 |
| 3 | 0.332 / 0.717 | 0.243 / 0.543 | 0.539 / 1.042 |
| 4 | 0.788 / 1.169 | 0.272 / 1.306 | 0.144 / 0.670 |
| 5 | 0.346 / 0.956 | 0.241 / 0.281 | 0.123 / 0.178 |
| 6 | 0.326 / 0.368 | 0.241 / 0.321 | 0.121 / 0.139 |
| 7 | 0.326 / 0.357 | 0.241 / 0.301 | 0.123 / 0.191 |

Candidate block 3 is broadly slower, rather than a single slow node parse:
its median is 0.539 ms, compared with 0.121-0.189 ms in its other blocks.
Node and previous Rust also have nearby slow batches. This supports a local
measurement/scheduling disturbance as an explanation, but those timed
requests had no corresponding profiles; the exact historical trigger cannot
be assigned to OS scheduling, GC, or an environment miss from these data.

[Focused repeat 39781818295](https://github.com/DSLZL/Sub-Store/actions/runs/39781818295)
uses three fresh server rounds, seven balanced 200-request blocks per round
and alternating equivalent original/newline inputs. It measures both connection
modes for Clash Hy2, Hy2 URI and a Clash SS control: 75,600 timed responses,
all checked against Node, plus 3,000 separate profile-on responses.
The previous Rust source remains `9c19111bae2af2654caa339151e4eabd12b586bb`.
Driver/candidate checkout is `666938d60d60d4e8837bc230ef8487e9d77b0151`;
the exact candidate and previous binary hashes match the old accepted run.
Only the workload/sampling changes, so no product fix is credited for the
changed distribution.

| Case | Connection | Requests per backend | Node P50 / P95 ms | Previous Rust P50 / P95 ms | Candidate P50 / P95 ms |
|---|---|---:|---:|---:|---:|
| clash_hysteria2 | keep_alive | 4,200 | 0.233 / 0.360 | 0.292 / 0.335 | 0.146 / 0.197 |
| clash_hysteria2 | new_connection | 4,200 | 0.442 / 0.501 | 0.346 / 0.446 | 0.250 / 0.318 |
| hysteria2 | keep_alive | 4,200 | 0.245 / 0.366 | 0.221 / 0.273 | 0.139 / 0.192 |
| hysteria2 | new_connection | 4,200 | 0.418 / 0.554 | 0.305 / 0.434 | 0.244 / 0.313 |
| clash_json | keep_alive | 4,200 | 0.243 / 0.378 | 0.249 / 0.311 | 0.143 / 0.205 |
| clash_json | new_connection | 4,200 | 0.379 / 0.508 | 0.340 / 0.430 | 0.231 / 0.270 |

Clash Hy2 keep-alive P95 is 41.3% lower than previous Rust and 45.2% lower
than Node in the enlarged repeat. This is not zero jitter: candidate round 2,
block 1 still has 0.885 ms P95; its overall maximum is 1.243 ms versus
previous Rust's 1.117 ms and Node's 4.370 ms. No outliers are removed.
The regression in the original pooled P95 is not a sustained Hy2 parsing
slowdown in the repeat, and isolated latency spikes remain observable.

For the separate 1,000 Clash Hy2 keep-alive profile pairs, all environments
are reused/retained, with zero misses/teardowns. P95 values are 10.986 us
queue, 16.524 us native parse, 9.875 us production, 81.076 us worker lifetime,
169.884 us server response ready and 269.903 us client request. The slowest
paired client request is 361.710 us: its worker is 94.756 us, queue 12.553 us
and native parse 19.940 us. These profiles include instrumentation and report
writing, and do not enter the timed table. They do not reproduce or explain
the exact historical 0.705 ms P95 spike.

Current runner self-checks pass all 31 tests, including nested artifact discovery,
pooled percentiles and exact slow-request/profile pairing. The focused run also
passes formatting, Clippy and all 48 ordinary release Rust tests (four ignored
drivers). An initial inspection failed because the artifact contains nested
result/log directories; `1eaea49f516bcceb39cec0af381780553c486120` fixes
discovery and rejects ambiguous metrics. The failed inspection and original
timing artifacts are retained. Product/frontend code and the accepted binary
remain unchanged, so native/frontend/Docker compatibility is not rerun here.
All builds, tests, artifact extraction and measurements execute in Actions;
local inspection uses only Git/GitHub metadata and sanitized plaintext logs.
