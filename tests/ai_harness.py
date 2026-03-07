from __future__ import annotations

import json
from collections.abc import Callable
from dataclasses import dataclass
from typing import Any, Union

from context_cutter import query_handle


@dataclass(frozen=True)
class ToolCall:
    name: str
    arguments: dict[str, Any]


@dataclass(frozen=True)
class FinalAnswer:
    value: Any


Action = Union[ToolCall, FinalAnswer]
Planner = Callable[[list[dict[str, Any]], list[Exception]], Action]


def build_tool_registry(*, store: Any | None = None) -> dict[str, Callable[..., Any]]:
    def _query(handle_id: str, json_path: str) -> Any:
        return query_handle(handle_id, json_path, store=store)

    return {
        "query_handle": _query,
        "query_json_path": _query,
    }


def run_agent_loop(
    planner: Planner,
    *,
    store: Any | None = None,
    max_steps: int = 8,
) -> dict[str, Any]:
    tools = build_tool_registry(store=store)
    transcript: list[dict[str, Any]] = []
    errors: list[Exception] = []

    for _ in range(max_steps):
        action = planner(transcript, errors)

        if isinstance(action, FinalAnswer):
            return {
                "final": action.value,
                "transcript": transcript,
                "errors": errors,
            }

        if action.name not in tools:
            raise KeyError(f"unknown tool: {action.name}")

        try:
            output = tools[action.name](**action.arguments)
            transcript.append(
                {
                    "tool": action.name,
                    "arguments": action.arguments,
                    "output": output,
                }
            )
        except Exception as exc:  # pragma: no cover - behavior validated in callers
            errors.append(exc)

    raise TimeoutError("planner did not return a final answer within max_steps")


def teaser_ratio(payload: Any, envelope: dict[str, Any]) -> float:
    full_len = len(json.dumps(payload, separators=(",", ":")))
    teaser_len = len(json.dumps(envelope, separators=(",", ":")))
    return teaser_len / full_len
