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
  assets in the same listener.
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
reference matching is intentionally looser than collection matching. Other malformed
payload and HTTP edge cases remain part of the unfinished M2 acceptance.
Raw JSON exports retain the legacy local-calendar filenames and escaped resource
names. Manual restore keeps the pre-migration backup in `root.json`'s `sub-store`
key and preserves unrelated root entries. A private undo record protects the two
files during restore; startup recovers an interrupted restore before serving.
Malformed recovery records stop startup without resetting either file. Let Rust
recover a pending restore before rolling back to Node. The no-Gist refresh path
clears the three resource caches used by frontend restore; Gist refresh remains
pending. Merged frontend hosting preserves gzip/identity preferences, compressed
assets, legacy cache headers, filesystem ETag/Last-Modified validators, conditional
304 responses and representation metadata on HEAD. Broader HTTP/static edge cases,
including ranges/preconditions and the standalone frontend proxy, remain M2
acceptance work. Stored excluded Node scripts remain intact.
