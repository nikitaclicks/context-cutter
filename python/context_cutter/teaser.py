"""Token-efficient teaser generation for JSON payloads."""

from __future__ import annotations

from typing import Any


def _small_scalar(value: Any, max_string_len: int = 24) -> Any:
    if isinstance(value, bool) or value is None:
        return value
    if isinstance(value, int):
        return value if abs(value) < 10_000 else "int"
    if isinstance(value, float):
        return value if abs(value) < 10_000 else "float"
    if isinstance(value, str):
        return value if len(value) <= max_string_len else "string"
    return None


def _summarize(value: Any, depth: int, max_depth: int) -> Any:
    if depth >= max_depth:
        if isinstance(value, dict):
            return "{...}"
        if isinstance(value, list):
            return f"Array[{len(value)}]"
        scalar = _small_scalar(value)
        return scalar if scalar is not None else type(value).__name__

    if isinstance(value, dict):
        out: dict[str, Any] = {}
        for key, inner in value.items():
            out[key] = _summarize(inner, depth + 1, max_depth)
        return out

    if isinstance(value, list):
        length = len(value)
        if length == 0:
            return "Array[0]"
        first = value[0]
        if isinstance(first, dict):
            item_keys = sorted(first.keys())
            return {"_type": f"Array[{length}]", "item_keys": item_keys}
        if isinstance(first, list):
            return {"_type": f"Array[{length}]", "item": _summarize(first, depth + 1, max_depth)}
        scalar = _small_scalar(first)
        return {"_type": f"Array[{length}]", "item_type": scalar if scalar is not None else type(first).__name__}

    scalar = _small_scalar(value)
    if scalar is not None:
        return scalar
    return type(value).__name__


def generate_teaser_map(payload: Any, *, max_depth: int = 3) -> dict[str, Any]:
    """Generate a compact structural map for a JSON payload."""
    if isinstance(payload, dict):
        structure = _summarize(payload, depth=0, max_depth=max_depth)
        return {
            "_teaser": True,
            "_type": "object",
            "keys": sorted(payload.keys()),
            "structure": structure,
        }
    if isinstance(payload, list):
        return {
            "_teaser": True,
            "_type": f"Array[{len(payload)}]",
            "structure": _summarize(payload, depth=0, max_depth=max_depth),
        }
    return {
        "_teaser": True,
        "_type": type(payload).__name__,
        "structure": _summarize(payload, depth=0, max_depth=max_depth),
    }
