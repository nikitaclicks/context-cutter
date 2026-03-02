"""Tests for Python-facing UX helpers."""

from __future__ import annotations

import json

import pytest

from context_cutter import lazy_tool, query_path


@lazy_tool
def dict_payload_tool() -> dict[str, object]:
    """Return a sample dictionary payload."""
    return {"status": "ok", "items": [1, 2, 3]}


@lazy_tool
def string_payload_tool() -> str:
    """Return a sample JSON string payload."""
    return json.dumps({"kind": "string_payload", "count": 2})


@lazy_tool
def invalid_payload_tool() -> list[int]:
    """Return an unsupported payload type."""
    return [1, 2, 3]


def test_lazy_tool_wraps_dict_payload() -> None:
    result = dict_payload_tool()
    assert set(result.keys()) == {"handle_id", "teaser"}
    assert isinstance(result["handle_id"], str) and result["handle_id"]
    assert isinstance(result["teaser"], dict)


def test_lazy_tool_wraps_json_string_payload() -> None:
    result = string_payload_tool()
    assert set(result.keys()) == {"handle_id", "teaser"}
    assert isinstance(result["handle_id"], str) and result["handle_id"]
    assert isinstance(result["teaser"], dict)


def test_handle_from_lazy_tool_works_with_query_path() -> None:
    result = dict_payload_tool()
    out = query_path(result["handle_id"], "$.items[*]")
    assert out == '["stub_result"]'


def test_lazy_tool_rejects_unsupported_type() -> None:
    with pytest.raises(TypeError):
        invalid_payload_tool()


def test_lazy_tool_preserves_metadata() -> None:
    assert dict_payload_tool.__name__ == "dict_payload_tool"
    assert dict_payload_tool.__doc__ == "Return a sample dictionary payload."
