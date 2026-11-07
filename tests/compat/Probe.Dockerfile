FROM rust:1.98.0-bookworm@sha256:82150a52ec202c1b14d7817e14516c392bb7f5cfebd88f1ed531cb37ebd39922 AS build
WORKDIR /work
COPY Cargo.toml Cargo.lock ./
COPY tests/Cargo.toml tests/
COPY crates/sub-crate/Cargo.toml crates/sub-crate/
COPY crates/sub-crate/examples crates/sub-crate/examples
COPY crates/sub-crate/src/script/io.rs crates/sub-crate/src/script/transport.rs crates/sub-crate/src/script/primitives.js crates/sub-crate/src/script/
COPY crates/sub-crate/src/dns_transport.rs crates/sub-crate/src/mmdb.rs crates/sub-crate/src/
COPY tests/compat/fixtures tests/compat/fixtures
COPY tests/compat/.generated/script-helpers.js tests/compat/.generated/script-helpers.js
COPY tests/compat/.generated/script-helper-licenses.txt tests/compat/.generated/script-helper-licenses.txt
RUN cargo build -p sub-crate --locked --release --example script_probe

# Contained compatibility probe; the production image uses the root Dockerfile.
FROM debian:bookworm-slim@sha256:7c7b2c966bc9ee8cedfeef67e0e279108992c77681fa595db4a9d65c06ccc587
WORKDIR /work/crates/sub-crate
COPY --from=build /work/target/release/examples/script_probe /usr/local/bin/script_probe
COPY --from=build /work/tests/compat/.generated/script-helpers.js /work/tests/compat/.generated/script-helpers.js
COPY --from=build /work/tests/compat/.generated/script-helper-licenses.txt /usr/local/share/doc/sub-store-compat/script-helper-licenses.txt
COPY --from=build /etc/ssl/certs/ca-certificates.crt /etc/ssl/certs/ca-certificates.crt
RUN test -r /work/crates/sub-crate/../../tests/compat/.generated/script-helpers.js \
    && ! command -v node && ! command -v nodejs && ! command -v npm
ENTRYPOINT ["/usr/local/bin/script_probe"]
