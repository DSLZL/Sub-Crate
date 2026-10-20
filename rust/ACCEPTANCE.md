# Standalone Rust compatibility acceptance

This replacement targets backend `9cc6b5fac966993364ae33da8797f446bbf7f571`
and official frontend `4bd7b0acbe65466b933ccea14fc61dbebe036ead` without
frontend source changes. It runs independently with Rust, rquickjs and locked
generic JavaScript utilities. Application policies, conversion, processors,
resources, storage, scheduling and host I/O have native Rust owners.

## Accepting a revision

Use a successful **Rust service and compatibility** workflow for the exact
source commit. All six jobs are mandatory: Linux x64/arm64 and Windows x64
native execution, Linux x64/arm64 native Docker execution, and the unchanged
frontend in headless Chrome. A partial job, older successful revision or probe
alone does not satisfy this gate. All product compilation, generation, tests,
servers, browsers and Docker execution run in Actions.

The finite source inventory covers 79 route registrations (77 standalone and
two excluded proxy-app host registrations), 33 proxy output aliases, 47 proxy
parsers, six preprocessors, seventeen processors and four rule output producers.
It also records 48 frontend request callers expanded to 60 paths, six dynamic
type domains, two direct URL builders, 114 classified URL literals and 29
environment options. Three additional SUB_STORE identifiers are logging regex
constants. Prior discovery inventories remain historical evidence.

The full workflow requires the original 826 Node tests on each native platform,
strict Node/Rust HTTP and persisted-state comparisons, outgoing request and
cache checks, general script/async/host calls, and Rust capability/error checks.
The frontend must pass all 21 declared workflows against Node, merged Rust and
separate Rust: CRUD/edit/reload, previews, shares, archives, logs, backups,
permanent deletion/restarts, configured backend selection, cache refresh and
backend data isolation. The same extracted official assets are used throughout.

Data acceptance includes unknown fields and stored scripts, plaintext/Base64/
age backups, denied/corrupt/interrupted writes, restart and Node rollback. It
does not modify production data. A replacement is not accepted while a required
inventory row lacks a native owner and actual result on its required targets.

## Deliveries and provenance

Three native archives, two Docker image archives and the official frontend
archive are downloadable from the successful run. Each carries source,
dependency/toolchain/target and license records, an exact file manifest and
archive checksums. Native tests execute the extracted delivered binary. Docker
tests remove the built tag and reload the extracted saved image before service
acceptance. Browser tests use the extracted frontend and delivered Rust binary.
The runtime container includes no Node, npm or Python.

The `m1-*` evidence artifacts retain commands, failures, source hashes, individual
results, HAR/state observations and distribution reports. Product downloads are
the separate `m5-*` artifacts. Failed attempts remain evidence. Packaging success
without service execution is insufficient. See [README.md](README.md) for the
archive names, runtime/linking bounds, startup, frontend, Docker and rollback
instructions. Artifact retention is finite; these are reviewable deliveries,
not a public release, registry publication or production cutover.

## Deliberate differences and limits

- Frontend discovery retains backend identity `Node` and protocol version
  `2.42.4` for the unchanged UI. `meta.rust` identifies the actual server.
  Script environment/capability reporting describes Rust and its real helpers.
- Node builtins/globals, CommonJS `require`, npm loading, native addons,
  `$substore.node` and proxy-app-only hosts are excluded. Such stored scripts
  survive import/export/restart and receive execution errors when invoked.
- RF01 fixes native Windows paths. RF02 returns a missing-token artifact-restore
  interface error and keeps the process alive. Their original failures and the
  narrowly approved reference patch are retained.
- RF03 FormData redirect and RF04 Blob/Fetch abort were part of the retired
  blanket Fetch experiment, removed at the user's request. The final AST has no
  free retained application callers for these globals. These experiments are
  classified outside the retained scope, never counted as executed passes.
- Runtime-specific compiler/minified constructor names, stacks and native
  malformed-MMDB diagnostic text are qualified. Public model identity,
  prototypes, fields/methods, expected errors, caller effects and recovery have
  executable cases; arbitrary SDK-private object graphs are not promised.
- This is finite source-derived compatibility, not exhaustive branch coverage
  or every cross-product of arbitrary inputs, providers, TLS/proxy settings and
  timezones. Required feature families and actual callers remain mandatory.

`SUB_STORE_MAX_HEADER_SIZE` keeps the existing positive decimal-prefix setting
and 32768-byte default for response header field/value bytes. The configured
limit applies to script HTTP and shared downloads; overflow reports
`Headers Overflow Error`. `SUB_STORE_BACKEND_DEFAULT_PROXY` remains a helper
fallback below explicit/settings proxy options. `SUB_STORE_X_POWERED_BY`
defaults to `Sub-Store`. Deprecated `SUB_STORE_BACKEND_CRON` and `SUB_STORE_CRON`
only log deprecation when the supported sync cron is absent; they do not start
a replacement job.
