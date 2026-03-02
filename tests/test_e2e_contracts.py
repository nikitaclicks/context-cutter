from __future__ import annotations

import json
from pathlib import Path
from typing import Any

import pytest

from context_cutter import InMemoryStore, generate_teaser, query_handle, store_response

FIXTURE_DIR = Path(__file__).parent / "fixtures" / "evals"


def _load_fixture(name: str) -> dict[str, Any]:
    return json.loads((FIXTURE_DIR / name).read_text(encoding="utf-8"))


CONTRACT_CASES = [
    (
        "github_contributors.json",
        [
            ("contributors.0.login", "octocat"),
            ("$.contributors[*].id", [1, 2]),
            ("meta.total", 2),
        ],
    ),
    (
        "nested_metrics.json",
        [
            ("service", "billing"),
            ("regions.1.stats.errors", 5),
            ("$.regions[*].name", ["us-east-1", "eu-west-1"]),
        ],
    ),
    (
        "mixed_scalar_and_arrays.json",
        [
            ("flags.enabled", True),
            ("$.tags[*]", ["alpha", "beta", "gamma"]),
            ("owner.size", 8),
        ],
    ),
]


@pytest.mark.parametrize("fixture_name,queries", CONTRACT_CASES)
def test_e2e_contract_queries_default_rust_store(
    fixture_name: str, queries: list[tuple[str, Any]]
) -> None:
    payload = _load_fixture(fixture_name)
    handle = store_response(payload)
    teaser = json.loads(generate_teaser(handle))

    assert teaser["_teaser"] is True
    assert teaser["_type"] == "object"
    for key in payload.keys():
        assert key in teaser["keys"]

    for path, expected in queries:
        assert query_handle(handle, path) == expected


@pytest.mark.parametrize("fixture_name,queries", CONTRACT_CASES)
def test_e2e_contract_queries_custom_store(
    fixture_name: str, queries: list[tuple[str, Any]]
) -> None:
    payload = _load_fixture(fixture_name)
    store = InMemoryStore()
    handle = store_response(payload, store=store)

    for path, expected in queries:
        assert query_handle(handle, path, store=store) == expected


@pytest.mark.parametrize("fixture_name,_queries", CONTRACT_CASES)
def test_e2e_contract_handles_are_deterministic(
    fixture_name: str, _queries: Any
) -> None:
    payload = _load_fixture(fixture_name)

    rust_h1 = store_response(payload)
    rust_h2 = store_response(payload)
    assert rust_h1 == rust_h2

    custom_store = InMemoryStore()
    py_h1 = store_response(payload, store=custom_store)
    py_h2 = store_response(payload, store=custom_store)
    assert py_h1 == py_h2
    assert rust_h1 == py_h1


def test_e2e_contract_negative_unknown_handle() -> None:
    with pytest.raises(KeyError):
        query_handle("missing-handle", "$.x")


def test_e2e_contract_negative_invalid_path() -> None:
    payload = _load_fixture("github_contributors.json")
    handle = store_response(payload)
    with pytest.raises(ValueError):
        query_handle(handle, "$.[")
