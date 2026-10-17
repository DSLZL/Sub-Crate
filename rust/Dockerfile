FROM rust:1.98.0-bookworm@sha256:82150a52ec202c1b14d7817e14516c392bb7f5cfebd88f1ed531cb37ebd39922 AS build
WORKDIR /work
COPY rust/Cargo.toml rust/Cargo.lock rust/
COPY rust/src rust/src
RUN cargo build --manifest-path rust/Cargo.toml --locked --release --bin sub-store-rust

FROM debian:bookworm-slim@sha256:7c7b2c966bc9ee8cedfeef67e0e279108992c77681fa595db4a9d65c06ccc587
COPY --from=build /work/rust/target/release/sub-store-rust /usr/local/bin/sub-store-rust
COPY --from=build /etc/ssl/certs/ca-certificates.crt /etc/ssl/certs/ca-certificates.crt
COPY --from=build /usr/share/zoneinfo /usr/share/zoneinfo
RUN ! command -v node && ! command -v nodejs && ! command -v npm
WORKDIR /data
ENV SUB_STORE_DATA_BASE_PATH=/data SUB_STORE_BACKEND_API_HOST=0.0.0.0 SUB_STORE_BACKEND_API_PORT=3000
EXPOSE 3000
ENTRYPOINT ["/usr/local/bin/sub-store-rust"]
