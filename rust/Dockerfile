FROM rust:1.98.0-bookworm@sha256:82150a52ec202c1b14d7817e14516c392bb7f5cfebd88f1ed531cb37ebd39922 AS build
WORKDIR /work
COPY rust/Cargo.toml rust/Cargo.lock rust/
COPY rust/src rust/src
COPY rust/vendor rust/vendor
COPY LICENSE LICENSE
COPY rust/examples/collect_notices.rs rust/examples/
COPY tests/src/notices.rs tests/src/
COPY tests/compat/fixtures/licenses/rquickjs-v0.14.0-LICENSE tests/compat/fixtures/licenses/
RUN cargo build --manifest-path rust/Cargo.toml --locked --release --bin sub-store-rust
RUN mkdir /delivery \
    && rustc -vV > /delivery/toolchain.txt \
    && rustc -vV | sed -n 's/^host: //p' > /delivery/target.txt \
    && cargo metadata --manifest-path rust/Cargo.toml --locked --format-version 1 --filter-platform "$(cat /delivery/target.txt)" > /tmp/cargo-metadata.json \
    && cargo run --manifest-path rust/Cargo.toml --locked --release --example collect_notices -- /tmp/cargo-metadata.json /delivery "$(rustc --print sysroot)"

FROM debian:bookworm-slim@sha256:7c7b2c966bc9ee8cedfeef67e0e279108992c77681fa595db4a9d65c06ccc587
COPY --from=build /work/rust/target/release/sub-store-rust /usr/local/bin/sub-store-rust
COPY --from=build /etc/ssl/certs/ca-certificates.crt /etc/ssl/certs/ca-certificates.crt
COPY --from=build /usr/share/zoneinfo /usr/share/zoneinfo
COPY --from=build /delivery /usr/share/licenses/sub-store/
RUN ! command -v node && ! command -v nodejs && ! command -v npm
WORKDIR /data
ENV SUB_STORE_DATA_BASE_PATH=/data SUB_STORE_BACKEND_API_HOST=0.0.0.0 SUB_STORE_BACKEND_API_PORT=3000
EXPOSE 3000
ENTRYPOINT ["/usr/local/bin/sub-store-rust"]
