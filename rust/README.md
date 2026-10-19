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
Final source-inventory and integration/release qualification remains part of the
parent rewrite; this is not a complete replacement release.

The engine embeds locked generic utility packages for formats, hashes, IP/bytes,
cryptography, DNS packets and the Web primitives required by those helpers.
Application conversion, processing and generation policies remain native Rust.
The utility-only build in Actions records package input hashes and license
notices in `vendor/`; the development application-helper bundle is separate.

All builds and tests run in the **Rust compatibility native baseline** GitHub Actions
workflow. Native Linux x64/arm64 and Windows x64 jobs produce release binaries;
Linux x64/arm64 Docker jobs build and run `rust/Dockerfile`. The browser job uses
the pinned unchanged frontend: twenty workflows each against Node, merged Rust
and standalone Rust, including resource editing, previews, shares, archives, logs,
permanent deletion and restored/deleted-state restarts. No local build, test or
Docker setup is required.

M4 business-feature acceptance passed at `04357fb8` in
[Actions 39323623553](https://github.com/DSLZL/Sub-Store/actions/runs/39323623553).
The M1 final inventory and M5 integration/release gates remain open.

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
Rust metadata identifies crate version `0.1.0`, milestone M4 and the incomplete
parent rewrite. Archive operations and Gist/age settings use their native owners.
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

Subscription rename and permanent deletion retain the legacy errors for invalid
collection references, including live versus stored raw buffers. Rejected Rust
mutations retain both durable files and the published graph. The old backend can
partly mutate references in memory or persist a deletion before raising the same
error; its original behavior remains captured separately in CI evidence.
