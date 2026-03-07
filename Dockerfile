# syntax=docker/dockerfile:1

FROM rust:1.86-slim AS builder
WORKDIR /app

RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config \
    libssl-dev \
    && rm -rf /var/lib/apt/lists/*

COPY Cargo.toml Cargo.lock ./
COPY src ./src

RUN cargo build --release --bin context-cutter-mcp

FROM gcr.io/distroless/cc-debian12:nonroot
WORKDIR /app

COPY --from=builder /app/target/release/context-cutter-mcp /usr/local/bin/context-cutter-mcp

ENTRYPOINT ["/usr/local/bin/context-cutter-mcp"]
