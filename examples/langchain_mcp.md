# LangChain + ContextCutter MCP

ContextCutter runs as a standard stdio MCP server, so any LangChain stack that supports MCP can launch it.

## Command

```bash
npx -y context-cutter-mcp
```

## Expected tools

- `fetch_json_cutted`
- `query_handle`

Use these tools in a two-step retrieval flow:
1. Fetch/store API JSON through `fetch_json_cutted`.
2. Pull only required fields with `query_handle`.
