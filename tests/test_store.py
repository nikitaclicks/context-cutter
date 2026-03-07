from __future__ import annotations

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


def test_set_default_store_replaces_global_default() -> None:
    original = get_default_store()
    replacement = InMemoryStore()
    try:
        set_default_store(replacement)
        assert get_default_store() is replacement
    finally:
        set_default_store(original)
