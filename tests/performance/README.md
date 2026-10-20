# Supplied-subscription comparison

Run **Supplied subscription performance** (`rust-performance.yml`) manually on
`codex/rust-server-parity`. All builds, self-checks, browsers, services and
measurements run in GitHub Actions; the harness refuses local execution.

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

The workflow freezes original `9cc6b5fa`, Rust `9faaa86d`, and unchanged frontend
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
