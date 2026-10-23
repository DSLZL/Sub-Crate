# Sub-Store Rust

A standalone Rust backend for [Sub-Store](https://github.com/sub-store-org/Sub-Store), compatible with the unchanged official [Sub-Store frontend](https://github.com/sub-store-org/Sub-Store-Front-End).

[![Compatibility](https://github.com/DSLZL/Sub-Store/actions/workflows/rust-compat-native.yml/badge.svg?branch=main)](https://github.com/DSLZL/Sub-Store/actions/workflows/rust-compat-native.yml)

## Download and deployment

[GitHub Releases](https://github.com/DSLZL/Sub-Store/releases) provides Linux x64/arm64 and Windows x64 backend archives with `SHA256SUMS`. Multi-platform Docker images are published to `ghcr.io/dslzl/sub-store` using the release tag. Obtain the frontend separately from its official project; backend releases do not build or bundle it. Build manifests stay in Actions artifacts.

See [the deployment guide](rust/README.md) for archive selection, environment variables, frontend hosting, Docker startup, data migration and rollback. The runtime needs no Node installation. Use a copy of existing data when migrating, and run only one backend against a writable data directory.

## Compatibility

Subscription conversion, processing, resource management, storage and scheduling have Rust implementations. General JavaScript scripts run in the embedded rquickjs engine with the documented Sub-Store helpers. Node builtins, CommonJS `require`, npm loading and native addons are excluded; choose a script's general JS variant.

The frontend API and existing data formats are retained. The frontend still sees its required `Node` discriminator; Rust metadata identifies the actual implementation. See [the acceptance contract](rust/ACCEPTANCE.md) for the pinned baseline, tested scope and deliberate differences.

## Branches

- [`main`](https://github.com/DSLZL/Sub-Store/tree/main) contains the Rust backend and its Actions workflows.
- [`js`](https://github.com/DSLZL/Sub-Store/tree/js) follows the upstream JavaScript backend. The wei/pull GitHub App reads [.github/pull.yml](.github/pull.yml) on the default branch and synchronizes upstream `sub-store-org/Sub-Store:master` into `js` using `hardreset`. Local JS changes should use a separate branch.
- Compatibility tests restore the frozen Node reference from commit `9cc6b5fac966993364ae33da8797f446bbf7f571` on Actions runners and apply the approved reference fixes. The JavaScript application is not carried on `main`; updates to `js` do not change the comparison baseline.

## Development and validation

All product builds, tests, browser checks, Docker execution and benchmarks run in GitHub Actions. The compatibility workflow requires all six native, Docker and unchanged frontend jobs. [Performance records](tests/performance/README.md) retain the measured source revisions and limitations.

See the [cleanup inventory](rust/CLEANUP.md) for removed upstream application files and retained runtime, fixture and license dependencies.

## License and acknowledgements

[AGPL-3.0-only](LICENSE). This fork builds on the work of the [Sub-Store authors and contributors](https://github.com/sub-store-org/Sub-Store/graphs/contributors). Embedded utilities and delivered archives retain their dependency license notices.
