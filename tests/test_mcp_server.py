from __future__ import annotations

import json
from email.message import Message
from urllib import request as urllib_request

import pytest

from context_cutter import InMemoryStore
from context_cutter.mcp_server import (
    _load_fastmcp_class,
    fetch_json_cutted,
    query_handle_tool,
)


class _FakeResponse:
    def __init__(self, payload: bytes, content_type: str = "application/json") -> None:
        self._payload = payload
        self._cursor = 0
        headers = Message()
        headers["Content-Type"] = content_type
        self.headers = headers

    def read(self, amt: int = -1) -> bytes:
        if amt is None or amt < 0:
            if self._cursor >= len(self._payload):
                return b""
            out = self._payload[self._cursor :]
            self._cursor = len(self._payload)
            return out
        if self._cursor >= len(self._payload):
            return b""
        end = min(self._cursor + amt, len(self._payload))
        out = self._payload[self._cursor : end]
        self._cursor = end
        return out

    def __enter__(self) -> "_FakeResponse":
        return self

    def __exit__(self, exc_type: object, exc_value: object, traceback: object) -> None:
        return None


def test_fetch_json_cutted_returns_envelope(monkeypatch: pytest.MonkeyPatch) -> None:
    store = InMemoryStore()
    source_payload = {
        "objects": [
            {"id": "1", "name": "Google Pixel 6 Pro"},
            {"id": "2", "name": "Apple iPhone 12 Mini"},
        ]
    }

    def _fake_urlopen(req: object, timeout: float = 45.0) -> _FakeResponse:
        return _FakeResponse(json.dumps(source_payload).encode("utf-8"))

    monkeypatch.setattr(urllib_request, "urlopen", _fake_urlopen)

    envelope = fetch_json_cutted(
        url="https://api.restful-api.dev/objects",
        store=store,
    )

    assert set(envelope.keys()) == {"handle_id", "teaser"}
    count = query_handle_tool(
        handle_id=envelope["handle_id"],
        json_path="$.objects[*]",
        store=store,
    )
    assert isinstance(count, list)
    assert len(count) == 2


def test_fetch_json_cutted_rejects_non_json(monkeypatch: pytest.MonkeyPatch) -> None:
    def _fake_urlopen(req: object, timeout: float = 45.0) -> _FakeResponse:
        return _FakeResponse(b"not-json", "text/plain")

    monkeypatch.setattr(urllib_request, "urlopen", _fake_urlopen)

    with pytest.raises(ValueError):
        fetch_json_cutted(url="https://api.restful-api.dev/objects")


def test_load_fastmcp_class_missing_dependency(monkeypatch: pytest.MonkeyPatch) -> None:
    import builtins

    real_import = builtins.__import__

    def _blocked_import(name: str, globals: object = None, locals: object = None, fromlist: object = (), level: int = 0):
        if name.startswith("mcp"):
            raise ModuleNotFoundError("blocked for test")
        return real_import(name, globals, locals, fromlist, level)

    monkeypatch.setattr(builtins, "__import__", _blocked_import)
    with pytest.raises(ModuleNotFoundError):
        _load_fastmcp_class()
