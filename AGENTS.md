# Agent Instructions

This file is the canonical project context for this repository.

If there is any conflict between this file and other documentation, prefer `AGENTS.md`.

## Project Source of Truth: `ContextCutter`

### 1. The Mission

Eliminate "JSON Bloat" in LLM agentic workflows. `ContextCutter` intercepts large API responses and replaces them with a lightweight "Teaser" and a "Lazy Handle," saving up to 90% of context-window tokens.

### 2. The Architecture

**Primary integration — Rust MCP server binary (`context-cutter-mcp`)**

- Written in Rust. Built with `cargo build --release --bin context-cutter-mcp`.
- Runs as a stdio MCP server using the `rmcp` crate.
- Exposes two MCP tools: `fetch_json_cutted` and `query_handle`.
- HTTP fetching via `ureq` (sync, spawned inside `tokio::task::spawn_blocking`).
- In-memory handle store via `DashMap` (global, shared across requests).
- Handle IDs are deterministic SHA-256 digests of canonicalized JSON (`hdl_<12hex>`).
- Teaser generation and JSONPath query execution live in `src/engine.rs` (pure Rust, no PyO3 dependency).
- PyO3 bindings live in `src/lib.rs` gated under the `python` Cargo feature.

**Secondary integration — Python SDK (optional)**

- PyO3/Maturin compiled extension. Build with `maturin develop --features python`.
- Wraps the same Rust engine functions from `src/engine.rs`.
- Provides `@lazy_handle` / `@lazy_tool` decorators, `query_handle`, `store_response`, `generate_teaser`, `query_path`, `generate_tool_manifest`.
- Optional custom stores: `InMemoryStore`, `RedisStore` (pure Python, for non-default flows).
- Default paths use the Rust-backed global DashMap store.

### 3. The Interaction Flow

**Via MCP (primary)**

1. LLM client calls `fetch_json_cutted(url, ...)`.
2. Rust binary fetches the URL, stores the JSON, returns `{handle_id, teaser}`.
3. LLM client calls `query_handle(handle_id, "$.path.to.data")` to retrieve specific fields.

**Via Python SDK (optional)**

1. Agent calls a `@lazy_handle`-decorated function.
2. `ContextCutter` intercepts the return value (the JSON payload).
3. Stores it and returns `{handle_id, teaser}`.
4. Agent calls `query_handle(handle_id, "$.path.to.data")` for specific fields.

### 4. Key Technical Requirements

- Deterministic `handle_id` generation: SHA-256 of canonicalized JSON, prefix `hdl_`, 12-char hex suffix.
- Storage agnostic: in-memory `DashMap` for dev/MCP; optional Redis for Python SDK prod flows.
- Schema Teaser: lists keys and array lengths without raw data values.
- MCP tool names are stable and must not change: `fetch_json_cutted`, `query_handle`.
- PyO3 feature is opt-in (`--features python`); the binary target has no PyO3 dependency.

### 5. Source Layout

```
src/
  engine.rs      # Pure Rust: compute_handle_id, engine_store, engine_teaser, engine_query
  parser.rs      # JSON parsing helpers (no PyO3)
  store.rs       # DashMap global store + PyO3 ContextStore (feature-gated)
  lib.rs         # PyO3 module entry point (feature-gated "python")
  bin/
    mcp.rs       # Rust MCP server binary
python/
  context_cutter/
    __init__.py  # Public API
    core.py      # query_handle, lazy_handle implementation
    store.py     # BaseStore, InMemoryStore, RedisStore
    tools.py     # generate_tool_manifest
    ...
tests/           # pytest test suite (Python SDK)
```
