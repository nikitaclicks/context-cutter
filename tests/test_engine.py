"""Tests for Rust core bindings."""

from __future__ import annotations

import json

import pytest

from context_cutter._lib import ContextStore, generate_teaser, query_path, store_response


def test_store_response_returns_unique_handle_ids() -> None:
    h1 = store_response('{"a": 1}')
    h2 = store_response('{"a": 2}')

    assert isinstance(h1, str) and h1
    assert isinstance(h2, str) and h2
    assert h1 != h2


def test_store_response_rejects_invalid_json() -> None:
    with pytest.raises(ValueError):
        store_response("{not valid json}")


def test_generate_teaser_returns_stub_json() -> None:
    handle = store_response('{"nested": {"ok": true}}')
    teaser = generate_teaser(handle)
    parsed = json.loads(teaser)

    assert parsed["_teaser"] is True
    assert parsed["_type"] == "object"
    assert "stub" in parsed["_keys"]


def test_generate_teaser_unknown_handle_raises_key_error() -> None:
    with pytest.raises(KeyError):
        generate_teaser("missing-handle")


def test_query_path_returns_stub_result() -> None:
    handle = store_response('{"items": [1, 2, 3]}')
    result = query_path(handle, "$.items[*]")
    assert result == '["stub_result"]'


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
