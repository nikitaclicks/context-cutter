"""Decorator interceptor that returns handle + teaser."""

from __future__ import annotations

import json
from functools import wraps
from typing import Any, Callable, TypeVar

try:
    # Python 3.10+
    from typing import ParamSpec
except ImportError:  # pragma: no cover - exercised on 3.9 only
    from typing_extensions import ParamSpec

from .core import generate_teaser_map_for_handle, store_response
from .schemas import LazyHandleResponse
from .store import BaseStore

P = ParamSpec("P")
R = TypeVar("R")


def _coerce_payload(result: Any) -> Any:
    if isinstance(result, str):
        return json.loads(result)
    if isinstance(result, (dict, list)):
        return result
    raise TypeError(
        "lazy_handle expects wrapped function to return dict, list, or JSON string"
    )


def lazy_handle(
    func: Callable[P, R] | None = None,
    *,
    store: BaseStore | None = None,
) -> Callable[..., dict[str, Any]]:
    """Decorate a tool function to return a teaser + handle instead of full JSON."""

    def decorator(inner: Callable[P, R]) -> Callable[P, dict[str, Any]]:
        @wraps(inner)
        def wrapped(*args: P.args, **kwargs: P.kwargs) -> dict[str, Any]:
            payload = _coerce_payload(inner(*args, **kwargs))
            handle_id = store_response(payload, store=store)
            teaser = generate_teaser_map_for_handle(handle_id, store=store)
            return LazyHandleResponse(handle_id=handle_id, teaser=teaser).model_dump()

        return wrapped

    if func is not None:
        return decorator(func)
    return decorator
