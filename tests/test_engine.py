"""Tests for core handle-and-path engine."""

from __future__ import annotations

import json

import pytest

from context_cutter import (
    ContextStore,
    InMemoryStore,
    generate_teaser,
    query_handle,
    query_path,
    set_default_store,
    store_response,
)


@pytest.fixture(autouse=True)
def isolated_default_store() -> None:
    set_default_store(InMemoryStore())


def test_store_response_is_deterministic_for_same_payload() -> None:
    h1 = store_response('{"a": 1, "b": {"x": 2, "y": 3}}')
    h2 = store_response('{"b": {"y": 3, "x": 2}, "a": 1}')

    assert isinstance(h1, str) and h1.startswith("hdl_")
    assert isinstance(h2, str) and h2.startswith("hdl_")
    assert h1 == h2


def test_store_response_differs_for_different_payloads() -> None:
    h1 = store_response('{"a": 1}')
    h2 = store_response('{"a": 2}')
    assert h1 != h2


def test_store_response_custom_store_is_deterministic() -> None:
    store = InMemoryStore()
    h1 = store_response({"a": 1, "b": {"x": 2, "y": 3}}, store=store)
    h2 = store_response({"b": {"y": 3, "x": 2}, "a": 1}, store=store)
    assert h1 == h2


def test_store_response_rust_and_custom_store_match_contract() -> None:
    payload = {"a": 1, "b": {"x": 2, "y": 3}}
    rust_handle = store_response(payload)
    custom_handle = store_response(payload, store=InMemoryStore())
    assert rust_handle == custom_handle


def test_store_response_rejects_invalid_json() -> None:
    with pytest.raises(ValueError):
        store_response("{not valid json}")


def test_generate_teaser_returns_real_structure_map() -> None:
    handle = store_response(
        json.dumps(
            {
                "contributors": [
                    {"login": "octocat", "contributions": 500, "id": 1},
                    {"login": "hubot", "contributions": 123, "id": 2},
                ],
                "meta": {"total": 2},
            }
        )
    )
    teaser = json.loads(generate_teaser(handle))

    assert teaser["_teaser"] is True
    assert teaser["_type"] == "object"
    assert "contributors" in teaser["keys"]
    assert teaser["structure"]["contributors"]["_type"] == "Array[2]"
    assert "login" in teaser["structure"]["contributors"]["item_keys"]


def test_generate_teaser_unknown_handle_raises_key_error() -> None:
    with pytest.raises(KeyError):
        generate_teaser("missing-handle")


def test_query_path_returns_actual_json_result() -> None:
    handle = store_response('{"items": [1, 2, 3]}')
    result = query_path(handle, "$.items[*]")
    assert json.loads(result) == [1, 2, 3]


def test_query_handle_supports_dot_notation() -> None:
    handle = store_response('{"contributors":[{"login":"octocat","contributions":500}]}')
    result = query_handle(handle, "contributors.0.login")
    assert result == "octocat"


def test_query_path_unknown_handle_raises_key_error() -> None:
    with pytest.raises(KeyError):
        query_path("missing-handle", "$.x")


def test_context_store_round_trip_operations() -> None:
    store = ContextStore()
    assert store.len() == 0

    store.insert("h1", '{"x": 1}')
    assert store.len() == 1

    payload = store.get("h1")
    assert payload is not None
    assert json.loads(payload) == {"x": 1}

    assert store.remove("h1") is True
    assert store.len() == 0

    store.insert("h2", '{"y": 2}')
    assert store.len() == 1
    store.clear()
    assert store.len() == 0


def test_generate_teaser_with_custom_store() -> None:
    store = InMemoryStore()
    handle = store_response({"a": 1, "b": [1, 2]}, store=store)
    teaser = json.loads(generate_teaser(handle, store=store))
    assert teaser["_teaser"] is True
    assert teaser["_type"] == "object"
    assert "a" in teaser["keys"]


def test_generate_teaser_with_custom_store_unknown_handle_raises() -> None:
    store = InMemoryStore()
    with pytest.raises(KeyError):
        generate_teaser("hdl_missing", store=store)


def test_query_path_with_custom_store() -> None:
    store = InMemoryStore()
    handle = store_response({"items": [10, 20, 30]}, store=store)
    result = query_path(handle, "$.items[1]", store=store)
    assert json.loads(result) == 20
