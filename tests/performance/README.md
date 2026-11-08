# Performance tests

Two workflows compare Sub-Crate with the original Sub-Store backend on the same runner. Both run only in GitHub Actions.

## Single-node comparison

[rust-single-node.yml](../../.github/workflows/rust-single-node.yml) converts the 48 public fixtures in `single-node.json`, one per supported protocol, and requires every response to match Sub-Store.

| Input | Revision |
| --- | --- |
| Sub-Store | `2888806251d90d45eabf2baf7a7fd4e8be2a4787` |
| Sub-Crate | The workflow's commit, or the release given in `release_tag` |
| Sub-Crate baseline | `9c19111bae2af2654caa339151e4eabd12b586bb` |

It measures parsing alone, first conversion, HTTP requests over new and kept-alive connections, load with 1 and 8 clients, memory, and per-stage profiles. Each measurement alternates backends over seven blocks. Set `diagnose_tail` to repeat the Hy2 and SS cases with detailed tail-latency profiles, or `inspect_run` to re-read an earlier run's results.

## Supplied subscription

[rust-performance.yml](../../.github/workflows/rust-performance.yml) imports a real subscription through the frontend and times parsing, health checks and production on both backends. The subscription comes from the `BENCHMARK_SUBSCRIPTION_B64` secret; its remote sources are captured once and replayed so provider latency does not affect the result.

| Input | Revision |
| --- | --- |
| Sub-Store | `2888806251d90d45eabf2baf7a7fd4e8be2a4787` |
| Sub-Crate | `af08b0649c4d2f2814fe38ce6943b1397f5e1c14` |
| Diagnostic baseline | `ee64ab9f7376a68c126950caee256bf3fb9c2b12` |
| Frontend | `4bd7b0acbe65466b933ccea14fc61dbebe036ead` |

A run has 35 independent pairs of jobs. Inputs move between jobs only as encrypted capsules, and only numeric metrics and hashes are published. The `diagnose_parse`, `diagnose_health` and `profile_pipeline` inputs add diagnostic cells.

## README chart

After each release, [benchmark-chart.yml](../../.github/workflows/benchmark-chart.yml) runs the single-node comparison on the latest release tag and commits `assets/benchmark.png` and `assets/benchmark-data.json`. Only a successful run on `main` for the current latest release can update the chart.

| Metric | Measure | Better |
| --- | --- | --- |
| Pure parsing | Median of the 48 fixtures, µs per call | Lower |
| Bulk conversion | 10,000 nodes to ClashMeta, Surge, QX and sing-box, ms | Lower |
| JavaScript pipeline | The same script filtering, deduplicating and renaming 10,000 nodes, ms | Lower |
| 8-client throughput | Requests per second | Higher |
| CPU efficiency | CPU ms per 10,000-node conversion | Lower |
| Settled RSS | Memory after one client finishes, MiB | Lower |

Sub-Store scores 100 on every metric. Sub-Crate's score is `100 × Sub-Store / Sub-Crate` for time and memory and `100 × Sub-Crate / Sub-Store` for throughput, so 200 means twice as fast or half the memory. The results describe these fixtures on a hosted Linux x64 runner, not every subscription or machine.

## Records

[HISTORY.md](HISTORY.md) keeps earlier measurements under their original revisions.
