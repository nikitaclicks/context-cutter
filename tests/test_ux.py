"""Tests for Python-facing UX helpers."""

from __future__ import annotations

import json

import pytest

from context_cutter import InMemoryStore, lazy_handle, lazy_tool, query_path, set_default_store


@pytest.fixture(autouse=True)
def isolated_default_store() -> None:
    set_default_store(InMemoryStore())


@lazy_handle
def dict_payload_tool() -> dict[str, object]:
    """Return a sample dictionary payload."""
    return {"status": "ok", "items": [1, 2, 3], "contributors": [{"login": "octocat"}]}


@lazy_tool
def string_payload_tool() -> str:
    """Return a sample JSON string payload."""
    return json.dumps({"kind": "string_payload", "count": 2})


@lazy_tool
def invalid_payload_tool() -> int:
    """Return an unsupported payload type."""
    return 42


def test_lazy_tool_wraps_dict_payload() -> None:
    result = dict_payload_tool()
    assert set(result.keys()) == {"handle_id", "teaser"}
    assert isinstance(result["handle_id"], str) and result["handle_id"]
    assert isinstance(result["teaser"], dict)
    assert result["teaser"]["_teaser"] is True
    assert "contributors" in result["teaser"]["keys"]


def test_lazy_tool_wraps_json_string_payload() -> None:
    result = string_payload_tool()
    assert set(result.keys()) == {"handle_id", "teaser"}
    assert isinstance(result["handle_id"], str) and result["handle_id"]
    assert isinstance(result["teaser"], dict)


def test_handle_from_lazy_tool_works_with_query_path() -> None:
    result = dict_payload_tool()
    out = query_path(result["handle_id"], "$.items[0]")
    assert json.loads(out) == 1


def test_lazy_tool_rejects_unsupported_type() -> None:
    with pytest.raises(TypeError):
        invalid_payload_tool()


def test_lazy_tool_preserves_metadata() -> None:
    assert dict_payload_tool.__name__ == "dict_payload_tool"
    assert dict_payload_tool.__doc__ == "Return a sample dictionary payload."


def test_lazy_handle_with_custom_store() -> None:
    store = InMemoryStore()

    @lazy_handle(store=store)
    def my_tool() -> dict[str, object]:
        return {"key": "value", "nums": [1, 2, 3]}

    result = my_tool()
    assert set(result.keys()) == {"handle_id", "teaser"}
    assert result["handle_id"].startswith("hdl_")
    assert result["teaser"]["_teaser"] is True
    # The payload should be in the custom store, not the default store.
    assert store.get(result["handle_id"]) is not None


def test_lazy_handle_factory_form_returns_decorator() -> None:
    store = InMemoryStore()

    @lazy_handle(store=store)
    def list_tool() -> list[object]:
        return [{"id": 1}, {"id": 2}]

    result = list_tool()
    assert result["teaser"]["_teaser"] is True
    assert "Array" in result["teaser"]["_type"]
