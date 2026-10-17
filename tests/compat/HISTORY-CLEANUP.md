# Rust rewrite history cleanup

The user requested an audit and removal of unrelated commits on 2026-10-17.
The original goal remains a complete standalone Rust backend, an unchanged
official frontend, existing data/business features and general JavaScript
scripts using the existing Sub-Store helper interfaces. Node runtime APIs
remain excluded. This cleanup changes implementation priority and removes
speculative experiments; it does not declare the rewrite complete.

## Result and boundary

- Original task branch: 69 commits after `9cc6b5fac966993364ae33da8797f446bbf7f571`.
- Keep the first 41 commits unchanged, through `23fd10229b164ad8833bc53b996e60448d7d60ad`.
- Drop 21 commits expanding standalone Web APIs without a demonstrated existing
  backend, unchanged-frontend or injected-helper dependency.
- Retain necessary fixes from seven later commits in four focused replacements.
- With this audit/documentation commit, the cleaned branch has 46 task commits.

Retain HTTP/state contracts, conversion/processor inventories, actual injected
DNS/MMDB/Gist/download/artifact helpers, age interoperability with native entropy,
JavaScript feasibility, the unchanged frontend browser harness, both Docker
architectures, approved RF01/RF02 fixes and native Node reference repairs.

Remove the standalone URL/DOMException/legacy-encoding and event/cancellation
conformance expansion; general Headers, Fetch/codecs/integrity/body ownership,
Blob/File/object URLs, FormData/multipart/stream/iterator uploads; and general
WebCrypto digest/UUID/HMAC/HKDF/PBKDF2 implementations and dependent tests.
The small Response/UTF-8/stream/random seam actually used by age remains.

The removed general APIs are not blanket requirements inferred from Node's
global object. Future support needs a concrete existing script/helper or
application dependency and belongs to that feature's implementation slice.
Do not turn exhaustive standard conformance into a prerequisite for the Rust
HTTP/state service. Unknown third-party script compatibility is not proven by
this cleanup or by the retained minimal host experiment.

Approved RF03 (multipart boundary on 307) and RF04 (Blob response cancellation
without process death) remain recorded behavior decisions. Their removed
Fetch-specific implementation is not counted as delivered; validate the
applicable behavior when implementing the actual Rust business network path.
Historical Node failure evidence and user approvals remain preserved locally.

## Necessary later fixes

- `9dabb9b6d708d93cce609c208cd0618965609979`: `de3833f1`.
- `0c98f6f587081b8f42f3c92918cb75dcaaac8950`: `a20656f3`.
- `1fc464f0fe41815c5552e71a1b177d37f0b1c86b`: `08fad139`, `5e1e9fb0`, `26557e29`, `b5f3651b`.
- `e9961df5091fcc4e2085fc1f52de6099d7f49005`: `d8dcd06f`.

Reference result flushing, collection edit-button navigation and release
execution, DNS dual-stack hostname fallback/error causes, and complete HTTP
request writes are retained independently of the removed Web API stack.
The DNS diagnostics/style changes are combined with their functional fix.

## Commit-by-commit audit

`keep` retains the original commit. `replace` retains its relevant behavior in
the mapped focused commit. `drop` excludes the commit and its experimental code
from the cleaned task branch. These decisions were based on changed files,
existing helper injection, backend/frontend requirements and patch dependencies.

| Original commit | Decision | Subject and reason |
| --- | --- | --- |
| `7170d33c192ed402e068625e7eb5f9152ab5f38f` | keep | test: establish Rust rewrite native compatibility baseline. Existing HTTP/state/frontend or injected business helper evidence (R2-R7) |
| `1a7991da3eb54ebd6d1ae9933918d308f5cf15de` | keep | test: verify frontend workflows and script backup restart. Existing HTTP/state/frontend or injected business helper evidence (R2-R7) |
| `7b0a6e00a416cb4da257405737902766cbe619ff` | keep | test: expand resource and merged share compatibility cases. Existing HTTP/state/frontend or injected business helper evidence (R2-R7) |
| `7d9ca98732d57ba055ce9b535fbe122ff550b057` | keep | test: bound script execution and verify share frontend flows. Existing HTTP/state/frontend or injected business helper evidence (R2-R7) |
| `6225bd35868bdba925d4501ef1151980c8803695` | keep | test: freeze proxy rule and processor conversion contracts. Existing HTTP/state/frontend or injected business helper evidence (R2-R7) |
| `3eeed5e6597d59012485a5f301715ef33ea64978` | keep | test: freeze resource contracts and collection frontend flows. Existing HTTP/state/frontend or injected business helper evidence (R2-R7) |
| `4cbd8baf562a27631d5168c62181fc866006722e` | keep | test: cover unchanged frontend file archive and restore. Existing HTTP/state/frontend or injected business helper evidence (R2-R7) |
| `36d645d970f23e765be65e9abe15679909a6f781` | keep | test: freeze cache cors and migration contracts. Existing HTTP/state/frontend or injected business helper evidence (R2-R7) |
| `53ff13f074419b6d8146efadedc8910d9582f5c3` | keep | test: verify controlled gist sync and backup contracts. Existing HTTP/state/frontend or injected business helper evidence (R2-R7) |
| `6a7b294b32b7b4d94c1436836050c6d4f8d1bf35` | keep | test: verify unchanged frontend log workflows. Existing HTTP/state/frontend or injected business helper evidence (R2-R7) |
| `f2245a519803b0bff1659c8707dc222d2c696ec4` | keep | test: freeze controlled node information lookups. Existing HTTP/state/frontend or injected business helper evidence (R2-R7) |
| `c4954899a8dcebe8fb54fcf9416257104d63d5be` | keep | test: verify unchanged frontend backup round trips. Existing HTTP/state/frontend or injected business helper evidence (R2-R7) |
| `47cbef29d29f99cdc3d4f4c3a4550b4cb9ea0f5f` | keep | test: resolve computed registry names and validate UI references. Existing HTTP/state/frontend or injected business helper evidence (R2-R7) |
| `7f1942f6900c6e0f18c0377a83397516740dc711` | keep | fix: read native paths and contain artifact restore errors. Existing HTTP/state/frontend or injected business helper evidence (R2-R7) |
| `8cda8e7c4c9a7d3f90eadf1392660d4caf4b5114` | keep | test: preserve unit progress on native process failures. Existing HTTP/state/frontend or injected business helper evidence (R2-R7) |
| `45dd4405389bf146c59a59d9f90a7d6dc4d22bf5` | keep | test: compare Rust script host against Node contracts. Existing HTTP/state/frontend or injected business helper evidence (R2-R7) |
| `8436bbb0cc7c2a796ae895c97bf9cd3e3a797fb6` | keep | test: verify script processors across Node and Rust. Existing HTTP/state/frontend or injected business helper evidence (R2-R7) |
| `57ba804aac101cbb3a508945c65d5f428892dd1b` | keep | test: verify public script helpers and flow bridge. Existing HTTP/state/frontend or injected business helper evidence (R2-R7) |
| `33bd8efc5c334723f403a809928ce1efd6f0184e` | keep | ci: run frontend and Docker compatibility baselines. Existing HTTP/state/frontend or injected business helper evidence (R2-R7) |
| `7664d91bfcbb904410af9fcbd4d689e6b4b6ca16` | keep | fix: resolve container helpers and await language menus. Existing HTTP/state/frontend or injected business helper evidence (R2-R7) |
| `acbb2064b05d65c4fda1ed1c1c6623bacdd549a6` | keep | test: match frontend controls by exact text. Existing HTTP/state/frontend or injected business helper evidence (R2-R7) |
| `419bdc67cc18db63e4fcaddb30ff69bb538bf0ac` | keep | fix: select visible frontend controls and await editor data. Existing HTTP/state/frontend or injected business helper evidence (R2-R7) |
| `734da501dbea6bd0daa5daab3b18e999d1a6df8e` | keep | fix: wait for frontend backdrop dismissal. Existing HTTP/state/frontend or injected business helper evidence (R2-R7) |
| `5864480fff99117b8346885f4d30d00c8f7775f2` | keep | fix: require frontend menus inside the viewport. Existing HTTP/state/frontend or injected business helper evidence (R2-R7) |
| `c36a0cc6ffb7ca3fc3ab2b08f769ee2abf6e14e6` | keep | fix: await rendered frontend controls before interaction. Existing HTTP/state/frontend or injected business helper evidence (R2-R7) |
| `d2982c209f90574e294c81dfec0fff6c51cbda1d` | keep | fix: center collection controls before native clicks. Existing HTTP/state/frontend or injected business helper evidence (R2-R7) |
| `10a8136d28fe67852b98f1c5d0186c0b5d595b3e` | keep | test: bridge DoH and custom DNS helpers in Rust probe. Existing HTTP/state/frontend or injected business helper evidence (R2-R7) |
| `d1b3108255967cd5eddcc4f387e08aff82e48ee1` | keep | test: bridge native UDP TCP and TLS DNS in Rust probe. Existing HTTP/state/frontend or injected business helper evidence (R2-R7) |
| `3404f30d32e3ddbdd6d4feb20e1aaf21670b75b1` | keep | test: capture native TLS handshake timing and diagnostics. Existing HTTP/state/frontend or injected business helper evidence (R2-R7) |
| `21e7b45c6e6933166a8d8f34dbcda89d4f38933c` | keep | test: isolate DNS inactivity checks from hostname selection. Existing HTTP/state/frontend or injected business helper evidence (R2-R7) |
| `104f40915d9fa49d916eaaf6f2ac37d1c76a7f55` | keep | test: wait for initial settings before editing log retention. Existing HTTP/state/frontend or injected business helper evidence (R2-R7) |
| `db16096270952cd66b0f13b56b7c9dccd097087f` | keep | test: validate native MMDB script helpers with pinned databases. Existing HTTP/state/frontend or injected business helper evidence (R2-R7) |
| `7fb75b3cee7bf72e51e05dfc485d689fd81091db` | keep | fix: preserve native MMDB TypeError identity. Existing HTTP/state/frontend or injected business helper evidence (R2-R7) |
| `7a0ab1339192b7a63d97b80434bd23443c8ee5f2` | keep | test: retain synchronous processor progress on native crashes. Existing HTTP/state/frontend or injected business helper evidence (R2-R7) |
| `1b03ba89e371ca1292a307d25f9503219058ecc6` | keep | test: prove public Gist helpers over native Rust HTTP. Existing HTTP/state/frontend or injected business helper evidence (R2-R7) |
| `f2c0c1a7ee54fb022debd0e6b0fb77c227b8ac15` | keep | test: prove age crypto interoperability in Rust script host. Existing HTTP/state/frontend or injected business helper evidence (R2-R7) |
| `8c909d5c8a876f2cc742da40f4b85a54f611b5f1` | keep | test: prove download helpers over native Rust file I/O. Existing HTTP/state/frontend or injected business helper evidence (R2-R7) |
| `82c236c567cf530f55391a3be2022308d687dc20` | keep | test: prove proxy conversion helpers in Rust script host. Existing HTTP/state/frontend or injected business helper evidence (R2-R7) |
| `10682f3693d82594a4f5623f994d368d8a02c794` | keep | fix: initialize native entropy before loading script helpers. Existing HTTP/state/frontend or injected business helper evidence (R2-R7) |
| `57927281047fc4ed2792ce38995f7a4258455322` | keep | test: prove real artifact generation in Rust script host. Existing HTTP/state/frontend or injected business helper evidence (R2-R7) |
| `23fd10229b164ad8833bc53b996e60448d7d60ad` | keep | fix(ci): use repaired Node oracle for Windows TCP crashes. Existing HTTP/state/frontend or injected business helper evidence (R2-R7) |
| `2abe778aac6c54677b58c0a7f7b3093a7bbef792` | drop | test: verify Web URL APIs in Rust script host. Standalone general Web API expansion without a demonstrated backend/frontend/injected-helper dependency |
| `de3833f1aa4851088e4d73faab9774c4f2495cfc` | replace | fix(test): flush complete Node oracle result records. Complete source-backed reference results without truncated stdout; replacement 9dabb9b6 |
| `24f8dae62d0db3e2261e1b78091f6adcea434055` | drop | perf(test): reduce URL helper initialization. Standalone general Web API expansion without a demonstrated backend/frontend/injected-helper dependency |
| `8041be4e8b7e2ea77caa378b8e4749f7ba7e124b` | drop | test: verify cancellation primitives in Rust script host. Standalone general Web API expansion without a demonstrated backend/frontend/injected-helper dependency |
| `a20656f320a25aa3798039bb7035a53f4110b7c0` | replace | test: validate release probes and existing frontend navigation. Existing collection edit navigation and actual release binary verification; replacement 0c98f6f5 |
| `898ddb39adfce69813f4c020e318477484a4fd69` | drop | test: verify Headers compatibility in Rust script host. Standalone general Web API expansion without a demonstrated backend/frontend/injected-helper dependency |
| `7ec2e7af5db1bd06c8b3c63974666642d5c5ab00` | drop | perf(test): reuse the contained HTTP client. Performance workaround for expanded probe; later DNS commit already reversed its source behavior |
| `08fad139a4c4516a3aa132bd366c289e8432f8ad` | replace | test: record native DNS connection latency. Connection diagnostics supporting actual public DNS helper regression; replacement 1fc464f0 |
| `5e1e9fb07f7c03b706bc031fa6d384e91aedf8ce` | replace | style(test): format native DNS timing records. Formatting of retained DNS connection diagnostics; replacement 1fc464f0 |
| `26557e292f6ab469b1690318dea416fee3cbf0a0` | replace | fix: use dual-stack fallback for native DNS connections. IPv4/IPv6 hostname fallback in existing public DNS helper; replacement 1fc464f0 |
| `b5f3651bba84ecf6ac9184a780c9faac88258311` | replace | fix: preserve DNS connector error diagnostics. Preserve DNS connection error causes after hostname fallback; replacement 1fc464f0 |
| `b91c82ce3570e1d5b44078797b14c3781a9272c8` | drop | feat: add native Fetch core to contained script host. Standalone general Web API expansion without a demonstrated backend/frontend/injected-helper dependency |
| `9d4e5f36a3302d7def200eac6716458982900018` | drop | test: avoid Fetch frame boundary assumptions. Standalone general Web API expansion without a demonstrated backend/frontend/injected-helper dependency |
| `9e0017bd9ed7762ca6a91e86e841bab824008551` | drop | feat: decode compressed Fetch responses natively. Standalone general Web API expansion without a demonstrated backend/frontend/injected-helper dependency |
| `17585abfb32dad5e6eb75809f9be7e8c6c7b9c67` | drop | test: stabilize streamed gzip fixture headers. Standalone general Web API expansion without a demonstrated backend/frontend/injected-helper dependency |
| `b8a45279ce59005f50f5bc6aa8e9e3164bad28b0` | drop | feat: verify Fetch response integrity natively. Standalone general Web API expansion without a demonstrated backend/frontend/injected-helper dependency |
| `d8dcd06fcfc3df4da015c7c6b88315ea30e217ab` | replace | test: send complete HTTP contract requests together. One complete request write avoids Windows early CORS close races; replacement e9961df5 |
| `4e6894a5d1527749e52b93b05b8d8257d93d7de9` | drop | feat: support Blob and File in Fetch scripts. Standalone general Web API expansion without a demonstrated backend/frontend/injected-helper dependency |
| `939e6d3eeadfb7e0947f99362562a1e6420fbc11` | drop | feat: support FormData and multipart Fetch bodies. Standalone general Web API expansion without a demonstrated backend/frontend/injected-helper dependency |
| `e75d356eb60409f9013b4bba4f45091dd3d742b8` | drop | fix: align multipart parsing with pinned Node behavior. Standalone general Web API expansion without a demonstrated backend/frontend/injected-helper dependency |
| `59f29ded77016d0433e95a6bdd449e554e01b193` | drop | feat: support Blob object URLs and explicit Fetch defect regressions. Remove object-URL/Fetch experiment; keep approved RF03/RF04 behavior decisions pending business-service integration |
| `820536ce9d2dd2bf6b88b6a7d0af2956a6b1f99b` | drop | feat: stream Fetch uploads through bounded native frames. Standalone general Web API expansion without a demonstrated backend/frontend/injected-helper dependency |
| `a27925218375eff1b9b29cceb5e5b775b02253ba` | drop | feat: support iterable Fetch bodies and preserve request ownership. Standalone general Web API expansion without a demonstrated backend/frontend/injected-helper dependency |
| `e3d8b935b8357ff12baad6eb732c470969b20abb` | drop | feat: add native WebCrypto digests and system UUIDs. Standalone general Web API expansion without a demonstrated backend/frontend/injected-helper dependency |
| `eadbc27bf247c90fb7b88e3a39bc5d302b9f58e6` | drop | feat: add HMAC keys and native signature verification. Standalone general Web API expansion without a demonstrated backend/frontend/injected-helper dependency |
| `6b5888a4f46481242f72c560700bebe7d03dbeff` | drop | feat: add native HKDF derivation and derived HMAC keys. Standalone general Web API expansion without a demonstrated backend/frontend/injected-helper dependency |
| `17c8f156e7f5e8b2e6b155b3d0afaf6067b795bb` | drop | fix: keep crypto imports and native callbacks private. Standalone general Web API expansion without a demonstrated backend/frontend/injected-helper dependency |
| `9a57d1a58e87814d80d17483108b76b2742a4d20` | drop | feat: add native PBKDF2 derivation for script keys. Standalone general Web API expansion without a demonstrated backend/frontend/injected-helper dependency |

## Validation and recovery

Before publishing the rewritten branch, require a clean scoped index, unchanged
protected user files, unchanged original backend/reference patch and frontend
source, passing runner checks and the retained native/Docker/browser CI jobs.
Existing-data, production Rust service and frontend-to-Rust acceptance remain
open. Old CI runs describe the old commits and do not validate the new history.

The pre-cleanup tip is `9a57d1a58e87814d80d17483108b76b2742a4d20`. It is retained
locally as `backup/rust-server-parity-before-cleanup-20261017`, together with a
Git bundle, all 69 original binary patches and a Trellis snapshot. These
recovery artifacts are outside the cleaned branch. Publish with an explicit
lease against that old remote tip; do not overwrite any concurrent remote work.

The next implementation milestone is the Rust HTTP/state service. Retained
contracts are a regression oracle, not evidence that this service exists.
