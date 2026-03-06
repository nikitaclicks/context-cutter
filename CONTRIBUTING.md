# Contributing to ContextCutter

Thank you for helping make ContextCutter better. This guide covers everything you need to get started.

## Table of Contents

- [Development Setup](#development-setup)
- [Running Tests](#running-tests)
- [Code Style](#code-style)
- [Pull Request Process](#pull-request-process)
- [Reporting Issues](#reporting-issues)

---

## Development Setup

### Prerequisites

- **Rust** ≥ 1.75 (stable toolchain)
- **Python** ≥ 3.9 (for the Python SDK and tests)
- `maturin` (for building PyO3 extension)

### Clone and build

```bash
git clone https://github.com/nikitaclicks/context-cutter.git
cd context-cutter

# Build the MCP binary
cargo build --bin context-cutter-mcp

# Build the Python SDK (optional)
python3 -m venv .venv && source .venv/bin/activate
pip install maturin
maturin develop --features python
pip install -e ".[dev]"
```

### Pre-commit hooks (optional but recommended)

```bash
pip install pre-commit
pre-commit install
```

---

## Running Tests

### Rust tests

```bash
cargo test
```

### Python tests

```bash
# Unit + integration (no network, no API keys required)
pytest -m "not ai_e2e_offline and not ai_e2e_live"

# All offline AI harness tests
pytest -m ai_e2e_offline

# Full suite (requires API key env vars)
pytest
```

### Lint checks

```bash
# Rust
cargo clippy -- -D warnings
cargo fmt --check

# Python
ruff check python/ tests/
```

---

## Code Style

- **Rust** — `rustfmt` default style. Run `cargo fmt` before committing.
- **Python** — `ruff` + `black`. Configured in `pyproject.toml`.
- Keep public items documented with doc-comments.
- New Rust functions in `engine.rs` must have a unit test.
- New Python functions must have a matching test in `tests/`.

---

## Pull Request Process

1. Fork the repo and create a feature branch: `git checkout -b feat/my-feature`.
2. Make your changes with tests.
3. Run the full lint and test suite locally.
4. Open a PR against `main` with a clear description of _what_ and _why_.
5. A maintainer will review within a few business days. Iterative review is normal; don't be discouraged by feedback.

### PR checklist

- [ ] Tests pass: `cargo test && pytest -m "not ai_e2e_live"`
- [ ] Lint passes: `cargo clippy -- -D warnings && cargo fmt --check`
- [ ] CHANGELOG.md entry added under `[Unreleased]`
- [ ] Public API changes are reflected in docs

---

## Reporting Issues

- **Bugs**: Open a GitHub Issue with reproduction steps, expected vs. actual behaviour, and version info (`cargo version`, `python --version`).
- **Security vulnerabilities**: See [SECURITY.md](SECURITY.md) — please do **not** open a public issue for security bugs.
- **Feature requests**: Open a GitHub Discussion or Issue with your use case.

---

## Architecture Notes

| Layer | File | Purpose |
|---|---|---|
| Core engine | `src/engine.rs` | Pure Rust: handle ID computation, store, teaser, query |
| Storage | `src/store.rs` | Global DashMap store + optional eviction |
| MCP binary | `src/bin/mcp.rs` | stdio MCP server (`rmcp`) |
| PyO3 bindings | `src/lib.rs` | Python extension (`--features python`) |
| Python SDK | `python/context_cutter/` | Decorators, query helpers |

The MCP binary has zero PyO3 dependency — keep it that way. The `python` feature is strictly opt-in.
