# ContextCutter

[![Tests](https://github.com/nikitaclicks/context-cutter/actions/workflows/tests.yml/badge.svg)](https://github.com/nikitaclicks/context-cutter/actions/workflows/tests.yml)
[![codecov](https://codecov.io/gh/nikitaclicks/context-cutter/branch/main/graph/badge.svg)](https://codecov.io/gh/nikitaclicks/context-cutter)

**Stop feeding entire API responses to your LLM. Give it a handle instead.**

When an agent calls a REST API, the full JSON response lands in the context window — even if the agent only needs one field. On a 500-item list, that's 97 KB of tokens consumed to read two values. ContextCutter intercepts those responses before they reach the model, stores them in a fast in-memory store, and returns a compact structural summary (a *teaser*) plus a deterministic handle ID. The agent then queries only the fields it actually needs.

The result: **86–99% fewer tokens spent on API responses** in typical agent workflows.

## How it works

```
┌─────────┐   fetch_json_cutted(url)   ┌──────────────────┐   HTTP GET   ┌─────────────┐
│  Agent  │ ────────────────────────► │  ContextCutter   │ ───────────► │  Remote API │
│  (LLM)  │                           │   MCP Server     │ ◄─────────── │             │
│         │ ◄──────────────────────── │  (Rust binary)   │   JSON blob  └─────────────┘
│         │   { handle_id, teaser }   │                  │
│         │                           │  DashMap store   │
│         │   query_handle(id, path)  │  (in-memory)     │
│         │ ────────────────────────► │                  │
│         │ ◄──────────────────────── │                  │
└─────────┘   "$.users[0].email"      └──────────────────┘
              → "alice@example.com"
```

**Step 1 — fetch:** The agent calls `fetch_json_cutted(url)`. The server fetches the URL, stores the full JSON payload, and responds with a *teaser* (structural summary) and a `handle_id`.

**Step 2 — query:** The agent inspects the teaser to understand the shape of the data, then calls `query_handle(handle_id, "$.path.to.field")` to retrieve only what it needs.

The full payload never enters the context window.

## Token savings

Measured against realistic API response shapes:

| Response type              | Full payload | Teaser returned | Tokens saved |
|----------------------------|-------------:|----------------:|-------------:|
| 10-item paginated list     |   2,005 chars|        287 chars|        **86%**|
| 50-item repo listing       |  11,576 chars|        268 chars|        **98%**|
| 100-item event stream      |  21,005 chars|        283 chars|        **99%**|
| 500-item batch export      |  97,465 chars|        261 chars|       **100%**|
| Deep nested config blob    |  19,943 chars|        341 chars|        **98%**|

> Teaser size stays roughly constant (~250–350 chars) regardless of payload size, because it describes *structure*, not values.

## Quickstart

The fastest way to try ContextCutter is with `npx` — no install required:

```bash
npx context-cutter-mcp
```

Add it to your agent client in under a minute:

**OpenCode** (`~/.config/opencode/config.json`):

```json
{
  "mcp": {
    "context-cutter": {
      "type": "local",
      "command": "npx",
      "args": ["-y", "context-cutter-mcp"]
    }
  }
}
```

**Claude Desktop** (`~/Library/Application Support/Claude/claude_desktop_config.json`):

```json
{
  "mcpServers": {
    "context-cutter": {
      "command": "npx",
      "args": ["-y", "context-cutter-mcp"]
    }
  }
}
```

Once connected, ContextCutter registers two tools with your agent automatically. No prompting or configuration needed — the server describes itself via MCP.

See [`examples/`](examples/) for Cursor, VS Code, OpenAI Agents SDK, and LangChain configs.

## MCP tool reference

### `fetch_json_cutted`

Fetches a URL, stores the JSON response, and returns a structural teaser.

| Parameter        | Type    | Default | Description                                |
|------------------|---------|---------|--------------------------------------------|
| `url`            | string  | —       | HTTPS URL to fetch (required)              |
| `method`         | string  | `GET`   | HTTP method                                |
| `headers`        | object  | `{}`    | Additional request headers                 |
| `body`           | any     | —       | Request body (serialized as JSON)          |
| `timeout_seconds`| number  | `45`    | Request timeout                            |

**Returns:** `{ handle_id: "hdl_<12hex>", teaser: { ... } }`

### `query_handle`

Runs a JSONPath expression against a previously stored payload.

| Parameter   | Type   | Description                            |
|-------------|--------|----------------------------------------|
| `handle_id` | string | Handle returned by `fetch_json_cutted` |
| `json_path` | string | JSONPath expression (e.g. `$.users[0].email`) |

**Returns:** The matched value(s) as JSON.

Handle IDs are deterministic (SHA-256 of canonicalized JSON) — the same payload always produces the same `hdl_<12hex>`, making repeated fetches idempotent.

## Install

### Binary (recommended for production)

Download the pre-built binary for your platform from [Releases](https://github.com/nikitaclicks/context-cutter/releases) and place it on `PATH`:

| Platform             | Binary name                                    |
|----------------------|------------------------------------------------|
| Linux x86_64         | `context-cutter-mcp-x86_64-linux-gnu`          |
| macOS Intel          | `context-cutter-mcp-x86_64-apple-darwin`       |
| macOS Apple Silicon  | `context-cutter-mcp-aarch64-apple-darwin`      |
| Windows x86_64       | `context-cutter-mcp-x86_64-pc-windows-msvc.exe`|

Then point your client at the binary directly instead of using `npx`.

### npx (zero-install)

```bash
npx context-cutter-mcp
```

Downloads the matching GitHub Release binary on first run. Suitable for development and CI.

### npm (global install)

```bash
npm install -g context-cutter-mcp
context-cutter-mcp
```

### Docker

```bash
docker run --rm -i ghcr.io/nikitaclicks/context-cutter-mcp:latest
```

### Build from source

Requires Rust 1.77+:

```bash
cargo build --release --bin context-cutter-mcp
./target/release/context-cutter-mcp
```

### Python SDK (optional)

For embedding ContextCutter directly in a Python agent without running a separate process:

```bash
pip install context-cutter
```

```python
from context_cutter import store_response, generate_teaser, query_handle

handle = store_response(api_response_dict)
teaser = generate_teaser(handle)   # compact summary for the model
value  = query_handle(handle, "$.users[0].email")
```

The `@lazy_handle` decorator wraps any function that returns JSON:

```python
from context_cutter import lazy_handle

@lazy_handle
def get_users() -> dict:
    return requests.get("https://api.example.com/users").json()

result = get_users()
# result = {"handle_id": "hdl_...", "teaser": {...}}
```

See [`CONTRIBUTING.md`](CONTRIBUTING.md) for full Python SDK documentation.

## Configuration

Environment variables for the MCP server:

| Variable                          | Default    | Description                               |
|-----------------------------------|------------|-------------------------------------------|
| `CONTEXT_CUTTER_MAX_HANDLES`      | `1000`     | Max payloads held in the LRU store        |
| `CONTEXT_CUTTER_TTL_SECS`         | `3600`     | Seconds before a handle expires           |
| `CONTEXT_CUTTER_MAX_PAYLOAD_BYTES`| `10485760` | Max accepted response size (10 MB)        |
| `CONTEXT_CUTTER_LOG_FORMAT`       | `plain`    | `plain` or `json` structured logs         |
| `RUST_LOG`                        | `info`     | Tracing filter (e.g. `debug`, `trace`)    |

## Security

- HTTPS-only URL fetching (SSRF hardening — `http://` is rejected)
- Null-byte rejection on all string inputs
- JSONPath expressions capped at 4096 characters
- Payload size enforced before storing (`MAX_PAYLOAD_BYTES`)
- No credentials stored — headers are not persisted with payloads

## Performance

Operation latencies (median, on commodity hardware):

| Operation                     | Median latency |
|-------------------------------|---------------:|
| `generate_teaser` (medium payload) |     35 µs |
| `store_response` (small payload)   |     64 µs |
| `query_handle` (wildcard path)     |     94 µs |

Throughput: ~10,000–27,000 operations/second per operation type.

## Development

```bash
# Rust
cargo test
cargo clippy -- -D warnings
cargo fmt --check

# Python SDK
pip install -e ".[dev]"
maturin develop --features python
pytest -m "not ai_e2e_live"

# Benchmarks
pytest -m benchmark --benchmark-json benchmark.json
```

See [`CONTRIBUTING.md`](CONTRIBUTING.md) for the full contributor workflow and architecture notes.

## Project layout

```
src/
  engine.rs        Pure Rust: handle ID, store, teaser, JSONPath query
  store.rs         Bounded in-memory store (TTL + LRU eviction)
  parser.rs        Teaser generation and JSONPath helpers
  lib.rs           Optional PyO3 bindings (--features python)
  bin/mcp.rs       MCP stdio server binary
python/context_cutter/
  core.py          store_response, generate_teaser, query_path
  interceptor.py   @lazy_handle decorator
  store.py         BaseStore, InMemoryStore, RedisStore
  tools.py         generate_tool_manifest (OpenAI-style schemas)
examples/
  opencode.md      Full OpenCode walkthrough with session transcript
  claude-desktop.md  Claude Desktop showcase
  openai-agents-sdk.py
  langchain_mcp.md
```

## License

MIT. See [`LICENSE`](LICENSE).
