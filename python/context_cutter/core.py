"""Core Handle & Path operations."""

from __future__ import annotations

import json
from hashlib import sha256
from typing import Any

from ._lib import (
    generate_teaser as _rust_generate_teaser,
    query_path as _rust_query_path,
    store_response as _rust_store_response,
)
from .query import normalize_json_path, query_handle
from .store import BaseStore
from .teaser import generate_teaser_map


def _normalize_payload(payload: str | Any) -> Any:
    if isinstance(payload, str):
        return json.loads(payload)
    return payload


def _deterministic_handle_id(payload: Any) -> str:
    canonical = json.dumps(
        payload, sort_keys=True, separators=(",", ":"), ensure_ascii=False
    )
    digest = sha256(canonical.encode("utf-8")).hexdigest()
    return f"hdl_{digest[:12]}"


def store_response(payload: str | Any, *, store: BaseStore | None = None) -> str:
    """Store JSON payload and return handle ID.

    Default path delegates to Rust for performance.
    """
    normalized = _normalize_payload(payload)
    if store is None:
        return _rust_store_response(json.dumps(normalized))
    selected_store = store
    handle_id = _deterministic_handle_id(normalized)
    selected_store.set(handle_id, normalized)
    return handle_id


def generate_teaser_map_for_handle(
    handle_id: str, *, store: BaseStore | None = None
) -> dict[str, Any]:
    """Generate teaser map for an existing handle."""
    if store is None:
        return json.loads(_rust_generate_teaser(handle_id))
    payload = store.get(handle_id)
    if payload is None:
        raise KeyError(f"unknown handle_id: {handle_id}")
    return generate_teaser_map(payload)


def generate_teaser(handle_id: str, *, store: BaseStore | None = None) -> str:
    """Compatibility helper returning teaser as JSON string."""
    if store is None:
        return _rust_generate_teaser(handle_id)
    teaser = generate_teaser_map_for_handle(handle_id, store=store)
    return json.dumps(teaser)


def query_path(
    handle_id: str, json_path: str, *, store: BaseStore | None = None
) -> str:
    """Compatibility helper returning query result as JSON string."""
    if store is None:
        return _rust_query_path(handle_id, normalize_json_path(json_path))
    result = query_handle(handle_id, json_path, store=store)
    return json.dumps(result)
