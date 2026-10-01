FROM rust:1 AS builder

WORKDIR /app

COPY Cargo.toml Cargo.lock ./
COPY src ./src
# The web interface files are embedded into the binary at compile time
COPY web ./web

RUN cargo build --release

FROM debian:bookworm-slim

WORKDIR /app

COPY --from=builder /app/target/release/RustFeed /usr/local/bin/RustFeed

# Web interface to manage feeds
EXPOSE 3060

CMD ["RustFeed"]
