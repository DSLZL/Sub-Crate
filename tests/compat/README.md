# Rust rewrite compatibility baseline (M1, in progress)

This directory tests the frozen backend, builds development-only JS helpers,
and captures native rquickjs feasibility evidence. It does **not** contain a
replacement Rust server or demonstrate complete frontend/backend parity.

- Backend: `9cc6b5fac966993364ae33da8797f446bbf7f571` (2.42.4).
- Unchanged frontend: `4bd7b0acbe65466b933ccea14fc61dbebe036ead` (2.34.0).
- Development oracle: Node 24.15.0, pnpm 11.0.9, locked backend dependencies.
- Contained Rust experiment: Rust 1.98.0, rquickjs 0.14.0, locked Cargo dependencies.

Run from the repository root, after `pnpm --dir backend install --frozen-lockfile`.
Replace `/absolute/path/to/node` with the pinned Node executable; the harness
checks its exact version. On Windows use native paths and Python 3.

```sh
python -m unittest discover -s tests/compat -p test_runner.py
python tests/compat/reference.py --node /absolute/path/to/node --mode unit
python tests/compat/reference.py --node /absolute/path/to/node --mode contracts
python tests/compat/reference.py --node /absolute/path/to/node --mode helpers
python tests/compat/probe.py
```

The pinned backend's Babel alias plugin cannot load Windows filenames. Use
`--mode unit-windows` for a separately labeled run with a development-only
loader that retains the Babel preset and resolves aliases as native paths.
The original loader failure remains evidence; neither loader changes backend
source. Windows local-file and symlink failures still fail the run.

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
Another standalone listener runs `resources`: twelve scenarios and 73 requests
cover age X25519/hybrid key generation and fixed-key derivation, five resource
sorts and four legacy replacements, artifact CRUD and upload-disabled sync,
archive sorting/deletion/restore, and enabled/disabled log capture and filtering.
The key fixtures are disposable test keys. Dynamic keys, log IDs/times and sync
times have typed captures and checks against subsequent responses and state.
The SS output samples cover alias dispatch, not every protocol/option combination.
Expected JSON fields and output bytes are fixed fixtures, reviewed against the
frozen source; acceptance never regenerates expectations from its target.

Frozen negative behavior remains explicit: unsupported rules can produce
500/null-length errors in QX/Surge/Loon; a failed function script can fall back
to a shortcut and retain the original nodes, while a top-level throw fails.
Shortcut filters require an explicit return; omitting it selects no nodes.
Missing artifact PATCH returns DUPLICATE_KEY; legacy artifact PUT accepts names
that POST rejects. Invalid archive-sort payload returns the frozen HTML 500 body.
No-token artifact restore is a separately classified defect: its unawaited promise
rejection terminates Node 24 without an HTTP response. It is not counted as a
passing HTTP case; provider restore remains an open gate.

The Rust example evaluates unchanged `scripts/ip-flag-node.js` with controlled
HTTP, request headers, memory-backed storage, locked lodash/YAML/Buffer/Base64
helpers, promises/timers, and asynchronous Rust-to-JS callback re-entry. It
checks full/shortcut filters, operators and response transforms, timer arguments
and cancellation, absent Node APIs, and excluded script text in storage. No Node
executable is launched by the Rust probe. Helper generation uses Node only at
development/build time.

`shortcut: the probe uses a partial ProxyUtils/timer/storage/artifact host,
upgrade to the full shared-behavior fixtures before M3 acceptance.` Separate
negative probes verify 100ms interruption of synchronous and promise-job infinite
loops, 64MiB memory and 128KiB stack limits, error diagnostics and subsequent
execution recovery. The main script runtime also installs a 15-second interpreter
interrupt alongside its async timeout. The harness first builds the example, then
owns its binary directly so a 60-second watchdog can kill that process and preserve
partial output with exit 124. RTK still wraps builds and other short commands.
Production isolation, complete host semantics and per-request resource cleanup
remain unverified; passing the contained experiment does not establish them.

`rust-compat-native.yml` runs the probe, runner self-checks, thirteen Node HTTP
scenarios, one restart, two merged-share, 66 conversion and twelve resource scenarios,
plus the frozen unit baseline on native `ubuntu-24.04`,
`ubuntu-24.04-arm`, and `windows-2025`. Each platform uploads independent logs
and JSON summaries even on failure. A successful probe is not a full Rust
compatibility pass. Docker and all frontend workflows remain separate gates.

The installed agent-browser native CLI can exercise the unchanged frontend:

```sh
python tests/compat/frontend.py --node /absolute/path/to/node \
  --frontend /path/to/pinned-frontend-checkout \
  --browser /path/to/native-agent-browser
```

Build that checkout first with its frozen pnpm lockfile and `pnpm build`, using
Node 24.15.0 and pnpm 11.0.9. The harness verifies the frontend commit/source,
records hashes of every built asset, and uses a fresh browser session and
disposable merged Node listener. The fixture config permits exactly its local
origin through the existing CORS setting. URL-based backend setup, subscription
create/reopen/rename/preview, local-file create/reopen/rename/preview, count-share
creation/public access/reopen/update, and collection create/reopen/rename/preview have
UI/persisted-state/upstream assertions. Each workflow also requires its actual
successful backend request in the HAR; preview checks inspect the preview window.
File editor input uses keyboard selection/replacement and the visible content
tab. Screenshots, command logs, HAR and page errors are preserved on
failure. No frontend source or application state is injected to perform these
operations. The collection picker scrolls into view before selecting its visible
checkbox; the fixed footer otherwise covers the control. This is a thirteen-workflow
slice; all other UI workflows remain pending.

`frontend-contract-map.json` links all 48 API request objects to source-checked
backend routes, concrete dynamic type domains, partial HTTP/UI cases and milestone
owners. It does not assert all query/body/workflow variants are covered. Regenerate
source discovery into a new output file (previous evidence is immutable):

```sh
node tests/compat/inventory.cjs /path/to/pinned-frontend-checkout /path/to/new-inventory.json
```

The inventory command rejects missing/stale call mappings, backend routes and
fixture IDs, and missing parse/download checks for any exported proxy output alias
or base/unsupported rule producer. Its 114 direct URL expressions still require complete builder and
parameter analysis; the two mapped builders are only an initial slice.
