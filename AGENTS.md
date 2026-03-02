# Agent Instructions

This file is the canonical project context for this repository.

If there is any conflict between this file and other documentation, prefer `AGENTS.md`.

## Project Source of Truth: `ContextCutter`

### 1. The Mission
To eliminate "JSON Bloat" in LLM agentic workflows. `ContextCutter` is a high-performance middleware that intercepts massive API responses and replaces them with a lightweight "Teaser" and a "Lazy Handle," saving up to 90% of context window tokens.

### 2. The Architecture

- **Engine (Rust):** High-speed JSON parsing (`serde_json`), tree-walking for schema mapping, and JSONPath execution (`jsonpath-rust`).
- **Storage:** A stateful, thread-safe in-memory cache (`DashMap`) or a persistent `Valkey/Redis` store to hold the full JSON payloads.
- **Bridge (PyO3/Maturin):** A compiled Rust-to-Python bridge that ensures near-zero latency for data transfers between the agent and the storage.
- **UX (Python):** A simple `@lazy_tool` decorator that wraps existing API functions and automatically registers the `query_handle` tool for the agent.

### 3. The Interaction Flow

1. Agent calls a tool (e.g., `get_user_data`).
2. `ContextCutter` intercepts the 50KB JSON.
3. It stores the JSON and returns a 200-token "Teaser" map + `handle_id`.
4. Agent uses `query_handle(handle_id, "$.path.to.data")` to fetch only the specific value it needs.

### 4. Key Technical Requirements

- Must support deterministic `handle_id` generation for session persistence.
- Must be storage agnostic (in-memory for dev, Redis for prod).
- Must provide a minimized "Schema Teaser" that lists keys and array lengths without the raw data.
