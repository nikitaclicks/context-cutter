"""Tool manifest helpers for LLM tool-calling."""

from __future__ import annotations

from typing import Any


def generate_tool_manifest() -> list[dict[str, Any]]:
    """Return a minimal tool manifest for querying stored JSON by JSONPath."""
    return [
        {
            "type": "function",
            "function": {
                "name": "query_json_path",
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
        }
    ]
