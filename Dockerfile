# Multi-arch image: amd64 runs on Debian bookworm-slim, arm64 on Ubuntu 24.04.
# The builder is pinned to bookworm (glibc 2.36) so the binary also runs on
# Ubuntu 24.04 (glibc 2.39).
ARG TARGETARCH

FROM rust:1-bookworm AS builder

WORKDIR /app

COPY Cargo.toml Cargo.lock ./
COPY src ./src
# The web interface files are embedded into the binary at compile time
COPY web ./web

RUN cargo build --release

FROM debian:bookworm-slim AS runtime-amd64
FROM ubuntu:24.04 AS runtime-arm64

FROM runtime-${TARGETARCH}

WORKDIR /app

COPY --from=builder /app/target/release/RustFeed /usr/local/bin/RustFeed

# Configuration, account and articles (managed from the web interface)
VOLUME /app/data

# Web interface
EXPOSE 3060

CMD ["RustFeed"]
