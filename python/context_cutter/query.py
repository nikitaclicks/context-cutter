"""Handle querying helpers using jsonpath-ng."""

from __future__ import annotations

import json
import re
from typing import Any

from jsonpath_ng import parse

from ._lib import query_path as _rust_query_path
from .schemas import QueryRequest
from .store import BaseStore

_DOT_INDEX_RE = re.compile(r"\.(\d+)")


def normalize_json_path(path: str) -> str:
    """Normalize simple dot notation to a JSONPath expression."""
    if not path:
        raise ValueError("json path must not be empty")
    if path.startswith("$"):
        return path
    normalized = f"$.{path}"
    return _DOT_INDEX_RE.sub(r"[\1]", normalized)


def query_handle(
    handle_id: str,
    json_path: str,
    *,
    store: BaseStore | None = None,
) -> Any:
    """Query a stored payload by handle_id and JSONPath/dot notation.

    Default path delegates to Rust for performance.
    """
    req = QueryRequest(handle_id=handle_id, json_path=json_path)
    normalized_path = normalize_json_path(req.json_path)
    if store is None:
        return json.loads(_rust_query_path(req.handle_id, normalized_path))

    selected_store = store
    payload = selected_store.get(req.handle_id)
    if payload is None:
        raise KeyError(f"unknown handle_id: {req.handle_id}")

    try:
        expression = parse(normalized_path)
    except Exception as exc:
        raise ValueError(f"invalid json path: {req.json_path}") from exc

    matches = [match.value for match in expression.find(payload)]
    if not matches:
        return None
    if len(matches) == 1:
        return matches[0]
    return matches
