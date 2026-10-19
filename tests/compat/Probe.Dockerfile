FROM rust:1.98.0-bookworm@sha256:82150a52ec202c1b14d7817e14516c392bb7f5cfebd88f1ed531cb37ebd39922 AS build
WORKDIR /work
COPY rust/Cargo.toml rust/Cargo.lock rust/
COPY rust/examples rust/examples
COPY rust/src/script_io.rs rust/src/script-primitives.js rust/src/
COPY scripts/ip-flag-node.js scripts/ip-flag-node.js
COPY tests/compat/fixtures tests/compat/fixtures
COPY tests/compat/.generated/script-helpers.js tests/compat/.generated/script-helpers.js
COPY tests/compat/.generated/script-helper-licenses.txt tests/compat/.generated/script-helper-licenses.txt
RUN cargo build --manifest-path rust/Cargo.toml --locked --release --example script_probe

# shortcut: this packages the M1 experiment; replace it with the service image at M5.
FROM debian:bookworm-slim@sha256:7c7b2c966bc9ee8cedfeef67e0e279108992c77681fa595db4a9d65c06ccc587
WORKDIR /work/rust
COPY --from=build /work/rust/target/release/examples/script_probe /usr/local/bin/script_probe
COPY --from=build /work/tests/compat/.generated/script-helpers.js /work/tests/compat/.generated/script-helpers.js
COPY --from=build /work/tests/compat/.generated/script-helper-licenses.txt /usr/local/share/doc/sub-store-compat/script-helper-licenses.txt
COPY --from=build /etc/ssl/certs/ca-certificates.crt /etc/ssl/certs/ca-certificates.crt
RUN test -r /work/rust/../tests/compat/.generated/script-helpers.js \
    && ! command -v node && ! command -v nodejs && ! command -v npm
ENTRYPOINT ["/usr/local/bin/script_probe"]
