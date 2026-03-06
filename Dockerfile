# syntax=docker/dockerfile:1

FROM rust:1.84-slim AS builder
WORKDIR /app

COPY Cargo.toml Cargo.lock ./
COPY src ./src

RUN cargo build --release --bin context-cutter-mcp

FROM gcr.io/distroless/cc-debian12:nonroot
WORKDIR /app

COPY --from=builder /app/target/release/context-cutter-mcp /usr/local/bin/context-cutter-mcp

ENTRYPOINT ["/usr/local/bin/context-cutter-mcp"]
