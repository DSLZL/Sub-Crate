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

`smoke` is currently eight ordered scenarios. These write/delete fixture data;
use only disposable state. Response status, selected headers, JSON types/raw
content, persisted side effects, and controlled upstream requests are asserted.
A cached flow lookup must make zero additional upstream requests. Missing
fixtures, failed assertions, unavailable backends and `python -O` fail. Existing
result files are never overwritten. `--suite full` intentionally fails while
the inventory and executable fixture set remain incomplete. A scoped contract
success is always labeled `full_coverage: false` and `rust_parity: false`.

The orchestrated `--mode contracts` also stops its first listener, starts a new
process over the same disposable state, and runs the separate `restart` scenario.
That stage verifies backup-imported excluded script bytes (Unicode and CRLF/LF)
and unknown/falsy fields survive export and restart. It has independent listener
logs and evidence. Running `--suite restart` alone requires the D03 state seed.

The Rust example evaluates unchanged `scripts/ip-flag-node.js` with controlled
HTTP, request headers, memory-backed storage, locked lodash/YAML/Buffer/Base64
helpers, promises/timers, and asynchronous Rust-to-JS callback re-entry. It
checks full/shortcut filters, operators and response transforms, timer arguments
and cancellation, absent Node APIs, and excluded script text in storage. No Node
executable is launched by the Rust probe. Helper generation uses Node only at
development/build time.

`shortcut: the probe uses a partial ProxyUtils/timer/storage/artifact host,
upgrade to the full shared-behavior fixtures before M3 acceptance.` The async
deadline does not interrupt CPU-bound JS; production execution limits and
complete host semantics remain unverified.

`rust-compat-native.yml` runs the probe, runner self-checks, eight Node HTTP
scenarios plus a separate restart scenario and frozen unit baseline on native `ubuntu-24.04`,
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
create/reopen/rename and preview currently have UI/persisted-state/upstream
assertions. Screenshots, command logs, HAR and page errors are preserved on
failure. No frontend source or application state is injected to perform these
operations. This is a four-workflow slice; all other UI workflows remain pending.
