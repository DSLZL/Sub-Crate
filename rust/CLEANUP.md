# Rust main cleanup inventory

Audited source: `d2d9403eefc77394f213196a79e6d1fdd49da303`.
The cleanup changes repository layout and CI reference preparation. Production
Rust source, utility snapshots, Cargo dependencies and runtime behavior are retained.

## Removed from main

Counts and sizes below use the original Git blobs, excluding checkout line-ending
conversion. The removed files total **183 files / 2,664,676 bytes**. Two banner
images had already been deleted locally when this audit began. Git history is
retained; deleting working files does not shrink the historical object database.

| Paths | Files | Reason and replacement |
| --- | ---: | --- |
| `backend/**` | 158 | Old JS application, Node host adapters, packaging/development tools, dependency manifests and original unit tests. CI restores the fixed original source when needed; the Rust service and Docker image do not load this directory. |
| `config/**` | 11 | Upstream proxy-app deployment modules and installation guide. These target the excluded in-app JS backend. CI restores them for the original Node unit tests. Rust proxy-format conversion remains supported. |
| `scripts/demo.js`, `scripts/fancy-characters.js`, `scripts/ip-flag.js`, `scripts/media-filter.js`, `scripts/revert.js`, `scripts/tls-fingerprint.js`, `scripts/udp-filter.js`, `scripts/vmess-ws-obfs-host.js` | 8 | Unreferenced upstream demonstration/user scripts. Original copies remain in repository history and the JS branch. |
| `assets/banners/MaruCode.jpg`, `assets/banners/Thordata.png`, `support.nodeseek.com_page_promotion_id=8.png` | 3 | Advertising images with no current README, build or test consumer. |
| `.github/workflows/star-history.yml` | 1 | Scheduled star-graph publication, unrelated to Rust delivery or acceptance. |
| `.node-version` | 1 | Root Node application toolchain selector. CI pins Node explicitly and restores the original file only for reference tests that read it. |
| `vs.code-workspace` | 1 | Empty single-folder editor workspace with no project settings or build consumer. |

## Relocated rather than removed

`scripts/ip-flag-node.js` is now `tests/compat/fixtures/ip-flag.js`.
Its original Git blob is `650ded1c0cfe90ea5bd0f9a6969bd47b645d95d4` (2,104 bytes).
The Rust contained probe compiles this input, so deleting it would break the
script-host regression check. The example, Docker probe inputs and evidence file
manifest now reference its fixture location. The CI bootstrap compares its
contents with the fixed original before building the probe.

## Updated consumers

- `.github/actions/node-reference/action.yml` restores `backend/`, `config/`,
  `.node-version` and the original IP-flag script from
  `9cc6b5fac966993364ae33da8797f446bbf7f571`. It applies only the existing
  `tests/compat/approved-reference-fixes.patch`, checks the exact full-blob Git
  diff and compares the relocated fixture. Every required compatibility runner
  uses this bootstrap before installing reference dependencies.
- Restored reference files are staged in the disposable runner index so the
  existing source guards can compare their blob identities with the original.
  No commit is created. Reference provenance reports this materialized source as
  a changed runner worktree; the production source revision remains the run's SHA.
- The compatibility workflow watches the new local action path. The standalone
  performance workflows already check out their own immutable Node reference;
  they do not require a root `backend/` on Rust main.
- Issue forms now list Rust's Linux, Windows and Docker targets instead of Node
  and proxy-app backend hosts. Target-client configuration remains a valid area.
- The root ignore file retains private data, generated CI/build files and local
  tool state, replacing unrelated Grunt/Bower/Next/Nuxt/webpack-era entries.

## Required files retained

| Paths | Why they remain |
| --- | --- |
| `rust/src/**` | Native application code and the JS bridge used by supported general scripts. A `.js` extension does not make a runtime helper obsolete. |
| `rust/vendor/**` | Locked generic utility snapshots, package/input hashes and license notices embedded in the runtime. |
| `rust/Cargo.toml`, `rust/Cargo.lock` | Reproducible Rust dependencies. For example, `reqwest::Url` is still used by production URL handling as well as the contained probe. |
| `rust/Dockerfile*`, `LICENSE`, acceptance/deployment docs | Production packaging, licensing and deployment contracts. |
| `tests/compat/**`, `rust/examples/**` | Active regression, source/license checks, unchanged-frontend acceptance and contained probe inputs. Actions requires these checked-in inputs; only generated outputs are disposable. |
| `tests/performance/**`, Rust workflows | Reproducible performance measurements and actual delivery validation. Earlier failed/result records retain their original revision boundaries. |
| `.github/pull.yml` | Upstream synchronization rule targets `js`, independently of the frozen comparison source. |

## Local-only files and disposable output

- `rust/target/`, `node_modules/`, `tests/compat/evidence/`,
  `tests/compat/.generated/`, `__pycache__/`, `.performance/`,
  `.compat-frontend/` and `.compat-delivery/` are generated output and may be
  removed locally. All product generation, builds and tests run in Actions.
- The audit found approximately 391 MiB under the regenerated `rust/target/`.
  Cache deletion was rejected by the execution tool with `blocked by policy`;
  this directory has not been removed. An active rust-analyzer process may
  regenerate editor cache after a manual cleanup.
- `1.md`, private `sub-store*.json` / `root.json`, `.trellis/`, assistant/editor
  settings and `.git/info/branch-migration-20261022/` are not application code.
  They remain local and ignored. User data, notes, accepted/failed historical
  records and migration recovery files are not classified as disposable build
  output and are not published by this cleanup.

## Validation

The unchanged six-job `Rust service and compatibility` gate must validate the
new layout on native Linux x64/arm64 and Windows x64, Docker x64/arm64 and the
unmodified frontend. Its source/patch checks, 826 original Node tests per native
target, Rust tests, service contracts and browser workflows remain required.
No local product build, generator, server, browser or test execution is needed.
