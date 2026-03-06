# Copilot Instructions

Use `AGENTS.md` as the canonical project context for this repository.

If there is any conflict between this file and other documentation, prefer `AGENTS.md`.

## Key Defaults

- Mission: eliminate JSON bloat in LLM agentic workflows.
- Primary integration: Rust MCP binary (`context-cutter-mcp`) — no Python required.
- Secondary integration: Python SDK via PyO3/Maturin compiled extension (optional).
- Core MCP tool contract (stable, do not rename): `fetch_json_cutted` + `query_handle`.
- Engine lives in `src/engine.rs` (pure Rust). PyO3 bindings in `src/lib.rs` (feature-gated).
- Handle IDs: deterministic SHA-256, prefix `hdl_`, 12-char hex suffix.
