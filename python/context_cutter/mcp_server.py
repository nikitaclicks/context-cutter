"""Native MCP server integration for ContextCutter."""

from __future__ import annotations

import json
from typing import Any
from urllib import request as urllib_request

from .core import generate_teaser_map_for_handle, store_response
from .query import query_handle
from .store import BaseStore


def fetch_json_cutted(
    *,
    url: str,
    method: str = "GET",
    headers: dict[str, str] | None = None,
    body: Any | None = None,
    timeout_seconds: float = 45.0,
    store: BaseStore | None = None,
) -> dict[str, Any]:
    """Fetch JSON over HTTP and return ContextCutter envelope."""
    payload = _fetch_json(
        url=url,
        method=method,
        headers=headers,
        body=body,
        timeout_seconds=timeout_seconds,
    )
    handle_id = store_response(payload, store=store)
    teaser = generate_teaser_map_for_handle(handle_id, store=store)
    return {"handle_id": handle_id, "teaser": teaser}


def query_handle_tool(
    *, handle_id: str, json_path: str, store: BaseStore | None = None
) -> Any:
    """Query a previously stored handle with JSONPath or dot notation."""
    return query_handle(handle_id, json_path, store=store)


def create_mcp_server(*, store: BaseStore | None = None, name: str = "context-cutter") -> Any:
    """Create an MCP server exposing ContextCutter tools."""
    fastmcp_cls = _load_fastmcp_class()
    app = fastmcp_cls(name)

    @app.tool()
    def fetch_json_cutted_tool(
        url: str,
        method: str = "GET",
        headers: dict[str, str] | None = None,
        body: Any | None = None,
        timeout_seconds: float = 45.0,
    ) -> dict[str, Any]:
        return fetch_json_cutted(
            url=url,
            method=method,
            headers=headers,
            body=body,
            timeout_seconds=timeout_seconds,
            store=store,
        )

    @app.tool()
    def query_handle_mcp(handle_id: str, json_path: str) -> Any:
        return query_handle_tool(handle_id=handle_id, json_path=json_path, store=store)

    return app


def run_mcp_server_stdio(*, store: BaseStore | None = None, name: str = "context-cutter") -> None:
    """Run ContextCutter MCP server over stdio."""
    app = create_mcp_server(store=store, name=name)
    app.run(transport="stdio")


def main() -> None:
    """Console entrypoint for MCP stdio server."""
    run_mcp_server_stdio()


def _fetch_json(
    *,
    url: str,
    method: str,
    headers: dict[str, str] | None,
    body: Any | None,
    timeout_seconds: float,
) -> Any:
    request_headers = {"Accept": "application/json"}
    if headers:
        request_headers.update(headers)

    request_data: bytes | None = None
    if body is not None:
        if isinstance(body, (dict, list)):
            request_data = json.dumps(body).encode("utf-8")
            request_headers.setdefault("Content-Type", "application/json")
        elif isinstance(body, str):
            request_data = body.encode("utf-8")
        elif isinstance(body, bytes):
            request_data = body
        else:
            raise TypeError("body must be dict, list, str, bytes, or None")

    req = urllib_request.Request(
        url,
        data=request_data,
        headers=request_headers,
        method=method.upper(),
    )
    with urllib_request.urlopen(req, timeout=timeout_seconds) as response:
        raw = response.read()

    try:
        return json.loads(raw.decode("utf-8"))
    except Exception as exc:
        raise ValueError("HTTP response is not valid JSON") from exc


def _load_fastmcp_class() -> Any:
    try:
        from mcp.server.fastmcp import FastMCP
    except Exception as exc:
        raise ModuleNotFoundError(
            "mcp package is required for MCP server support. Install with: pip install 'context-cutter[mcp]'"
        ) from exc
    return FastMCP
