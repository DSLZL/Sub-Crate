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

The workflow freezes original `9cc6b5fa`, Rust `e5890c2a`, and unchanged frontend
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
