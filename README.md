# ContextCutter

Rust-first Handle & Path library for LLM tool-calling.

When a tool returns a large JSON payload, sending the full object back to the model is expensive and harms reasoning quality.  
`ContextCutter` stores the full payload behind a lightweight handle and returns a compact structural teaser. The model can then request only the fields it needs.

## Why this exists

- Avoids pushing 50KB+ API payloads into context windows.
- Reduces token cost and "lost in the middle" failures.
- Supports multi-step retrieval via handle + JSONPath/dot-path queries.
- Keeps heavy operations in Rust (PyO3), with Python as the bridge.

## Architecture

- **Rust (`src/`)**
  - In-memory handle store
  - Teaser generation logic
  - JSONPath query execution
  - PyO3 module: `context_cutter._lib`
- **Python (`python/context_cutter/`)**
  - Decorator ergonomics (`@lazy_handle`)
  - Public API exports
  - Tool manifest helper
  - Optional custom stores (`InMemoryStore`, `RedisStore`) for non-default flows

## Install

```bash
python3 -m venv .venv
source .venv/bin/activate
python -m pip install -e .
```

For development:

```bash
python -m pip install -e ".[dev]"
```

Redis support (optional):

```bash
python -m pip install -e ".[redis]"
```

## Quick start

```python
from context_cutter import lazy_handle, query_handle


@lazy_handle
def get_github_contributors() -> dict:
    # Replace this with real HTTP call result.
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

- `lazy_handle`: decorator that:
  1. executes wrapped function
  2. stores payload under a generated handle
  3. returns `{handle_id, teaser}`
- `lazy_tool`: backward-compatible alias of `lazy_handle`

### Querying

- `query_handle(handle_id, json_path) -> Any`
  - Returns parsed Python values (`dict`, `list`, scalar, or `None`)
  - Accepts JSONPath and simple dot notation

### Compatibility functions

- `store_response(payload) -> str`  
  Stores payload and returns handle id.
- `generate_teaser(handle_id) -> str`  
  Returns teaser JSON string.
- `query_path(handle_id, json_path) -> str`  
  Returns query result as JSON string.

These are useful for legacy tool wiring that expects serialized JSON boundaries.

### Store abstractions (Python)

- `BaseStore`
- `InMemoryStore`
- `RedisStore`
- `set_default_store(...)` / `get_default_store()`

Note: default no-argument API paths use the Rust-backed global store for speed.

## Tool manifest

Generate tool definitions for LLM frameworks:

```python
from context_cutter import generate_tool_manifest

tools = generate_tool_manifest()
```

Includes `query_handle` and a backward-compatible alias entry.

## Run tests

```bash
source .venv/bin/activate
pytest -q
```
