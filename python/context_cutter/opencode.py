"""OpenCode-focused integration helpers.

This module provides:
- process-wide HTTP interception for common Python HTTP clients
- tool registry helpers compatible with function-calling runtimes
"""

from __future__ import annotations

import json
from contextlib import contextmanager
from dataclasses import dataclass
import importlib
from threading import RLock
from typing import Any, Callable, Iterator
from urllib.parse import urlparse
from urllib import request as urllib_request

from .core import generate_teaser_map_for_handle, store_response
from .query import query_handle
from .schemas import LazyHandleResponse
from .store import BaseStore
from .tools import generate_tool_manifest


@dataclass
class _PatchState:
    enabled: bool = False
    originals: dict[str, Callable[..., Any]] | None = None
    store: BaseStore | None = None
    include_hosts: tuple[str, ...] | None = None
    exclude_hosts: tuple[str, ...] = ()


_PATCH_LOCK = RLock()
_PATCH_STATE = _PatchState()

_DEFAULT_EXCLUDED_HOSTS: tuple[str, ...] = (
    "api.openai.com",
    "api.anthropic.com",
    "generativelanguage.googleapis.com",
    "openrouter.ai",
    "api.x.ai",
    "api.cohere.ai",
    "api.mistral.ai",
    "api.together.xyz",
    "api.deepseek.com",
)


def _extract_content_type(response: Any) -> str:
    headers = getattr(response, "headers", None)
    if headers is None:
        return ""
    if hasattr(headers, "get_content_type"):
        return str(headers.get_content_type()).lower()
    if hasattr(headers, "get"):
        value = headers.get("Content-Type", "")
        return str(value).split(";", 1)[0].strip().lower()
    return ""


def _extract_content_type_from_headers(headers: Any) -> str:
    if headers is None:
        return ""
    if hasattr(headers, "get_content_type"):
        return str(headers.get_content_type()).lower()
    if hasattr(headers, "get"):
        value = headers.get("Content-Type", "")
        return str(value).split(";", 1)[0].strip().lower()
    return ""


def _extract_charset(response: Any) -> str:
    headers = getattr(response, "headers", None)
    if headers is None:
        return "utf-8"
    if hasattr(headers, "get_content_charset"):
        charset = headers.get_content_charset()
        if charset:
            return str(charset)
    return "utf-8"


def _extract_charset_from_headers(headers: Any) -> str:
    if headers is None:
        return "utf-8"
    if hasattr(headers, "get_content_charset"):
        charset = headers.get_content_charset()
        if charset:
            return str(charset)
    return "utf-8"


def _try_build_envelope(raw_bytes: bytes, response: Any, store: BaseStore | None) -> bytes:
    content_type = _extract_content_type(response)
    charset = _extract_charset(response)
    return _try_build_envelope_from_meta(raw_bytes, content_type=content_type, charset=charset, store=store)


def _normalize_hosts(hosts: tuple[str, ...] | None) -> tuple[str, ...] | None:
    if hosts is None:
        return None
    normalized: list[str] = []
    for host in hosts:
        value = host.strip().lower()
        if value:
            normalized.append(value)
    return tuple(normalized)


def _hostname_matches(hostname: str, candidate: str) -> bool:
    return hostname == candidate or hostname.endswith(f".{candidate}")


def _extract_host_from_url_value(url_value: Any) -> str | None:
    if url_value is None:
        return None
    text = str(url_value)
    try:
        parsed = urlparse(text)
    except Exception:
        return None
    if not parsed.hostname:
        return None
    return parsed.hostname.lower()


def _should_intercept_for_host(hostname: str | None) -> bool:
    if hostname is None:
        return True

    include_hosts = _PATCH_STATE.include_hosts
    if include_hosts is not None and not any(
        _hostname_matches(hostname, host) for host in include_hosts
    ):
        return False

    exclude_hosts = _PATCH_STATE.exclude_hosts
    if any(_hostname_matches(hostname, host) for host in exclude_hosts):
        return False

    return True


def _try_build_envelope_from_meta(
    raw_bytes: bytes,
    *,
    content_type: str,
    charset: str,
    store: BaseStore | None,
) -> bytes:
    is_json_content_type = content_type == "application/json" or content_type.endswith("+json")
    if not is_json_content_type:
        return raw_bytes
    try:
        decoded = raw_bytes.decode(charset)
        payload = json.loads(decoded)
    except Exception:
        return raw_bytes
    handle_id = store_response(payload, store=store)
    teaser = generate_teaser_map_for_handle(handle_id, store=store)
    envelope = LazyHandleResponse(handle_id=handle_id, teaser=teaser).model_dump()
    return json.dumps(envelope).encode("utf-8")


class _ReplayResponse:
    def __init__(self, original: Any, body: bytes) -> None:
        self._original = original
        self._body = body
        self._cursor = 0

    def read(self, amt: int = -1) -> bytes:
        if amt is None or amt < 0:
            if self._cursor >= len(self._body):
                return b""
            data = self._body[self._cursor :]
            self._cursor = len(self._body)
            return data
        if self._cursor >= len(self._body):
            return b""
        end = min(self._cursor + amt, len(self._body))
        data = self._body[self._cursor : end]
        self._cursor = end
        return data

    def close(self) -> None:
        if hasattr(self._original, "close"):
            self._original.close()

    def __enter__(self) -> "_ReplayResponse":
        return self

    def __exit__(self, exc_type: Any, exc_value: Any, traceback: Any) -> Any:
        if hasattr(self._original, "__exit__"):
            return self._original.__exit__(exc_type, exc_value, traceback)
        self.close()
        return None

    def __getattr__(self, name: str) -> Any:
        return getattr(self._original, name)


def enable_opencode_http_interception(*, store: BaseStore | None = None) -> None:
    """Patch supported HTTP clients so JSON payloads are replaced with {handle_id, teaser}."""
    enable_opencode_integration(store=store)


def enable_opencode_integration(
    *,
    store: BaseStore | None = None,
    clients: tuple[str, ...] = ("urllib", "requests", "httpx"),
    include_hosts: tuple[str, ...] | None = None,
    exclude_hosts: tuple[str, ...] = _DEFAULT_EXCLUDED_HOSTS,
) -> None:
    """Enable OpenCode HTTP interception for selected clients.

    Supported client keys: urllib, requests, httpx.
    Missing optional dependencies are ignored.
    """
    with _PATCH_LOCK:
        if _PATCH_STATE.enabled:
            return

        originals: dict[str, Callable[..., Any]] = {}
        requested = {name.strip().lower() for name in clients}

        if "urllib" in requested:
            original_urlopen = urllib_request.urlopen

            def _patched_urlopen(*args: Any, **kwargs: Any) -> Any:
                request_or_url = args[0] if args else kwargs.get("url")
                if hasattr(request_or_url, "full_url"):
                    hostname = _extract_host_from_url_value(request_or_url.full_url)
                else:
                    hostname = _extract_host_from_url_value(request_or_url)
                if not _should_intercept_for_host(hostname):
                    return original_urlopen(*args, **kwargs)

                response = original_urlopen(*args, **kwargs)
                if not hasattr(response, "read"):
                    return response
                raw_bytes = response.read()
                transformed = _try_build_envelope(raw_bytes, response, _PATCH_STATE.store)
                return _ReplayResponse(response, transformed)

            originals["urllib.urlopen"] = original_urlopen
            urllib_request.urlopen = _patched_urlopen

        if "requests" in requested:
            requests_mod = _safe_import("requests")
            if requests_mod is not None:
                session_cls = requests_mod.sessions.Session
                original_requests_request = session_cls.request

                def _patched_requests_request(self: Any, *args: Any, **kwargs: Any) -> Any:
                    url_value = None
                    if len(args) >= 2:
                        url_value = args[1]
                    elif "url" in kwargs:
                        url_value = kwargs.get("url")
                    hostname = _extract_host_from_url_value(url_value)
                    if not _should_intercept_for_host(hostname):
                        return original_requests_request(self, *args, **kwargs)

                    response = original_requests_request(self, *args, **kwargs)
                    headers = getattr(response, "headers", None)
                    content_type = _extract_content_type_from_headers(headers)
                    charset = getattr(response, "encoding", None) or "utf-8"
                    raw_bytes = bytes(getattr(response, "content", b""))
                    transformed = _try_build_envelope_from_meta(
                        raw_bytes,
                        content_type=content_type,
                        charset=charset,
                        store=_PATCH_STATE.store,
                    )
                    if transformed != raw_bytes:
                        response._content = transformed
                        response.encoding = "utf-8"
                        if hasattr(response.headers, "__setitem__"):
                            response.headers["Content-Length"] = str(len(transformed))
                    return response

                originals["requests.sessions.Session.request"] = original_requests_request
                session_cls.request = _patched_requests_request

        if "httpx" in requested:
            httpx_mod = _safe_import("httpx")
            if httpx_mod is not None:
                original_httpx_client_request = httpx_mod.Client.request
                original_httpx_async_client_request = httpx_mod.AsyncClient.request

                def _patch_httpx_response(response: Any) -> Any:
                    headers = getattr(response, "headers", None)
                    content_type = _extract_content_type_from_headers(headers)
                    raw_bytes = response.read()
                    transformed = _try_build_envelope_from_meta(
                        raw_bytes,
                        content_type=content_type,
                        charset="utf-8",
                        store=_PATCH_STATE.store,
                    )
                    if transformed != raw_bytes:
                        response._content = transformed
                        if hasattr(response, "headers") and hasattr(response.headers, "__setitem__"):
                            response.headers["content-length"] = str(len(transformed))
                    return response

                def _patched_httpx_client_request(self: Any, *args: Any, **kwargs: Any) -> Any:
                    url_value = None
                    if len(args) >= 2:
                        url_value = args[1]
                    elif "url" in kwargs:
                        url_value = kwargs.get("url")
                    hostname = _extract_host_from_url_value(url_value)
                    if not _should_intercept_for_host(hostname):
                        return original_httpx_client_request(self, *args, **kwargs)

                    response = original_httpx_client_request(self, *args, **kwargs)
                    return _patch_httpx_response(response)

                async def _patched_httpx_async_client_request(
                    self: Any, *args: Any, **kwargs: Any
                ) -> Any:
                    url_value = None
                    if len(args) >= 2:
                        url_value = args[1]
                    elif "url" in kwargs:
                        url_value = kwargs.get("url")
                    hostname = _extract_host_from_url_value(url_value)
                    if not _should_intercept_for_host(hostname):
                        return await original_httpx_async_client_request(self, *args, **kwargs)

                    response = await original_httpx_async_client_request(self, *args, **kwargs)
                    return _patch_httpx_response(response)

                originals["httpx.Client.request"] = original_httpx_client_request
                originals["httpx.AsyncClient.request"] = original_httpx_async_client_request
                httpx_mod.Client.request = _patched_httpx_client_request
                httpx_mod.AsyncClient.request = _patched_httpx_async_client_request

        if not originals:
            return

        _PATCH_STATE.originals = originals
        _PATCH_STATE.store = store
        _PATCH_STATE.include_hosts = _normalize_hosts(include_hosts)
        _PATCH_STATE.exclude_hosts = _normalize_hosts(exclude_hosts) or ()
        _PATCH_STATE.enabled = True


def disable_opencode_http_interception() -> None:
    """Restore original urllib behavior."""
    disable_opencode_integration()


def disable_opencode_integration() -> None:
    """Disable OpenCode interception and restore original client behavior."""
    with _PATCH_LOCK:
        if not _PATCH_STATE.enabled:
            return

        originals = _PATCH_STATE.originals or {}
        if "urllib.urlopen" in originals:
            urllib_request.urlopen = originals["urllib.urlopen"]

        requests_mod = _safe_import("requests")
        if requests_mod is not None and "requests.sessions.Session.request" in originals:
            requests_mod.sessions.Session.request = originals["requests.sessions.Session.request"]

        httpx_mod = _safe_import("httpx")
        if httpx_mod is not None:
            if "httpx.Client.request" in originals:
                httpx_mod.Client.request = originals["httpx.Client.request"]
            if "httpx.AsyncClient.request" in originals:
                httpx_mod.AsyncClient.request = originals["httpx.AsyncClient.request"]

        _PATCH_STATE.enabled = False
        _PATCH_STATE.originals = None
        _PATCH_STATE.store = None
        _PATCH_STATE.include_hosts = None
        _PATCH_STATE.exclude_hosts = ()


def is_opencode_http_interception_enabled() -> bool:
    """Return whether OpenCode HTTP interception is currently enabled."""
    with _PATCH_LOCK:
        return _PATCH_STATE.enabled


def build_opencode_tool_registry(*, store: BaseStore | None = None) -> dict[str, Callable[..., Any]]:
    """Return tool registry mapping for OpenCode function-calling loops."""

    def _query(handle_id: str, json_path: str) -> Any:
        return query_handle(handle_id, json_path, store=store)

    return {
        "query_handle": _query,
        "query_json_path": _query,
    }


def get_opencode_tool_manifest() -> list[dict[str, Any]]:
    """Return OpenCode-compatible tool manifest entries."""
    return generate_tool_manifest()


@contextmanager
def opencode_http_interception(
    *,
    store: BaseStore | None = None,
    clients: tuple[str, ...] = ("urllib", "requests", "httpx"),
    include_hosts: tuple[str, ...] | None = None,
    exclude_hosts: tuple[str, ...] = _DEFAULT_EXCLUDED_HOSTS,
) -> Iterator[None]:
    """Context manager for scoped OpenCode interception."""
    enable_opencode_integration(
        store=store,
        clients=clients,
        include_hosts=include_hosts,
        exclude_hosts=exclude_hosts,
    )
    try:
        yield
    finally:
        disable_opencode_integration()


def _safe_import(module_name: str) -> Any | None:
    try:
        return importlib.import_module(module_name)
    except Exception:
        return None
