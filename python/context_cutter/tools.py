"""Tool manifest helpers for LLM tool-calling."""

from __future__ import annotations

from typing import Any


def generate_tool_manifest() -> list[dict[str, Any]]:
    """Return tool manifests for querying stored JSON by handle/path."""
    return [
        {
            "type": "function",
            "function": {
                "name": "query_handle",
                "description": (
                    "Query a previously stored JSON payload by handle_id and JSONPath."
                ),
                "parameters": {
                    "type": "object",
                    "properties": {
                        "handle_id": {
                            "type": "string",
                            "description": "Opaque handle returned by store_response.",
                        },
                        "json_path": {
                            "type": "string",
                            "description": "JSONPath expression to evaluate.",
                        },
                    },
                    "required": ["handle_id", "json_path"],
                },
            },
        },
        {
            "type": "function",
            "function": {
                "name": "query_json_path",
                "description": "Backward-compatible alias for query_handle.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "handle_id": {
                            "type": "string",
                            "description": "Opaque handle returned by store_response.",
                        },
                        "json_path": {
                            "type": "string",
                            "description": "JSONPath expression to evaluate.",
                        },
                    },
                    "required": ["handle_id", "json_path"],
                },
            },
        },
    ]
