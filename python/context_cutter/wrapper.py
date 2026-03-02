"""Python UX decorators for lazy JSON handling."""

from __future__ import annotations

import json
from functools import wraps
from typing import Any, Callable, ParamSpec, TypeVar

from ._lib import generate_teaser, store_response

P = ParamSpec("P")
R = TypeVar("R")


def lazy_tool(func: Callable[P, R]) -> Callable[P, dict[str, Any]]:
    """Decorate a tool function to return a teaser + handle instead of full JSON."""

    @wraps(func)
    def wrapped(*args: P.args, **kwargs: P.kwargs) -> dict[str, Any]:
        result = func(*args, **kwargs)

        if isinstance(result, dict):
            json_str = json.dumps(result)
        elif isinstance(result, str):
            # Validate shape and normalize formatting.
            parsed = json.loads(result)
            json_str = json.dumps(parsed)
        else:
            raise TypeError(
                "lazy_tool expects wrapped function to return dict or JSON string"
            )

        handle_id = store_response(json_str)
        teaser_str = generate_teaser(handle_id)

        return {
            "handle_id": handle_id,
            "teaser": json.loads(teaser_str),
        }

    return wrapped
