# MCP Proxy Mode — Design Spec

**Date:** 2026-04-13  
**Status:** Approved

## Problem

`context-cutter-mcp` currently reduces token bloat only when Claude explicitly calls
`fetch_json_cutted`. MCPs that return large JSON payloads natively (e.g. the ClickUp
staging MCP at `https://mcp.clickup-stg.com/mcp`) dump their full response straight
into Claude's context window — there is no interception point.

## Goal

Allow `context-cutter-mcp` to act as a **transparent stdio proxy** in front of any
HTTP-based MCP server, intercepting large tool responses before they reach Claude's
context and replacing them with a handle + preview teaser.

## Non-Goals

- stdio-to-stdio proxying (HTTP target only for now)
- Persistent handle storage across process restarts (in-memory is sufficient)
- Modifying upstream tool schemas or descriptions

---

## Invocation

Normal mode (unchanged):
```json
{ "command": "npx", "args": ["-y", "context-cutter-mcp"] }
```

Proxy mode (replaces the upstream MCP entry in the MCP config):
```json
{
  "command": "npx",
  "args": ["-y", "context-cutter-mcp", "--proxy", "https://mcp.clickup-stg.com/mcp"]
}
```

With auth headers (repeatable):
```json
{
  "command": "npx",
  "args": [
    "-y", "context-cutter-mcp",
    "--proxy", "https://mcp.clickup-stg.com/mcp",
    "--proxy-header", "Authorization: Bearer $TOKEN"
  ]
}
```

With custom threshold (default 2048 bytes):
```json
{
  "args": ["...", "--proxy-threshold", "4096"]
}
```

---

## Architecture

```
Claude Code  ←stdio JSON-RPC→  context-cutter-mcp --proxy <url>  ←HTTP JSON-RPC→  upstream MCP
```

The proxy is a **stdio MCP server** (same transport as today). It speaks MCP to Claude
Code over stdin/stdout, and speaks MCP-over-HTTP to the upstream server using `ureq`
(already a dependency).

---

## Startup Sequence (proxy mode)

1. Parse CLI args; detect `--proxy <url>`.
2. Validate URL (must be `https://`).
3. POST `initialize` to upstream → confirm capabilities.
4. POST `tools/list` to upstream → store tool list in memory.
5. Start stdio MCP server (`ProxyServer`) advertising upstream tools + `query_handle`.

---

## Per-Request Flow

```
call_tool(name, args)
  │
  ├─ name == "query_handle"
  │     └─ handled locally via engine_query()  →  return result
  │
  └─ any other tool
        └─ POST tools/call to upstream (ureq, spawn_blocking)
              │
              ├─ response size < threshold  →  pass through as-is
              │
              └─ response size ≥ threshold
                    ├─ engine_store(response_json)   →  handle_id
                    ├─ engine_teaser(handle_id)      →  teaser
                    └─ return handle+preview to Claude
```

---

## Intercepted Response Format

When a response meets or exceeds the threshold, Claude receives:

```
[context-cutter] Response stored (12.4 KB → handle: hdl_a1b2c3d4e5f6)

Preview:
  id: "86cxyz123"
  name: "Fix login bug in staging"
  status: "in progress"
  assignees: [2 items]
  description: "Long description text..." (truncated)
  custom_fields: [8 items]
  date_created: "1712345678000"

Call query_handle("hdl_a1b2c3d4e5f6", "$.field") to extract specific fields.
```

Preview rendering rules (top-level keys only):
- **String** → value as-is, truncated at 80 chars with `(truncated)` suffix
- **Array** → `[N items]` — no values shown
- **Object** → `{N keys}` — no values shown
- **Number / bool / null** → shown as-is

---

## New Components

### `src/bin/mcp.rs` additions

**`Args` struct (clap)**
```rust
struct Args {
    proxy: Option<String>,           // --proxy <url>
    proxy_threshold: usize,          // --proxy-threshold <bytes>, default 2048
    proxy_header: Vec<String>,       // --proxy-header "K: V" (repeatable)
}
```

**`ProxyServer` struct**
```rust
struct ProxyServer {
    upstream_url: String,
    threshold: usize,
    extra_headers: Vec<(String, String)>,
    upstream_tools: Vec<rmcp::model::Tool>,  // populated at startup
}
```

Implements `ServerHandler` directly (no `#[tool_router]` macro — tools are dynamic).

**`upstream_call(url, headers, method, params) -> String`**  
Synchronous `ureq` POST helper that sends a JSON-RPC request to the upstream and
returns the raw `result` field as a JSON string. Runs inside `spawn_blocking`.
Reuses the existing `read_response_with_limit` pattern.

### `Cargo.toml` additions
- `clap` with `derive` feature

No new `rmcp` features required — HTTP upstream calls are made via `ureq` manually.

---

## Error Handling

| Scenario | Behaviour |
|---|---|
| Upstream unreachable at startup | `eprintln` + `exit(1)` |
| `tools/list` returns empty | Start anyway; Claude sees only `query_handle` |
| Upstream tool call fails (non-2xx) | Forward error text as-is to Claude (no interception) |
| Response is not valid JSON | Forward raw text as-is (no interception) |
| Response exceeds `CONTEXT_CUTTER_MAX_PAYLOAD_BYTES` | Return payload-too-large error |
| `--proxy-header` malformed (no `: `) | `eprintln` + `exit(1)` at startup |

---

## Testing

- Unit: `upstream_call` helper with a mock HTTP server
- Unit: interception threshold logic (below / at / above boundary)
- Unit: preview rendering for each value type
- Integration: proxy against a local test MCP server that returns a large JSON fixture
- Existing tests: must continue to pass unmodified (normal mode untouched)
