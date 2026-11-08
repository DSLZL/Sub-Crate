# Compatibility fixtures

These files let the harness compare Sub-Crate with the original Sub-Store. The pass criteria are in [docs/ACCEPTANCE.md](../../docs/ACCEPTANCE.md).

| Path | Contents |
| --- | --- |
| `manifest.json` | Inventory of routes, formats, processors and environment options that must be covered |
| `fixtures/http-cases.json`, `fixtures/script-cases.json` | Request sequences and script cases run against both backends |
| `frontend-contract-map.json` | Frontend callers and the API paths they use |
| `fixtures/` | Certificates, MMDB databases, age vectors, licenses and script probes |
| `approved-reference-fixes.patch` | The only changes allowed on the frozen backend (RF01, RF02) |
| `*-reference.cjs`, `*-adapter.cjs`, `register.cjs` | Run the frozen backend's own helpers to produce reference results |
| `build-helpers.cjs`, `build-native-utilities.cjs` | Generate the test helper bundle and the pinned utilities in `crates/sub-crate/vendor` |
| `Probe.Dockerfile` | Container for the script probe |
| `package.json`, `pnpm-lock.yaml` | Reference-only JavaScript dependencies |
| Other `*.cjs` | Small helpers the workflows call, such as the frontend source inventory |

The frozen backend (`9cc6b5fac966993364ae33da8797f446bbf7f571`) is never stored here. The [node-reference](../../.github/actions/node-reference/action.yml) action restores it on the runner and checks that only the approved patch was applied. Generated bundles, installed packages and evidence are created on the runner and are not committed.

[HISTORY.md](HISTORY.md) is the unchanged record of earlier experiments.
