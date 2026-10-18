# Sub-Store Rust service (M2, in progress)

This branch now includes an independent Rust HTTP server in `src/`. It serves
subscription/collection/settings CRUD and legacy JSON storage without Node.
The JavaScript feasibility example remains separate. Conversion, script execution
inside the application, archive actions, Gist/age and remaining integrations are
still under development; this is not a complete replacement release.

All builds and tests run in the **Rust service and compatibility** GitHub Actions
workflow. Native Linux x64/arm64 and Windows x64 jobs produce release binaries;
Linux x64/arm64 Docker jobs build and run `rust/Dockerfile`. The browser job uses
the pinned unchanged frontend and runs a separate initial CRUD slice against the
Rust listener, including subscription/collection edits, settings and manual
export/restore. No local build, test or Docker setup is required.

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

Existing data is snapshotted as `sub-store.pre-rust.json` / `root.pre-rust.json`
before Rust writes. Corrupt data causes startup to fail without resetting it.
Use copies for evaluation and keep the originals for rollback. Run only one
backend against a writable data directory.

The HTTP environment keeps the frontend's `Node` discriminator, identifies Rust
in metadata, and advertises version `0.1.0` with final feature parity incomplete.
Archive deletion and Gist/age setting mutations return an explicit pending error.
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
before dispatch. Uncaught errors use production HTML responses. Development stack
bodies and invalid-reference-graph mutations remain unverified.
The HTTP entry point selects strict JSON, extended URL-encoded forms or raw
bytes before routing, including environment requests. Identity-encoded UTF-8
and UTF-16LE JSON, nested/repeated form fields, settings array/byte spreads and
raw-buffer CRUD retain legacy behavior. JSON uses SUB_STORE_BODY_JSON_LIMIT;
form/raw bodies retain their separate 100 KiB limit. Invalid JSON, excessive
size/depth/parameters and unsupported charset/encoding return production HTML
errors without publishing state. The legacy verifier rejects nonempty UTF-16BE
and UTF-7 bodies with 403 while permitting empty bodies in these charsets.
The locked decoder rejects UTF-32 with 415.
Gzip and zlib-wrapped deflate requests stream through the same parsers, applying
limits to expanded bytes. Concatenated gzip members are decoded before parsing;
corrupt/truncated streams return 400 and expanded overflow returns 413 without
publishing data. Other UTF aliases, exotic form overflow collisions and
raw-buffer bulk replacement remain M2 gaps.
Raw JSON exports retain the legacy local-calendar filenames and escaped resource
names. Manual restore keeps the pre-migration backup in `root.json`'s `sub-store`
key and preserves unrelated root entries. A private undo record protects the two
files during restore; startup recovers an interrupted restore before serving.
Object and array settings survive plain/Base64 restore and restart; settings
PATCH preserves an imported array's numbered fields when merging its changes.
Legacy migration treats falsy lists as empty maps, matching the old backend.
Malformed recovery records stop startup without resetting either file. Let Rust
recover a pending restore before rolling back to Node. The no-Gist refresh path
clears the three resource caches used by frontend restore; Gist refresh remains
pending. Merged frontend hosting preserves gzip/identity preferences, compressed
assets, legacy cache headers, filesystem ETag/Last-Modified validators, conditional
304 responses and representation metadata on HEAD. Standalone hosting reuses
these assets with legacy Accept-based history fallback and API/download forwarding.
Its public share proxy returns an explicit pending error until M4 implements
token enforcement. Both hosting modes support selected-representation byte ranges,
If-Range, If-Match and If-Unmodified-Since, including conditional/HEAD precedence.
Business API responses and JSON exports retain content-based weak ETags and UTF-8
lengths. Successful GET/HEAD revalidation returns 304; HEAD preserves entity
metadata. Mutations and errors keep their original status, and empty end-style
responses and production HTML errors remain untagged. Both frontend listeners
reuse this behavior. Range combination and validator matching retain the locked
Node dependency's behavior. Static 412/416 errors use production HTML messages;
Node development stack bodies and broader HTTP/API edge cases remain M2
acceptance work.
Stored excluded Node scripts remain intact.
