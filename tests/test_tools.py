from __future__ import annotations

from context_cutter.tools import generate_tool_manifest


def test_generate_tool_manifest_shape() -> None:
    tools = generate_tool_manifest()
    assert isinstance(tools, list)
    assert len(tools) >= 2
    for tool in tools:
        assert tool["type"] == "function"
        assert "function" in tool
        assert "name" in tool["function"]
        assert "parameters" in tool["function"]


def test_generate_tool_manifest_includes_alias_entry() -> None:
    tools = generate_tool_manifest()
    names = {tool["function"]["name"] for tool in tools}
    assert "query_handle" in names
    assert "query_json_path" in names

