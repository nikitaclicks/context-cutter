# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.0] - 2026-03-06

### Added

- Rust MCP server binary: `context-cutter-mcp`.
- Stable MCP tool contract: `fetch_json_cutted` and `query_handle`.
- Deterministic handle IDs using canonicalized JSON + SHA-256 (`hdl_<12hex>`).
- In-memory handle store powered by `DashMap`.
- Schema teaser generation for compact context previews.
- JSONPath query support against stored payloads.
- Optional Python SDK integration through PyO3/Maturin (`--features python`).
- Python decorators and helpers: `@lazy_handle`, `@lazy_tool`, `store_response`, `query_handle`, `generate_teaser`, and tool manifest generation.
- Unit and end-to-end test suite for core functionality.

[Unreleased]: https://github.com/nikitaclicks/context-cutter/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/nikitaclicks/context-cutter/releases/tag/v0.1.0
