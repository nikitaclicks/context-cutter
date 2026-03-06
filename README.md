# ContextCutter

[![Tests](https://github.com/nikitaclicks/context-cutter/actions/workflows/tests.yml/badge.svg)](https://github.com/nikitaclicks/context-cutter/actions/workflows/tests.yml)
[![codecov](https://codecov.io/gh/nikitaclicks/context-cutter/branch/main/graph/badge.svg)](https://codecov.io/gh/nikitaclicks/context-cutter)

Agent-agnostic middleware that eliminates JSON bloat in LLM tool-calling workflows.

`ContextCutter` keeps large API responses out of the model context window by returning:

- a deterministic handle (`hdl_<12hex>`) for the full payload
- a compact structural teaser for planning

Agents then request only targeted fields via JSONPath when needed.

## Why ContextCutter

- Reduces prompt token usage for large JSON responses.
- Prevents "lost in the middle" degradation on multi-step tasks.
- Works with any MCP-compatible client.
- Ships as a standalone Rust binary (`context-cutter-mcp`) with no Python runtime required.

## MCP Tool Contract (Stable)

- `fetch_json_cutted(url, method?, headers?, body?, timeout_seconds?)`
- `query_handle(handle_id, json_path)`

These tool names are stable and should not be renamed.

## Install

## 1) Binary (recommended)

### GitHub Release

Download the platform binary from Releases and place it on `PATH`:

- Linux: `context-cutter-mcp-x86_64-linux-gnu`
- macOS (Intel): `context-cutter-mcp-x86_64-apple-darwin`
- macOS (Apple Silicon): `context-cutter-mcp-aarch64-apple-darwin`
- Windows: `context-cutter-mcp-x86_64-pc-windows-msvc.exe`

### Build from source

```bash
cargo build --release --bin context-cutter-mcp
./target/release/context-cutter-mcp
```

## 2) Zero-install via `npx`

```bash
npx context-cutter-mcp
```

The npm wrapper downloads the matching GitHub Release binary automatically.

## 3) Docker

```bash
docker run --rm -i ghcr.io/nikitaclicks/context-cutter-mcp:latest
```

## 4) Homebrew

```bash
brew tap nikitaclicks/context-cutter
brew install context-cutter-mcp
```

## 5) Python SDK (optional)

```bash
pip install context-cutter
```

## Client Configuration Examples

## Claude Desktop

`~/Library/Application Support/Claude/claude_desktop_config.json` (macOS path):

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

## Cursor

`.cursor/mcp.json`:

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

## VS Code Copilot Chat

`.vscode/mcp.json`:

```json
{
  "servers": {
    "context-cutter": {
      "type": "stdio",
      "command": "npx",
      "args": ["-y", "context-cutter-mcp"]
    }
  }
}
```

## OpenAI Agents SDK

```python
from agents import Agent

agent = Agent(
    name="assistant",
    mcp_servers=[
        {
            "server_label": "context-cutter",
            "command": "npx",
            "args": ["-y", "context-cutter-mcp"],
        }
    ],
)
```

## Operational Configuration

Environment variables:

- `CONTEXT_CUTTER_MAX_HANDLES` (default: `1000`)
- `CONTEXT_CUTTER_TTL_SECS` (default: `3600`)
- `CONTEXT_CUTTER_MAX_PAYLOAD_BYTES` (default: `10485760`)
- `CONTEXT_CUTTER_LOG_FORMAT` (`plain` or `json`, default: `plain`)
- `RUST_LOG` (default: `info`)

## Security Defaults

- HTTPS-only URL fetching in `fetch_json_cutted` (SSRF hardening)
- Null-byte rejection for URL/headers/handle/path inputs
- JSONPath length bounds (`<= 4096` chars)
- Response payload size limits

## Development

```bash
cargo test
cargo clippy -- -D warnings
cargo fmt --check

python -m pip install -e ".[dev]"
pytest -m "not ai_e2e_live"
```

See `CONTRIBUTING.md` for full contributor workflow.

## Project Layout

```text
src/
  engine.rs      # Pure Rust core logic
  store.rs       # Bounded in-memory store (TTL + LRU)
  parser.rs      # Teaser + JSONPath helpers
  lib.rs         # Optional PyO3 bindings
  bin/mcp.rs     # MCP stdio server binary
python/context_cutter/
  ...            # Optional Python SDK wrappers
```

## License

MIT. See `LICENSE`.
