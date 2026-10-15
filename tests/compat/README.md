# Rust rewrite compatibility baseline (M1, in progress)

This directory tests the Node reference with approved RF01/RF02 fixes, builds development-only JS helpers,
and captures native rquickjs feasibility evidence. It does **not** contain a
replacement Rust server or demonstrate complete frontend/backend parity.

- Backend base: `9cc6b5fac966993364ae33da8797f446bbf7f571` (2.42.4), plus the exact `approved-reference-fixes.patch`.
- Unchanged frontend: `4bd7b0acbe65466b933ccea14fc61dbebe036ead` (2.34.0).
- Development oracle: Node 24.15.0, pnpm 11.0.9, locked backend dependencies.
- Contained Rust experiment: Rust 1.98.0, rquickjs 0.14.0, locked Cargo dependencies.

Run from the repository root, after `pnpm --dir backend install --frozen-lockfile`
and `pnpm --dir tests/compat install --frozen-lockfile`.
Replace `/absolute/path/to/node` with the pinned Node executable; the harness
checks its exact version. On Windows use native paths and Python 3.

```sh
python -m unittest discover -s tests/compat -p test_runner.py
python tests/compat/reference.py --node /absolute/path/to/node --mode unit
python tests/compat/reference.py --node /absolute/path/to/node --mode contracts
python tests/compat/reference.py --node /absolute/path/to/node --mode helpers
python tests/compat/probe.py --node /absolute/path/to/node
```

The contained host bundles the frozen age-encryption 0.3.0 implementation and
actual age.js wrappers. Test-only dependencies web-streams-polyfill 4.3.0 and
@kayahr/text-encoding 2.2.0 supply streams and UTF-8 encoding; Rust getrandom
0.4.3 supplies OS-backed crypto.getRandomValues with a native 65536-byte quota.
The source's explicit unsupported-X25519 fallback selects the existing noble
implementation; this does not supply general crypto.subtle. Response currently
implements only the body consumption used by age. Full Fetch/encoding/Web API
conformance remains HG17, and application age routes still need Rust integration.

AG01-AG11 compare actual Node exports with the contained host: encoding view
offsets/fatal/stream/BOM, stream backpressure/cancel/errors, one-shot body reads,
random integer views/quota, both X25519 and MLKEM768-X25519 generation/derivation,
Unicode/binary/empty/chunk-boundary armor, validation/authentication/recovery,
redaction and actual Script Operator injection. Four fresh processes exchange
32 ciphertexts/plaintexts in both directions, including Node encrypting to
Rust-generated keys. Each validates exact plaintext and independent byte
length/hash. Raw traces, fixture identities/recipients, ciphertexts, input/output
SHA256 and command deadlines are retained and cross-checked; disposable private
keys stay in owned age-*-state files and must never appear in console logs.
They are test keys, never production credentials. Random outputs are verified
by decryption rather than normalized into an equality check.

The debug profile optimizes only rquickjs-sys at level 3 so the interpreter can
run the complete crypto slice under the unchanged 15-second/64-MiB host limit.
The unoptimized-engine timeout remains failed evidence. Docker uses its release
build. Helper bundling also collects exact direct/transitive license texts into
script-helper-licenses.txt, copies them into Docker, and records their hash.

The pinned backend's Babel alias plugin cannot load Windows filenames. Use
`--mode unit-windows` for a separately labeled run with a development-only
loader that retains the Babel preset and resolves aliases as native paths.
The original loader failure remains evidence; neither loader changes backend
source. The working backend now recognizes native absolute paths; runtime-manifest
fixtures use Windows directory junctions without requiring symlink privilege.
CI, inventory and orchestration reject any backend delta outside the checked-in
approved patch. Reports record the actual checkout revision, frozen base, patch
hash and (for orchestration) whether backend files differ from HEAD. The runner's
`backend_base_commit` names fixture provenance, not an independently verified
target revision. Original failed runs and the frozen Git commit remain intact.
Unit runs synchronously record each test start/end in `mocha.stderr.log`, so a
native process crash retains its active test even without Mocha's final JSON.

`reference.py` gives each execution fresh state and starts its own actual Node
listener. It does not read the user's production state. Short commands use RTK
where installed; native CI without RTK records that exception. The listener is
owned directly because terminating the RTK parent leaves Node alive on Windows.

The contract runner also accepts another explicit listener:

```sh
python tests/compat/runner.py --url http://127.0.0.1:3000 --kind rust-contract \
  --state-dir /path/to/disposable/state --evidence /path/to/new-result.json
```

`smoke` is currently thirteen ordered scenarios. These write/delete fixture data;
use only disposable state. Response status, selected headers, JSON types/raw
content, persisted side effects, and controlled upstream requests are asserted.
A cached flow lookup must make zero additional upstream requests. Missing
fixtures, failed assertions, unavailable backends and `python -O` fail. Existing
result files are never overwritten. `--suite full` intentionally fails while
the inventory and executable fixture set remain incomplete. A scoped contract
success is always labeled `full_coverage: false` and `rust_parity: false`.

Generated archive IDs and timestamps have explicit type/pattern checks. Captured
values must match subsequent detail/restore requests and persisted state. A
capture does not replace assertions on the snapshot, error or transition. Failed
responses retain the actual state and upstream requests in their evidence.

The orchestrated `--mode contracts` also stops its first listener, starts a new
process over the same disposable state, and runs the separate `restart` scenario.
That stage verifies backup-imported excluded script bytes (Unicode and CRLF/LF)
and unknown/falsy fields survive export and restart. It has independent listener
logs and evidence. Running `--suite restart` alone requires the D03 state seed.
It then starts a third listener in merged mode with `/backend` as the admin
prefix and runs two `merged-share` scenarios at the public origin. These verify
token/path binding, GET-only access, count consumption, expiry and share archive
restore. Admin-prefixed access and public-origin access are asserted separately.
Finally, a fresh standalone listener runs `conversion`: 66 scenarios and 117
requests. This includes every one of the 33 output names/aliases through both
the parser API and a download URL, four rule output targets, existing parser/
preprocessor samples, ordered built-in and general-JS processors, collection
preview/download, target/platform/header precedence, missing resources and
forbidden share overrides. `--suite conversion` runs this slice independently.
Another standalone listener runs `resources`: thirteen scenarios and 78 requests
cover age X25519/hybrid key generation and fixed-key derivation, five resource
sorts and four legacy replacements, artifact CRUD and upload-disabled sync,
archive sorting/deletion/restore, and enabled/disabled log capture and filtering.
The key fixtures are disposable test keys. Dynamic keys, log IDs/times and sync
times have typed captures and checks against subsequent responses and state.
Six `operations` scenarios and a separate migrated-data restart cover default
CORS/preflight/HEAD/404 behavior, download and flow cache hits/noCache/refresh,
remote failure policies, Base64/invalid imports and legacy migrations. Three
additional listeners exercise configured CORS, wildcard CORS and branded local
origins with frontend-consumed metadata. Eight `gist` scenarios use a local
controlled provider to verify create/update/delete/restore, placeholder retention,
provider errors, plaintext/Base64 backup payloads, token strategies and no-change
upload reuse. Five `node-info` scenarios cover successful URI/lookup generation,
URI-generation failure, IPv6 normalization, provider errors, HTTP-status/body
precedence and missing input with listener recovery. The orchestrated total is
118 scenarios (392 requests).
`--suite operations` includes its restart; selecting `operations-restart` alone
requires the D06 state seed. `cors-configured`, `cors-wildcard` and `cors-branded`
automatically set their documented fixture environment in `reference.py`.
The SS output samples cover alias dispatch, not every protocol/option combination.
Expected JSON fields and output bytes are fixed fixtures, reviewed against the
frozen source; acceptance never regenerates expectations from its target.

Frozen negative behavior remains explicit: unsupported rules can produce
500/null-length errors in QX/Surge/Loon; a failed function script can fall back
to a shortcut and retain the original nodes, while a top-level throw fails.
Shortcut filters require an explicit return; omitting it selects no nodes.
Missing artifact PATCH returns DUPLICATE_KEY; legacy artifact PUT accepts names
that POST rejects. Invalid archive-sort payload returns the frozen HTML 500 body.
The original no-token artifact restore defect terminated Node 24 without an HTTP
response. The user explicitly requested repairing it and native Windows path
reading in the working backend. RF02 now throws into the existing handler catch:
Y03 requires 500/FAILED_TO_RESTORE_ARTIFACTS, no provider request, unchanged complete
exported data and a successful follow-up on the same listener. RF01 native-file
unit tests require exact Unicode/CRLF content, both Windows path separators,
returnRaw and missing-file errors without HTTP fallback. Later Rust acceptance
must pass the same corrections. Live-provider restore remains an open gate.

Default CORS denies local origins; a custom backend name permits HTTP localhost
and 127.0.0.1 only under the default policy. An explicit whitelist replaces that
policy, even for branded backends. Denied requests must omit allow-origin headers
and leave resource state intact. Downloaded flow headers populate the flow cache;
refresh invalidates both resource and flow caches. Quiet remote failure still
fails an empty subscription, while fallbackQuiet returns empty output. Migration
converts legacy maps only when schemaVersion is falsy: truthy `1.0` updates the
version marker while retaining the old shape. These frozen behaviors are explicit
fixtures; they are not additional approved reference fixes.

Provider fixtures seed only the controlled service, through `upstream_gists`.
`upstream_state` compares its complete stored files; `base64_backup` strictly
decodes the selected uploaded backup and compares all JSON fields/types. These
checks retain actual outbound request evidence and reuse captured times from a
subsequent public response. Plaintext backup replaces the token with its legacy
placeholder; Base64 preserves the token. Both omit the uploaded age secret while
retaining the local key. GET settings queries Gist and writes store status when
artifactStore is absent. An unchanged upload must make no write request. These
are controlled-provider contracts; live-provider acceptance, GitLab, encryption,
write-error recovery and scheduling remain separate checks.

The `node-info` listener uses a disposable HTTP proxy because its reference
lookup URL is fixed at ip-api.com. `reference.py` configures only that child
process, and the runner binds the selected loopback `--upstream-port`. The fixture
answers only the fixed lookup responses and rejects other absolute proxy hosts;
it never forwards requests. Actual absolute-form GET URLs, language queries and
User-Agent are asserted. The old handler discards lookup `status`, omits shareUrl
when URI generation throws, retains an empty URL for unsupported types, and uses
the JSON body's success even under HTTP 503. These are frozen reference behaviors.
An explicit Rust listener must use the same local HTTP proxy and runner port to
execute this suite; a live public IP service is not its acceptance oracle.

The Rust example evaluates unchanged `scripts/ip-flag-node.js` with controlled
HTTP, request headers, memory-backed storage, locked lodash/YAML/Buffer/Base64
helpers, promises/timers, and asynchronous Rust-to-JS callback re-entry. It
checks full/shortcut filters, operators and response transforms, timer arguments
and cancellation, absent Node APIs, and excluded script text in storage. No Node
executable is launched by the Rust probe. Helper generation uses Node only at
development/build time.

The same executable also runs eleven shared `SH01`-`SH11` host-contract groups.
`probe.py --node ...` executes the actual reference `OpenAPI` through the existing
development Babel loader, followed by the Rust host, against the same loopback
upstream and independent disposable JSON state. Both execute the unchanged
`fixtures/script-host-contracts.js`. Each side must pass every required group,
match typed results/persisted state, and send the expected 28 requests with exact
methods, paths, selected headers and body bytes. Separate Node and Rust processes
then reload both files and must retain the complete values without network I/O.
The Node executable is an external development oracle; the Rust binary does not
launch Node or load Node modules.

These groups pin synchronous read/write/delete/persist return values, object
identity, historical root-key slicing, missing/undefined/circular values, all
seven HTTP helpers plus request(), binary views/responses, Unicode/CRLF text,
cookies, explicit Accept omission, decoded URL credentials, async event hooks,
timeout and late-completion behavior, setup/status errors, redirects, intervals
and timer cross-cancellation. The pinned Undici adapter rejects the third followed
response even when it is a final 200; the shared fixture retains that behavior.
The contained Rust transport reuses the already locked Hyper/hyper-util/
hyper-rustls/http-body-util crates because reqwest inserts a default Accept header
even when the caller explicitly omits it. No additional transitive packages were
added. New source and binary hashes accompany each immutable evidence directory.

The executable additionally runs 124 shared `PC01`-`PC124` processor/helper groups.
The Node oracle loads the actual processor module and application OpenAPI from
`backend/`, matching the unit suite's Babel working directory. The helper builder
uses the existing Babel parser to extract the frozen wrapper/cache declarations
verbatim for QuickJS, plus the actual project YAML module. Missing/ambiguous
declarations fail the build. This demonstrates interpreter compatibility of those
JavaScript functions; it does not implement a Rust conversion pipeline or cache.

Both runtimes execute `fixtures/script-processor-contracts.js` unchanged. Checks
cover full/shortcut filter/operator/response wrappers, injected parameters and
truthful per-runtime environments, cloning versus aliases, fallback mutations,
return truthiness, user errors/recovery, file-local options, Mihomo patch/main
branches and the legacy file type. A Rust async callback re-enters pending JS
processors and HTTP, runs concurrent callbacks and recovers from a rejection.
This callback takes fixture payloads; it is not the `produceArtifact` API.

`PC89`-`PC104` exercise the actual `ProxyUtils.parse(raw)` and
`produce(proxies, targetPlatform, type, opts = {})` wrappers with their complete
frozen preprocessor/parser/producer registries. A test-only app binding forwards
source diagnostics to the existing Rust host; the production OpenAPI module and
Node capabilities are not bundled. Assertions cover mixed lines/PEG grammars,
HTML/Base64/SSD/full-config/Clash preprocessing, URI families, leading-zero
Reality short IDs, transport/port/password/unknown-field normalization,
certificate fingerprints, all 33 exact aliases in fourteen producer families,
internal/external/default output, target filtering/override, mutation/identity,
empty/error/recovery behavior and actual async Script Operator use while a Rust
callback is pending. V2Ray emits Base64 text even for `internal`; JSON internal
retains object identity and producer input mutations. Raw text and typed fields
are compared without a field-dropping normalizer. `conversionHelper` persists
exact exported text plus zero/false through independent process reloads.

The source fingerprint helper reuses locked jsrsasign; its missing npm license
is supplied from the exact upstream version under `fixtures/licenses/`. No new
dependency is installed. Every required PC group must also have an observation,
even when both runtimes omit the same result. Required progress markers remain
ordered; JSON object field order is not an acceptance condition. This is M1
source-helper feasibility in a Rust process. Native Rust conversion, CA file
integration, full format/option coverage and production integration remain pending.

`PC105`-`PC124` execute the actual `produceArtifact` source generator, complete
17-entry processor registry, process/processResponse wrappers and script loader.
The Node oracle uses the actual exports and sets the existing script-enabled
backend-path configuration to `/`. Rust source extraction removes only module
imports/exports and binds the same dependencies to native HTTP/filesystem/timers.
Both retain their truthful public script environment; Rust exposes no Node API.
Assertions cover local/remote subscriptions, raw envelopes, provenance/options,
context-controlled/disabled operators, module/generated-file/remote script links,
fragment arguments, Clash preprocessing and cache eligibility, ordered/tagged
collections, UA propagation, fallback/duplicate notification policy, ordinary/
legacy Mihomo files and recursive Add Proxies From Subscription. Real nested
Script Operators await HTTP/timers and generate another artifact; active native
callback reentry also executes the real generator. `artifactHelper` preserves
exact CRLF/Unicode text, zero and false through independent process reloads.
Fourteen independently checked artifact requests increase the processor total
to 116 per runtime. All 248 ordered progress events, full typed observations,
source caches, wire and stored state are required. Mutation checks reject wrong
headers/order/body/method/count, changed text/falsy types and unpersisted caches.
Notification callbacks are fixture-captured; external delivery remains pending.
The source rule branch's missing RuleUtils.process still throws native TypeError;
its engine-specific diagnostic wording is not normalized or treated as success.
This proves the source helper chain; a production Rust service, native Rust
pipeline/store, full processor/format options and full application integration
are separate obligations. Registry presence is not full behavior coverage.
Source cache checks use the same deterministic clock for TTL coercion, defaults,
settings units, inclusive expiry, prefix cleanup, falsy values and root JSON
persistence. Actual YAML wrappers cover merges/tags/errors, JSON serialization
and legacy dump options. Public helper groups reuse the actual IP/UUID/port,
country flag, JSON5, MD5 and flow implementations. They check Address4/Address6
constructors, BigInt/subnets, coercion/errors, deterministic random boundaries,
Unicode names, parser/stringifier options and fixed digest vectors. The fixed
UTC clock pins reset-day/month/cycle behavior without claiming other deployment
timezones or locale-dependent expired-date messages.

Flow groups execute the source HTTP/cache/request queue with Rust-owned network
I/O: HEAD/GET/body fallback, custom headers, noFlow/noCache, inclusive TTL expiry,
timeouts/late completion, one-slot queue serialization, an async injected script,
and header-cache persistence/restart. Invalid GET body can fall back to headers;
non-success GET status throws before those headers are retained and returns
undefined. Flow and earlier processor groups issue 19 requests, including exact
flow User-Agent/token headers. The logger captures probe diagnostics only; it
does not implement production log formatting/delivery. Every result/observation
and complete persisted state must match; two more independent processes
reload both files and all three source caches without any HTTP. Missing or duplicate
case IDs and false/zero output changes are rejected by runner self-checks.

`PC41`-`PC52` exercise actual DoH/Custom resolver declarations and the locked
dns-packet encoder/decoder over Rust HTTP. The independent Python wire fixture
requires all 23 DNS requests: A/AAAA, no EDNS or IPv4 /24/IPv6 /56 subnet packets,
Base64url query, binary responses, CNAME filtering, HTTP-status/rcode precedence,
malformed responses and timeout without late cache writes. Cache checks retain
aliases, normalized URL lists, non-enumerable winning-URL metadata, inclusive TTL,
noCache and separate insecure keys. Sequential fallback and a two-request race
assert that no queued third resolver is contacted. The race loser cannot respond
until the script acknowledges its observed fast winner; only those two concurrent
wire records may reorder. Exhaustion retains each source error. A real async
script uses both injected APIs, and fresh processes reload the complete DNS
resource cache. These pre-Gist contracts make 42 HTTP requests per runtime.

`PC53`-`PC64` additionally use real Rust UDP/TCP/DoT transports with the same
dns-packet encoding/decoding and Custom resolver source. Each runtime must issue
all twenty exact raw DNS packets and seventeen TCP/TLS connections. Check IPv4
and IPv6 loopback, hostname resolution, EDNS, fragmented length/body reads and
first-frame selection, premature EOF, malformed packets, timeouts, peer closure,
mixed-protocol fallback, cache partition and actual async script injection.
Fragmented replies take over 280 ms with a 200 ms inactivity timeout; an absolute
deadline would fail. TLS hostname calls must send SNI; IP calls must omit it.
The disposable self-signed certificate must fail by default and with a truthy
number; only explicit boolean skipCertVerify permits it. Handshake signatures
remain verified. The checked-in dns-key.pem is a public test key used only by
the loopback fixture, and never loaded by the Rust client or production service.
The native bridge reuses locked tokio-rustls/platform-verifier crates without
new transitive packages. DNS-only URL fields use Rust's URL parser; no complete
global web URL API is claimed. Twenty-two runner self-checks include packet/frame,
SNI/certificate, missing/extra request, fragment timing and leaked-connection
mutations. TLS trust-store success, proxy and fixed public-provider integration
remain open, as does integration with a production Rust DNS/cache service.

`PC65`-`PC76` add a synchronous Rust MMDB reader (`maxminddb` 0.32.0, ISC)
while inheriting the actual source `geoip`/`ipaso`/`ipasn` projections. Official
public Country/ASN test databases and independent source JSON are pinned in
`fixtures/mmdb/provenance.json` with their MIT license, sizes and SHA256 hashes.
Directory attributes preserve these upstream bytes on Windows checkouts.
The harness copies them into disposable Unicode paths and supplies the actual
`SUB_STORE_MMDB_COUNTRY_PATH` / `SUB_STORE_MMDB_ASN_PATH` process defaults;
Docker remaps both paths into its owned `/state` mount. Both runtimes check
optional readers, explicit/falsy default precedence, IPv4/IPv6 values, numeric
ASN, absent fields, invalid inputs, wrong database kinds, missing addresses/files,
constructor ordering, eager reads after deletion, instance isolation and async
script persistence. Each process/reload records intact input hashes and the
removed transient file. No Node module capability is exposed to Rust scripts.
PC74 asserts `InvalidDbBufferError` and each parser's exact current diagnostic;
its shared result compares the error class and successful diagnostic check.
The malformed-buffer texts differ and full error-text parity remains open.
Complete underlying public ReaderModel objects, custom input objects/path forms,
and production Rust service/host integration also remain open.

`PC77`-`PC88` run the verbatim public Gist source with the Rust HTTP adapter:
constructor credentials/methods, missing lookup, Unicode/raw downloads, upload
filtering and caller mutations, placeholder create/retain/remove metadata,
validation without writes, structured/raw provider errors, timeout recovery,
all five serialized methods, GitLab file actions/limits and legacy download
failure, actual async-script injection and persisted typed results. The HTTP
adapter now retains client baseURL and shallow default/override semantics.
Sixty additional outgoing requests are checked independently, bringing the
processor total to 102 per runtime. Each processor run/reload also captures and
checks the full controlled provider state. Two harness mutation tests reject
wire/auth/order/body changes and missing/changed/extra provider files.

GitHub uses the actual custom API URL setting. A test-only wrapper redirects
GitLab's fixed public origin before the original queued HTTP methods; it retains
paths/auth/body/queue. This does not establish real GitLab TLS/proxy/origin
behavior. Source oddities are preserved: initial creation ignores the empty-file
fallback, and GitLab download still requests `/gists/:id` and fails. Default proxy
environment fallback, complete HTTP options and production backup/artifact
integration remain HG09/HG16/M4 work. No new dependency was added.

The HG10 download slice runs DL01-DL15 against actual Node exports and the
contained Rust host in four fresh processes, including two independent reloads.
It checks defaults/custom headers/fragments/GitHub prefixing, native local UTF-8
and malformed bytes, typed module content, cache TTL/threshold/bypass/optimistic
refresh/fallback, flow validation order/noFlow, both age types with cached armor
and structured errors, actual async script injection and binary filesystem,
status/redirect/error behavior. The text helper is frozen source; Rust owns its
HTTP/local I/O and implements downloadFile with streaming writes. Script-visible
Node APIs remain absent. Forty-eight exact requests per runtime, sixty raw case
markers, full typed state/two caches and nine files are independently checked.
The 262147-byte stream must expose its 16384-byte prefix before a separate HTTP
release; interrupted bodies retain exactly that prefix and reject. Raw wire,
commands/status, logs, input hashes, persisted state and file bytes are retained
and rejoined by check_download_artifacts, including Docker's owned containers.
Source plain-object age errors and post-cache flow failures remain unchanged.
The processor artifact slice now separately covers preprocessing/cache eligibility
and successful internal files. Full proxy/TLS/environment options and production
service integration remain open.

`shortcut: the probe uses a partial ProxyUtils/timer/storage/artifact host,
upgrade to the full shared-behavior fixtures before M3 acceptance.` Separate
negative probes verify 100ms interruption of synchronous and promise-job infinite
loops, 64MiB memory and 128KiB stack limits, error diagnostics and subsequent
execution recovery. The main script runtime also installs a 15-second interpreter
interrupt alongside its async timeout. The harness first builds the example, then
owns its binary directly so a 60-second watchdog can kill that process and preserve
partial output with exit 124. RTK still wraps builds and other short commands.
Every processor case synchronously writes its start/end ID to stderr before a
final JSON result is needed. The probe preserves partial traces on native exits
and requires all ordered pairs on success (none on reload). These records
diagnose a crash; a later pass does not establish or repair its native cause.
Production isolation, complete host semantics and per-request resource cleanup
remain unverified; passing the contained experiment does not establish them.
In particular, the new adapter's direct JSON writes are limited to disposable
probe files; atomic production persistence belongs to M2. HTTP proxy/TLS/compression,
full DNS application integration, logging/notification delivery, full callback lifecycle, Rust resource-cache
services and actual application conversion/artifact calls still need their own shared
contracts before M3/M4 production acceptance.

`rust-compat-native.yml` runs the probe, runner self-checks, thirteen Node HTTP
scenarios, one restart, two merged-share, 66 conversion, thirteen resource, six
operation, one migrated-data restart, three configured-CORS, eight Gist and five
node-info scenarios,
plus the complete repaired Node unit suite on native `ubuntu-24.04`,
`ubuntu-24.04-arm`, and `windows-2025`. Each platform uploads independent logs
and JSON summaries even on failure. A successful probe is not a full Rust
compatibility pass. The same workflow also runs the twenty-flow browser slice in
headless Chrome on Linux x64, and the contained release probe in native Linux
x64/arm64 Docker containers. Complete UI and production container acceptance
remain separate gates.

Container acceptance reuses every shared host/processor assertion and the Node
oracle, with no Node installation in the Rust runtime image:

```sh
python tests/compat/probe.py --node /absolute/path/to/node --docker
```

Run this on native Linux with Docker and the generated helpers available. The
Dockerfile pins multi-architecture Rust/Debian image digests, builds the locked
release example inside Docker, and copies the binary, helper bundle, licenses and CA
certificates into the runtime. Its build asserts Node/npm are absent. The runner
checks image and executed binary architecture against the host, uses a separate
writable state bind mount and read-only container root, and connects to the
controlled loopback upstream with Linux host networking. Fresh containers reload
both persisted files and source caches without HTTP. Each owned container is
forcibly removed even on watchdog timeout; the unique test image is removed at
the end. Build/runtime/image metadata, failures and exact requests are uploaded.
This packages the M1 experiment, not the future Rust HTTP service. The external
Node oracle and frontend build tooling remain development-only.

The installed agent-browser native CLI can exercise the unchanged frontend:

```sh
python tests/compat/frontend.py --node /absolute/path/to/node \
  --frontend /path/to/pinned-frontend-checkout \
  --browser /path/to/native-agent-browser
```

Build that checkout first with its frozen pnpm lockfile and `pnpm build`, using
Node 24.15.0 and pnpm 11.0.9. Install agent-browser 0.27.3 and its Chrome with
`agent-browser install --with-deps` on Linux (`install` on Windows). The default
browser is headless; an interactive desktop is not required in Actions. CI checks
out the frontend at the pinned commit, runs its existing type check/build without
patches and executes the same harness. The harness verifies the CLI version,
records actual browser/host metadata and switches English then Chinese through
the real language control so every host exercises selection. It waits for menu
animations and notifications before clicks, including the closing backdrop that
can outlive the language popup. Opening requires the whole menu inside the viewport
with an unobstructed center; an empty animation list can precede animation startup.
The add-subscription popup uses the same readiness check. Named clicks first wait
for their rendered text, since navigation can finish before Vue renders controls.
Text controls use exact labels;
duplicated language menus require the unique visible row, not the first global
text match. Candidate DOM/visibility/coordinates are preserved in the report.
The harness verifies the frontend commit/source, records hashes of every built
asset, and uses a fresh browser session and
disposable merged Node listener. The fixture config permits exactly its local
origin through the existing CORS setting. URL-based backend setup, subscription
create/reopen/rename/preview, local-file create/reopen/rename/preview, count-share
creation/public access/reopen/update, collection create/reopen/rename/preview,
file archive/restore, log-retention settings, log search/clearing and manual
backup download/restore have
UI/persisted-state/upstream assertions. Each workflow also requires its actual
successful backend request in the HAR; preview checks inspect the preview window.
File editor input uses keyboard selection/replacement and the visible content
tab. Screenshots, command logs, HAR and page errors are preserved on
failure. No frontend source or application state is injected to perform these
operations. The collection picker and checkbox scroll instantly to the viewport
center and require an unobstructed hit before an actual CLI mouse click. Fixed
pixel scrolling can return before smooth movement ends, leaving the click under
the fixed footer. Native scrollintoview also inherits smooth behavior, so this
viewport-only action uses scrollIntoView with behavior:instant; no application
state or click handler is invoked through JavaScript. On editor reload, wait
for actual field/selection values and restored content before asserting or saving;
rendered controls can precede asynchronous resource data. Before deletion, wait
for the swipe menu's actual transform to finish; its archive dialog button is a
visible NutUI control without an ARIA button role. Restoration checks the captured archive ID
in HAR, the persisted snapshot and the reopened editor. Log settings wait for
initial network activity to settle before editing, then check the input before
saving: the initial store load can overwrite an already rendered form. They retain
the form's string value and keep editing after saving. Log search checks the selected
keyword in both the rendered results and the actual request query; clearing
requires UI confirmation and empty persisted logs. The file editor selects the
visible display tab when its previous content tab remains active. A manual backup
must deliver exactly the persisted JSON in its captured network response and
leave state intact. The file input restores a fixed backup with Unicode/CRLF,
unknown/falsy fields and excluded script text; after the automatic reload both
stored values and resource lists must match. The fixture hash is recorded.
This is a twenty-workflow slice; all other UI workflows remain pending.

`frontend-contract-map.json` links all 48 API request objects to source-checked
backend routes, concrete dynamic type domains, partial HTTP/UI cases and milestone
owners. It does not assert all query/body/workflow variants are covered. Regenerate
source discovery into a new output file (previous evidence is immutable):

```sh
node tests/compat/inventory.cjs /path/to/pinned-frontend-checkout /path/to/new-inventory.json
```

The inventory command rejects missing/stale call mappings, backend routes,
HTTP fixture IDs and UI workflow references, and missing parse/download checks for any exported proxy output alias
or base/unsupported rule producer. Its 114 direct URL expressions still require complete builder and
parameter analysis; the two mapped builders are only an initial slice.
The inventory checks the unchanged frontend and exact approved backend patch,
including backend source and lockfile.
Computed processor names resolve through top-level string constants; parser and
preprocessor factory names remain explicit. Unresolved registry entries are parse
gaps, and the inline checks cover computed names, inner-scope shadowing, factories
and unknown bindings. Registry discovery does not establish behavior coverage.
