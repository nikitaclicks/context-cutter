# Claude Project Instructions

Use `AGENTS.md` as the canonical project context for this repository.

If there is any conflict between this file and other documentation, prefer `AGENTS.md`.

## Key Defaults

- Mission: eliminate JSON bloat in LLM agentic workflows.
- Core flow: store full JSON, return teaser + `handle_id`, query via `query_handle`.
- Requirements: deterministic `handle_id`, storage-agnostic design, minimized schema teaser.
