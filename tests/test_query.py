from __future__ import annotations

import pytest

from context_cutter.query import normalize_json_path, query_handle
from context_cutter.store import InMemoryStore


def test_normalize_json_path_converts_dot_indices() -> None:
    assert normalize_json_path("contributors.0.login") == "$.contributors[0].login"


def test_normalize_json_path_accepts_existing_json_path() -> None:
    assert normalize_json_path("$.contributors[0].login") == "$.contributors[0].login"


def test_normalize_json_path_rejects_empty_string() -> None:
    with pytest.raises(ValueError):
        normalize_json_path("")


def test_query_handle_with_custom_store_returns_scalar() -> None:
    store = InMemoryStore()
    store.set("hdl_x", {"contributors": [{"login": "octocat"}]})
    assert query_handle("hdl_x", "contributors.0.login", store=store) == "octocat"


def test_query_handle_with_custom_store_returns_none_for_no_match() -> None:
    store = InMemoryStore()
    store.set("hdl_x", {"items": [1, 2, 3]})
    assert query_handle("hdl_x", "$.missing", store=store) is None


def test_query_handle_with_custom_store_raises_for_unknown_handle() -> None:
    with pytest.raises(KeyError):
        query_handle("missing", "$.items", store=InMemoryStore())


def test_query_handle_with_custom_store_raises_for_invalid_json_path() -> None:
    store = InMemoryStore()
    store.set("hdl_x", {"items": [1, 2, 3]})
    with pytest.raises(ValueError):
        query_handle("hdl_x", "$.[", store=store)

