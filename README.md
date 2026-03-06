# ContextCutter

Rust-first Handle & Teaser middleware for LLM tool-calling.

When a tool returns a large JSON payload, sending the full object back to the model is expensive and harms reasoning quality.  
`ContextCutter` stores the full payload behind a lightweight handle and returns a compact structural teaser. The model can then request only the fields it needs.

## Why this exists

- Avoids pushing 50KB+ API payloads into context windows.
- Reduces token cost and "lost in the middle" failures.
- Supports multi-step retrieval via handle + JSONPath/dot-path queries.
- Ships as a standalone Rust binary MCP server — no Python runtime required for integrations.

## Architecture

- **Rust MCP server (primary integration)** — single binary `context-cutter-mcp`
  - In-memory handle store (`DashMap`)
  - Teaser generation and JSONPath query execution
  - MCP stdio transport (`rmcp`)
  - HTTP fetch (`ureq`)
- **Python SDK (optional, developer ergonomics)** — PyO3/Maturin compiled bridge into the same Rust engine
  - `@lazy_handle` / `@lazy_tool` decorator ergonomics
  - `query_handle`, `store_response`, `generate_teaser`, `query_path` functions
  - Tool manifest helper for LLM framework wiring
  - Optional custom stores (`InMemoryStore`, `RedisStore`)

## Integration (MCP)

`context-cutter-mcp` is the primary integration surface. It runs as a Model Context Protocol server over stdio and exposes two tools:

- `fetch_json_cutted(url, method?, headers?, body?, timeout_seconds?)` — fetches a JSON endpoint, stores the payload, returns `{handle_id, teaser}`.
- `query_handle(handle_id, json_path)` — extracts a specific value from the stored payload via JSONPath without loading the full payload into context.

### Build from source

```bash
cargo build --release --bin context-cutter-mcp
# Binary: target/release/context-cutter-mcp
```

### MCP client configuration

#### OpenCode (`~/.config/opencode/opencode.json`)

```json
{
  "mcp": {
    "context-cutter": {
      "type": "local",
      "command": ["/path/to/context-cutter-mcp"],
      "enabled": true
    }
  }
}
```

#### Claude Code (`.mcp.json` in project root or `~/.mcp.json`)

```json
{
  "mcpServers": {
    "context-cutter": {
      "command": "/path/to/context-cutter-mcp",
      "args": []
    }
  }
}
```

#### Cursor / VS Code Copilot Chat (`.vscode/mcp.json`)

```json
{
  "servers": {
    "context-cutter": {
      "type": "stdio",
      "command": "/path/to/context-cutter-mcp"
    }
  }
}
```

### Suggested agent flow

1. Configure `context-cutter-mcp` as an MCP tool server in your LLM client.
2. Ask the agent to call `fetch_json_cutted` for external APIs instead of fetching directly.
3. Let the agent call `query_handle` for follow-up fields or array items.

This avoids dumping entire API responses into context and works across all MCP-compatible clients.

## Python SDK (optional)

The Python package wraps the same Rust engine via a PyO3/Maturin compiled extension — useful for Python-native agent stacks that do not use MCP.

### Install

```bash
python3 -m venv .venv
source .venv/bin/activate
pip install -e .
```

Development extras:

```bash
pip install -e ".[dev]"
```

Redis store support (optional):

```bash
pip install -e ".[redis]"
```

### Quick start

```python
from context_cutter import lazy_handle, query_handle


@lazy_handle
def get_github_contributors() -> dict:
    # Replace with a real HTTP call result.
    return {
        "contributors": [
            {"login": "octocat", "contributions": 500, "id": 1},
            {"login": "hubot", "contributions": 123, "id": 2},
        ],
        "meta": {"total": 2},
    }


result = get_github_contributors()
# {
#   "handle_id": "hdl_xxxxxxxxxxxx",
#   "teaser": {
#     "_teaser": True,
#     "_type": "object",
#     "keys": ["contributors", "meta"],
#     "structure": {
#       "contributors": {"_type": "Array[2]", "item_keys": ["contributions", "id", "login"]},
#       "meta": {"total": 2}
#     }
#   }
# }

top = query_handle(result["handle_id"], "$.contributors[0]")
# {"login": "octocat", "contributions": 500, "id": 1}
```

Dot-notation also works:

```python
login = query_handle(result["handle_id"], "contributors.0.login")
# "octocat"
```

## API

### Interception

- `lazy_handle`: decorator that executes the wrapped function, stores the payload, and returns `{handle_id, teaser}`.
- `lazy_tool`: backward-compatible alias of `lazy_handle`.

### Querying

- `query_handle(handle_id, json_path) -> Any`
  - Returns parsed Python values (`dict`, `list`, scalar, or `None`).
  - Accepts JSONPath (`$.foo.bar`) and simple dot notation (`foo.bar`).

### Low-level functions

- `store_response(payload) -> str` — stores payload, returns `handle_id`.
- `generate_teaser(handle_id) -> str` — returns teaser as a JSON string.
- `query_path(handle_id, json_path) -> str` — returns query result as a JSON string.

### Store abstractions (Python)

- `BaseStore`, `InMemoryStore`, `RedisStore`
- `set_default_store(...)` / `get_default_store()`

The default no-argument API paths use the Rust-backed global store (`DashMap`) for speed.

### Tool manifest

Generate tool definitions for LLM frameworks:

```python
from context_cutter import generate_tool_manifest

tools = generate_tool_manifest()
```

## Run tests

### Rust

```bash
cargo test
```

### Python

```bash
source .venv/bin/activate
pytest -q
```

Run benchmarks:

```bash
pytest -m benchmark --benchmark-json benchmark.json
```

Run deterministic AI e2e tests (offline):

```bash
pytest -m ai_e2e_offline -q
```

Run live AI e2e smoke (requires API key):

```bash
CONTEXT_CUTTER_LIVE_PROVIDER=openai OPENAI_API_KEY=... CONTEXT_CUTTER_OPENAI_MODEL=gpt-4.1-mini pytest -m ai_e2e_live -q
CONTEXT_CUTTER_LIVE_PROVIDER=gemini GEMINI_API_KEY=... CONTEXT_CUTTER_GEMINI_MODEL=gemini-1.5-pro pytest -m ai_e2e_live -q
```

CI notes:

- Default PR/push workflow excludes `ai_e2e_live` and `benchmark` markers.
- Full AI e2e execution via manual dispatch in `.github/workflows/ai-e2e.yml` (provider: `openai` or `gemini`).
