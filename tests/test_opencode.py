from __future__ import annotations

import json
from email.message import Message
from urllib import request as urllib_request

import pytest

from context_cutter import (
    InMemoryStore,
    build_opencode_tool_registry,
    disable_opencode_http_interception,
    enable_opencode_http_interception,
    get_opencode_tool_manifest,
    is_opencode_http_interception_enabled,
    opencode_http_interception,
    query_handle,
)


class _FakeResponse:
    def __init__(self, payload: bytes, content_type: str) -> None:
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

    def close(self) -> None:
        return None

    def __enter__(self) -> "_FakeResponse":
        return self

    def __exit__(self, exc_type: object, exc_value: object, traceback: object) -> None:
        return None


@pytest.fixture(autouse=True)
def clean_interceptor_state() -> None:
    disable_opencode_http_interception()
    yield
    disable_opencode_http_interception()


def test_opencode_interception_wraps_json_http_payload(monkeypatch: pytest.MonkeyPatch) -> None:
    store = InMemoryStore()
    source_payload = {
        "contributors": [
            {"login": "octocat", "contributions": 500},
            {"login": "hubot", "contributions": 123},
        ]
    }

    def _fake_urlopen(*args: object, **kwargs: object) -> _FakeResponse:
        return _FakeResponse(
            json.dumps(source_payload).encode("utf-8"),
            "application/json; charset=utf-8",
        )

    monkeypatch.setattr(urllib_request, "urlopen", _fake_urlopen)
    enable_opencode_http_interception(store=store)

    wrapped = urllib_request.urlopen("https://example.test")
    envelope = json.loads(wrapped.read().decode("utf-8"))

    assert set(envelope.keys()) == {"handle_id", "teaser"}
    assert query_handle(envelope["handle_id"], "contributors.0.login", store=store) == "octocat"


def test_opencode_interception_passthrough_non_json(monkeypatch: pytest.MonkeyPatch) -> None:
    source_payload = b"plain-text"

    def _fake_urlopen(*args: object, **kwargs: object) -> _FakeResponse:
        return _FakeResponse(source_payload, "text/plain")

    monkeypatch.setattr(urllib_request, "urlopen", _fake_urlopen)
    enable_opencode_http_interception()

    response = urllib_request.urlopen("https://example.test")
    assert response.read() == source_payload


def test_opencode_interception_enable_disable_idempotent(monkeypatch: pytest.MonkeyPatch) -> None:
    original = urllib_request.urlopen

    def _fake_urlopen(*args: object, **kwargs: object) -> _FakeResponse:
        return _FakeResponse(b"{}", "application/json")

    monkeypatch.setattr(urllib_request, "urlopen", _fake_urlopen)

    assert not is_opencode_http_interception_enabled()
    enable_opencode_http_interception()
    assert is_opencode_http_interception_enabled()
    enable_opencode_http_interception()
    assert is_opencode_http_interception_enabled()

    disable_opencode_http_interception()
    assert not is_opencode_http_interception_enabled()
    assert urllib_request.urlopen is _fake_urlopen
    assert urllib_request.urlopen is not original


def test_opencode_context_manager_scoped(monkeypatch: pytest.MonkeyPatch) -> None:
    def _fake_urlopen(*args: object, **kwargs: object) -> _FakeResponse:
        return _FakeResponse(b'{"ok":true}', "application/json")

    monkeypatch.setattr(urllib_request, "urlopen", _fake_urlopen)
    assert not is_opencode_http_interception_enabled()
    with opencode_http_interception(clients=("urllib",)):
        assert is_opencode_http_interception_enabled()
    assert not is_opencode_http_interception_enabled()


def test_opencode_tool_registry_roundtrip() -> None:
    store = InMemoryStore()
    source_payload = {"items": [{"name": "alpha"}]}

    def _fake_urlopen(*args: object, **kwargs: object) -> _FakeResponse:
        return _FakeResponse(
            json.dumps(source_payload).encode("utf-8"),
            "application/json; charset=utf-8",
        )

    original = urllib_request.urlopen
    urllib_request.urlopen = _fake_urlopen
    try:
        enable_opencode_http_interception(store=store)
        envelope = json.loads(urllib_request.urlopen("https://example.test").read().decode("utf-8"))
    finally:
        disable_opencode_http_interception()
        urllib_request.urlopen = original

    tools = build_opencode_tool_registry(store=store)
    assert set(tools.keys()) == {"query_handle", "query_json_path"}
    assert tools["query_handle"](envelope["handle_id"], "items.0.name") == "alpha"
    assert tools["query_json_path"](envelope["handle_id"], "$.items[0].name") == "alpha"


def test_opencode_tool_manifest_includes_query_tools() -> None:
    manifest = get_opencode_tool_manifest()
    names = {entry["function"]["name"] for entry in manifest}
    assert "query_handle" in names
    assert "query_json_path" in names


def test_opencode_exclude_hosts_skips_interception(monkeypatch: pytest.MonkeyPatch) -> None:
    source_payload = {"contributors": [{"login": "octocat"}]}

    def _fake_urlopen(*args: object, **kwargs: object) -> _FakeResponse:
        return _FakeResponse(
            json.dumps(source_payload).encode("utf-8"),
            "application/json; charset=utf-8",
        )

    monkeypatch.setattr(urllib_request, "urlopen", _fake_urlopen)
    with opencode_http_interception(clients=("urllib",), exclude_hosts=("api.restful-api.dev",)):
        response = urllib_request.urlopen("https://api.restful-api.dev/objects")
        parsed = json.loads(response.read().decode("utf-8"))

    assert parsed == source_payload


def test_opencode_include_hosts_limits_interception(monkeypatch: pytest.MonkeyPatch) -> None:
    source_payload = {"contributors": [{"login": "octocat"}]}

    def _fake_urlopen(*args: object, **kwargs: object) -> _FakeResponse:
        return _FakeResponse(
            json.dumps(source_payload).encode("utf-8"),
            "application/json; charset=utf-8",
        )

    monkeypatch.setattr(urllib_request, "urlopen", _fake_urlopen)
    with opencode_http_interception(clients=("urllib",), include_hosts=("example.test",)):
        response = urllib_request.urlopen("https://api.restful-api.dev/objects")
        parsed = json.loads(response.read().decode("utf-8"))

    assert parsed == source_payload
