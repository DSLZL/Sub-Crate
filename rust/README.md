# Sub-Store Rust service

This branch now includes an independent Rust HTTP server in `src/`. It serves
subscription/collection/settings CRUD and legacy JSON storage without Node.
Local subscriptions share native Rust parsing and JSON/URI/V2Ray and
Clash/Mihomo/Stash/Shadowrocket, QX/Loon, Surge/SurgeMac/Surfboard, Egern and sing-box output through
`/api/proxy/parse`, subscription preview and subscription download. Native input
includes SS/SSR, HTTP/SOCKS, Trojan, Hysteria1/2, TUIC, WireGuard, VMess, VLESS and
AnyTLS, plus the registered Surge/Loon/QX and Clash JSON5/YAML formats and
Base64/SSD/FullConfig/Clash preprocessing
with transport/TLS normalization. Surge External fields are parsed as data.
V2Ray supports all three legacy aliases.
SurgeMac's external/forced/fallback/merged modes generate native configuration
data and retain port/override policies; configured programs are never executed.
Egern retains its protocol filters, stream/TLS/ShadowTLS fields, WireGuard peers,
internal objects and compact or pretty YAML through both legacy aliases.
Both sing-box aliases retain the existing protocol filters, transport/TLS and
plugin options, paired ShadowTLS outputs and separate WireGuard/Tailscale endpoints.
`/api/rule/parse` uses native HTML/provider preprocessing and all thirteen rule
types to produce QX, Surge, Loon and Clash rules with the existing error policies.
The seventeen built-in processors and general JavaScript operators, filters and
response transforms run in the service through rquickjs. Subscription and
collection previews/downloads share the native pipeline, including ordered
local/remote sources, provenance, nested generation, linked/module scripts and
existing fallback policies. Raw input starting `<!DOCTYPE html>` is discarded
before parsing. Node builtins, require/npm loading and native addons are excluded;
stored Node-only scripts remain intact.

Scripts can use the native storage/cache, HTTP, DNS, MMDB, Gist, age, download,
flow and conversion helpers. Explicit script writes retain their original timing
across failures and restart. The development feasibility example remains separate.
Native file/module/artifact management, token sharing, archives and manual/batch
synchronization use those same helpers. Gist/GitLab providers, plaintext/Base64/age
backups, cache refresh, logs and configured notifications retain the existing
business contracts. Artifact and environment jobs use native scheduling and the
same generation, backup and MMDB owners as HTTP requests.
This is the M5 implementation candidate for the standalone replacement defined
in [ACCEPTANCE.md](ACCEPTANCE.md). Accept a source revision only after its complete
six-job workflow and source/evidence review pass; artifact delivery does not
publish a release or switch production data.

The engine embeds locked generic utility packages for formats, hashes, IP/bytes,
cryptography, DNS packets and the Web primitives required by those helpers.
Application conversion, processing and generation policies remain native Rust.
The utility-only build in Actions records package input hashes and license
notices in `vendor/`; the development application-helper bundle is separate.

All builds and tests run in the **Rust compatibility native baseline** GitHub Actions
workflow. Native Linux x64/arm64 and Windows x64 jobs produce release binaries;
Linux x64/arm64 Docker jobs build and run `rust/Dockerfile`. The browser job uses
the pinned unchanged frontend: twenty-one workflows each against Node, merged Rust
and standalone Rust, including resource editing, previews, shares, archives, logs,
permanent deletion, configured backend switching/cache refresh and
restored/deleted-state restarts. No local build, test or
Docker setup is required.

The complete gate includes all earlier service/state, conversion/script and
business-feature contracts, then verifies the actual distributed binaries,
reloaded Docker images and extracted official frontend assets. The final
acceptance boundary and runtime qualifications are recorded in
[ACCEPTANCE.md](ACCEPTANCE.md).

Downloaded service binaries use the existing environment variables:

- `SUB_STORE_BACKEND_API_HOST` / `SUB_STORE_BACKEND_API_PORT`: listen address
  (defaults `::` / `3000`).
- `SUB_STORE_DATA_BASE_PATH`: data folder, retaining `sub-store.json` and
  `root.json` (default current directory).
- `SUB_STORE_BACKEND_PREFIX` / `SUB_STORE_FRONTEND_BACKEND_PATH`: existing prefix
  mode and backend path. Prefix/merge modes require a path starting with `/`.
- `SUB_STORE_BACKEND_MERGE` / `SUB_STORE_FRONTEND_PATH`: serve unchanged frontend
  assets in the same listener. Without merge, FRONTEND_PATH starts a separate
  listener using `SUB_STORE_FRONTEND_HOST` / `SUB_STORE_FRONTEND_PORT` (defaults
  backend host / `3001`). FRONTEND_BACKEND_PATH mounts API/download forwarding
  to the same Rust backend; without that path the frontend port serves only
  static assets. Configure CORS to allow the frontend origin when needed.
- `SUB_STORE_CORS_ALLOWED_ORIGINS`, `SUB_STORE_BACKEND_CUSTOM_NAME`,
  `SUB_STORE_X_POWERED_BY`, `SUB_STORE_BODY_JSON_LIMIT`: existing HTTP policy.
  JSON limits accept binary units such as `1mb` and `1.5 kb`, or the legacy
  decimal integer prefix. Limits must be finite and nonnegative.
- `NODE_ENV`: `production` returns the legacy HTTP reason in uncaught HTML
  errors; other values include native Rust traces. Trace frames and JSON/zlib
  parser diagnostic wording differ from Node.
- `SUB_STORE_BACKEND_SYNC_CRON`, `SUB_STORE_BACKEND_UPLOAD_CRON` and
  `SUB_STORE_BACKEND_DOWNLOAD_CRON`: existing server sync/backup jobs.
  Artifact `cron` values are managed through the artifact API.
- `SUB_STORE_PRODUCE_CRON`: existing scheduled generation entries.
- `SUB_STORE_MMDB_COUNTRY_URL` / `SUB_STORE_MMDB_COUNTRY_PATH`,
  `SUB_STORE_MMDB_ASN_URL` / `SUB_STORE_MMDB_ASN_PATH` and `SUB_STORE_MMDB_CRON`:
  configured database download and refresh.
- `SUB_STORE_DATA_URL` / `SUB_STORE_DATA_URL_POST`: startup data restoration and
  the existing general-JavaScript postprocessing hook.

Existing data is snapshotted as `sub-store.pre-rust.json` / `root.pre-rust.json`
before Rust writes. Corrupt data causes startup to fail without resetting it.
Use copies for evaluation and keep the originals for rollback. Run only one
backend against a writable data directory.

The HTTP environment keeps the frontend's `Node` discriminator and protocol
version `2.42.4`, which the unchanged frontend uses to select supported features.
Rust metadata identifies crate version `0.1.0`, milestone M5 and completion within
the declared standalone scope. Archive operations and Gist/age settings use their native owners.
Subscription/collection public recipients are validated and normalized natively
for both X25519 and MLKEM768-X25519, including checksum and padding. Settings
retain the submitted JSON types after legacy numeric validation, and old object
stores migrate using JavaScript's numeric-key ordering.
Subscription/collection CRUD keeps legacy create ordering and typed names,
permanent-delete errors, rename references and reserved-field behavior. Subscription
reference matching is intentionally looser than collection matching. Array bodies
retain legacy create/PATCH behavior, and versioned subscription/collection lists
preserve mixed JSON entries across bulk writes, restart and restore. Null-record
lookup failures retain endpoint-specific errors; scalar JSON bodies are rejected
before dispatch. Invalid reference graphs return endpoint-specific errors;
rejected mutations preserve both files and the previously published state.
Bulk PUT rejects parsed objects, forms and skipped/empty JSON or form bodies with the
legacy caught-error envelope, before normalizing items or writing either data
file. Raw-byte bulk PUT retains the legacy Buffer JSON on disk and in exports.
Live buffers keep their lookup/duplicate/insertion behavior until restore or
restart, when they become ordinary JSON objects. Failed writes retain both
files and the previous live identity. JSON Buffer lookalikes submitted as bulk
bodies remain ordinary objects and are rejected before publication.
The HTTP entry point selects strict JSON, extended URL-encoded forms or raw
bytes before routing, including environment requests. Identity-encoded UTF-8
and UTF-16LE JSON, nested/repeated form fields, settings array/byte spreads and
raw-buffer CRUD retain legacy behavior. JSON uses SUB_STORE_BODY_JSON_LIMIT;
form/raw bodies retain their separate 100 KiB limit. Invalid JSON, excessive
size/depth/parameters and unsupported charset/encoding return mode-selected HTML
errors without publishing state. The legacy verifier rejects nonempty UTF-16BE
and UTF-7 bodies with 403 while permitting empty bodies in these charsets.
The locked decoder rejects UTF-32 with 415. Media selection preserves the locked
name/suffix rules, quoted parameters, escaped pairs and Latin-1 header bytes.
The last charset wins; an empty charset defaults to UTF-8. Recognized UTF aliases
retain their 403 verifier / empty-body behavior, including after decompression.
Gzip and zlib-wrapped deflate requests stream through the same parsers, applying
limits to expanded bytes. Concatenated gzip members are decoded before parsing;
corrupt/truncated streams return 400 and expanded overflow returns 413 without
publishing data. Form/query array overflow retains the locked decoders' separate
limits and merge rules; its private bookkeeping never enters user JSON.
Raw JSON exports retain the legacy local-calendar filenames and escaped resource
names. Jiff resolves the local zone and IANA `TZ` overrides on Windows and Linux;
Windows bundles its timezone data. Manual restore keeps the pre-migration backup in `root.json`'s `sub-store`
key and preserves unrelated root entries. A private undo record protects the two
files during restore; startup recovers an interrupted restore before serving.
Legacy Base64 imports and startup data retain the old decoder's URL-safe symbols,
padding/noise cleanup, permissive final bits and UTF-8 replacement behavior.
Decoded data must still pass JSON, settings and record validation; rejected
imports preserve both files. Private restore undo-record fields remain strict.
Non-null JSON settings survive import; truthy booleans, numbers, strings,
objects and arrays retain their type on reads and startup. Reading imported
false/zero/empty-string settings durably initializes an empty object. Object
and array settings retain their existing PATCH behavior; boolean/number PATCH
spreads no old fields. String property spreading uses the original UTF-16 code
units. JSON strings and keys retain isolated surrogates through parsing, export,
import and restart; valid pairs and literal private-use/backslash sequences keep
their original values.
Legacy migration treats falsy lists as empty maps, matching the old backend.
Malformed recovery records stop startup without resetting either file. Let Rust
recover a pending restore before rolling back to Node. Refresh uses the native
resource caches and configured Gist store owners. Merged frontend hosting preserves
gzip/identity preferences, compressed assets, legacy cache headers,
filesystem ETag/Last-Modified validators, conditional
304 responses and representation metadata on HEAD. Standalone hosting reuses
these assets with legacy Accept-based history fallback and API/download forwarding.
Its public share proxy uses the same native token, expiry, counter and generation
owners as the backend. Both hosting modes support selected-representation byte ranges,
If-Range, If-Match and If-Unmodified-Since, including conditional/HEAD precedence.
Business API responses and JSON exports retain content-based weak ETags and UTF-8
lengths. Successful GET/HEAD revalidation returns 304; HEAD preserves entity
metadata. Mutations and errors keep their original status, and empty end-style
responses and production HTML errors remain untagged. Both frontend listeners
reuse this behavior. Range combination and validator matching retain the locked
Node dependency's behavior. Static 412/416 errors use the same error-mode policy.
CRUD route tokens accept case variants and one trailing slash. Names retain
their case and decode once after raw segment matching, including encoded slashes
and percent signs. Extra segments remain unregistered; invalid escapes/UTF-8
return HTML 400 without changing durable data. Prefix admission keeps
the legacy raw path policy.
Standalone forwarding collapses repeated path slashes before backend matching,
matching the old proxy, while preserving the query and encoded name characters.
On M2 routes, OPTIONS with a nonempty allowed Origin is a CORS preflight;
originless OPTIONS goes through parsing and returns the legacy ordered Allow
methods and entity metadata. Root non-GET requests retain the backend greeting.
Unsupported methods return escaped production HTML 404 responses with the original
pathname, while GET/HEAD wildcard misses and explicit prefix rejections stay empty.
Structured business errors retain their response bodies and status.
CRUD query flags preserve repeated and bracketed values. Empty scalar raw flags
keep wrapped JSON; nonempty strings, arrays and objects select raw export.
Ambiguous array/object delete modes return INVALID_DELETE_MODE without changing
state. Query decoding retains plus/percent behavior, its 1,000-parameter cap,
depth-five remainder and array limit, separately from body form limits.
Object-to-primitive failures retain their endpoint-specific errors and preserve
state. JSON numbers and name coercion use the legacy double-precision values and
decimal/exponent boundaries, including negative zero.
Stored excluded Node scripts remain intact.

## Downloadable delivery

Use the artifacts of the accepted **Rust compatibility native baseline** run.
Each `m5-distribution-<target>-<run>-<attempt>` artifact contains a versioned
archive, `SHA256SUMS` and an external `manifest.json`. Unpack the GitHub artifact
first, verify the versioned archive against `SHA256SUMS`, then extract it.
The extracted directory contains the executable (or Docker `image.tar`), this
README, `Cargo.lock`, licenses, `SOURCE.md` and its own file manifest. Keep those
files together. Manifests record the exact commit, target, compiler, dependency
notices and file hashes; the GitHub artifact digest identifies the outer ZIP.

Linux archives are `sub-store-rust-0.1.0-linux-x64.tar.gz` and
`sub-store-rust-0.1.0-linux-arm64.tar.gz`. Their native runners use Ubuntu 24.04;
the current binaries require glibc 2.38 or newer and the shared libraries listed
in `manifest.json.linkage.dynamic_libraries`. These are GNU/Linux binaries.
Use the Docker delivery on hosts whose native library versions do not match.
The Windows x64 ZIP contains `sub-store-rust.exe`; its MSVC runtime is linked
statically. The package gate checks PE architecture and imported DLLs, including
the absence of a separate Visual C++ runtime dependency. Windows system DLLs
are still required; acceptance runs on the workflow's recorded Windows runner.

For example, after unpacking the outer GitHub artifact on Linux:

```sh
sha256sum -c SHA256SUMS
tar -xzf sub-store-rust-0.1.0-linux-x64.tar.gz
cd sub-store-rust-0.1.0-linux-x64
mkdir -p data
SUB_STORE_DATA_BASE_PATH="$PWD/data" SUB_STORE_BACKEND_API_HOST=127.0.0.1 ./sub-store-rust
```

On Windows, compare `(Get-FileHash .\sub-store-rust-0.1.0-windows-x64.zip
-Algorithm SHA256).Hash` with `SHA256SUMS`, then use PowerShell:

```powershell
Expand-Archive .\sub-store-rust-0.1.0-windows-x64.zip .\delivery
Set-Location .\delivery\sub-store-rust-0.1.0-windows-x64
New-Item -ItemType Directory -Force data | Out-Null
$env:SUB_STORE_DATA_BASE_PATH = (Join-Path $PWD data)
$env:SUB_STORE_BACKEND_API_HOST = '127.0.0.1'
.\sub-store-rust.exe
```

Both examples listen at `http://127.0.0.1:3000`; `/api/utils/env` returns the
legacy protocol metadata together with the Rust implementation metadata.
Only the selected data directory is mutable. No Node, npm or JS package
installation is needed to run the backend.

## Unchanged frontend assets

The `m5-frontend-<run>-<attempt>` artifact contains
`sub-store-frontend-4bd7b0acbe65.tar.gz`. It is the official pinned build without
source changes. Its extracted `dist` directory is the directory to serve.
The archive also includes the frontend license, package/lock files, available
dependency notices and source/build/asset hashes. Its frontend dependency report
describes the installed package set; the `licenses` folder contains available
installed notices, without claiming a new license audit of every frontend asset.

To serve that directory in the backend listener, add these variables to the
native startup above (PowerShell uses `$env:NAME = 'value'`):

```sh
export SUB_STORE_FRONTEND_PATH=/absolute/path/to/sub-store-frontend-4bd7b0acbe65/dist
export SUB_STORE_BACKEND_MERGE=true
export SUB_STORE_FRONTEND_BACKEND_PATH=/backend
```

Open `http://127.0.0.1:3000/subs?api=http%3A%2F%2F127.0.0.1%3A3000%2Fbackend`.
For the separate frontend listener, unset `SUB_STORE_BACKEND_MERGE`, retain
`SUB_STORE_FRONTEND_PATH` and the `/backend` mount, set
`SUB_STORE_BACKEND_PREFIX=true`, `SUB_STORE_FRONTEND_HOST=127.0.0.1`,
`SUB_STORE_FRONTEND_PORT=3001` and
`SUB_STORE_CORS_ALLOWED_ORIGINS=http://127.0.0.1:3001`. Open
`http://127.0.0.1:3001/subs?api=http%3A%2F%2F127.0.0.1%3A3001%2Fbackend`.
The frontend's own backend settings page can save/switch these URLs. Bind and
allowed-origin values must match the actual addresses used by the browser.

## Docker image archive

Extract the matching `sub-store-rust-0.1.0-docker-linux-x64.tar.gz` or
`sub-store-rust-0.1.0-docker-linux-arm64.tar.gz`. Its `manifest.json` records the
image tag, immutable image ID and platform. Load its `image.tar` and run the
tag printed by `docker load` (shown as `sub-store-m2:<run>` below):

```sh
docker load --input image.tar
mkdir -p data
docker run --rm --read-only --tmpfs /tmp --user "$(id -u):$(id -g)" \
  -p 127.0.0.1:3000:3000 -v "$PWD/data:/data" sub-store-m2:<run>
```

The mounted folder must be writable by that user. The runtime includes system
CA certificates and IANA timezone data on Debian bookworm; it contains no Node,
npm or Python. Add `-e TZ=Asia/Taipei` to select a timezone. To use merged
frontend hosting, mount the extracted `dist` directory read-only at `/frontend`
and add `-e SUB_STORE_FRONTEND_PATH=/frontend -e SUB_STORE_BACKEND_MERGE=true
-e SUB_STORE_FRONTEND_BACKEND_PATH=/backend`. Configuration and data behavior
match native startup. Both architecture gates save/remove/load the delivered
image and run the full service suite against that reloaded image identity.

## Existing data, backup and rollback

Stop the old backend, make a copy of its entire data folder, and start Rust
against the copy. Retain both `sub-store.json` and `root.json`, plus any files or
databases referenced by configured paths. Initial Rust snapshots supplement
that copy; they do not replace the offline original. Do not let two backends
write the same folder. Existing resource fields and stored script text remain
data, including excluded Node-specific scripts; those scripts need a general
JavaScript version to execute on Rust.

The unchanged frontend's manual backup and restore controls export/import the
existing storage format. Configured Gist backup/restore retains plaintext,
Base64 and age encryption; retain the encryption identity separately. Use the
same current configuration and keys when moving between backends. Corrupt data
or a failed restore must not replace the last valid state. The integrated gates
exercise exact unknown-field/script preservation, denied/corrupt writes,
restore, restart and Node-to-Rust-to-Node backup interoperability.

To return to the old backend, stop Rust and start the original version against
a separate copy of the latest valid data folder, or restore the retained
pre-migration backup if abandoning intervening changes. Keep the originals
until the chosen copy is verified through the frontend. Public release/tag,
registry publication and production cutover are separate actions; workflow
artifacts are reviewable deliveries and have the run's configured retention.

## Rebuild provenance

`SOURCE.md` links the exact backend source commit. The checked-in workflow pins
Rust 1.98.0, Node 24.16.0 and pnpm 11.0.9 and builds with the Cargo/pnpm lockfiles.
Node is build/reference tooling only. Generic utility source snapshots and
their input/package/license hashes live in `rust/vendor`; application policies
remain Rust. Push the exact source to a branch covered by that workflow in a
GitHub fork to rerun native builds, package extraction, full service acceptance,
both image load/run gates and unchanged frontend checks. Product execution for
this migration is performed exclusively in GitHub Actions.

Subscription rename and permanent deletion retain the legacy errors for invalid
collection references, including live versus stored raw buffers. Rejected Rust
mutations retain both durable files and the published graph. The old backend can
partly mutate references in memory or persist a deletion before raising the same
error; its original behavior remains captured separately in CI evidence.
