from __future__ import annotations

import pytest

from context_cutter.store import (
    InMemoryStore,
    RedisStore,
    get_default_store,
    set_default_store,
)


class FakeRedis:
    def __init__(self) -> None:
        self._data: dict[str, str] = {}

    def set(self, key: str, value: str) -> None:
        self._data[key] = value

    def get(self, key: str) -> str | None:
        return self._data.get(key)

    def delete(self, key: str) -> int:
        existed = key in self._data
        self._data.pop(key, None)
        return 1 if existed else 0

    def exists(self, key: str) -> int:
        return 1 if key in self._data else 0

    def scan_iter(self, match: str) -> list[str]:
        prefix = match[:-1] if match.endswith("*") else match
        return [key for key in self._data if key.startswith(prefix)]


def test_in_memory_store_crud() -> None:
    store = InMemoryStore()
    store.set("h1", {"x": 1})
    assert store.get("h1") == {"x": 1}
    assert store.exists("h1") is True
    assert store.len() == 1
    assert store.delete("h1") is True
    assert store.get("h1") is None
    assert store.delete("h1") is False


def test_in_memory_store_clear() -> None:
    store = InMemoryStore()
    store.set("a", 1)
    store.set("b", 2)
    assert store.len() == 2
    store.clear()
    assert store.len() == 0


def test_redis_store_with_injected_client() -> None:
    fake = FakeRedis()
    store = RedisStore(redis_client=fake, key_prefix="cc:")
    store.set("h1", {"ok": True})
    assert store.get("h1") == {"ok": True}
    assert store.exists("h1") is True
    assert store.len() == 1
    assert store.delete("h1") is True
    assert store.exists("h1") is False


def test_redis_store_get_returns_none_for_missing_key() -> None:
    fake = FakeRedis()
    store = RedisStore(redis_client=fake, key_prefix="cc:")
    assert store.get("not_here") is None


def test_redis_store_clear_removes_all_entries() -> None:
    fake = FakeRedis()
    store = RedisStore(redis_client=fake, key_prefix="cc:")
    store.set("a", {"x": 1})
    store.set("b", {"y": 2})
    assert store.len() == 2
    store.clear()
    assert store.len() == 0
    assert store.get("a") is None


def test_redis_store_raises_when_redis_not_importable() -> None:
    import sys
    import unittest.mock as mock

    with mock.patch.dict(sys.modules, {"redis": None}):
        with pytest.raises(ModuleNotFoundError, match="redis package is required"):
            RedisStore()


def test_redis_store_uses_from_url_when_no_client_provided() -> None:
    import sys
    import unittest.mock as mock

    fake_redis_module = mock.MagicMock()
    with mock.patch.dict(sys.modules, {"redis": fake_redis_module}):
        store = RedisStore(redis_url="redis://fake-host:6379/0")
        fake_redis_module.Redis.from_url.assert_called_once_with(
            "redis://fake-host:6379/0", decode_responses=True
        )
        assert store is not None


def test_set_default_store_replaces_global_default() -> None:
    original = get_default_store()
    replacement = InMemoryStore()
    try:
        set_default_store(replacement)
        assert get_default_store() is replacement
    finally:
        set_default_store(original)
