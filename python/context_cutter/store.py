"""Storage interfaces and implementations for handle payloads."""

from __future__ import annotations

import json
from abc import ABC, abstractmethod
from threading import RLock
from typing import Any


JsonValue = Any


class BaseStore(ABC):
    """Abstract storage for payloads keyed by handle_id."""

    @abstractmethod
    def set(self, handle_id: str, payload: JsonValue) -> None:
        """Store payload under handle_id."""

    @abstractmethod
    def get(self, handle_id: str) -> JsonValue | None:
        """Get payload by handle_id or None."""

    @abstractmethod
    def delete(self, handle_id: str) -> bool:
        """Delete payload by handle_id; True when removed."""

    @abstractmethod
    def exists(self, handle_id: str) -> bool:
        """Return True when handle_id exists."""

    @abstractmethod
    def clear(self) -> None:
        """Clear all stored entries (best-effort for remote stores)."""

    @abstractmethod
    def len(self) -> int:
        """Return count of entries if efficiently available."""


class InMemoryStore(BaseStore):
    """Thread-safe dict-backed store."""

    def __init__(self) -> None:
        self._data: dict[str, JsonValue] = {}
        self._lock = RLock()

    def set(self, handle_id: str, payload: JsonValue) -> None:
        with self._lock:
            self._data[handle_id] = payload

    def get(self, handle_id: str) -> JsonValue | None:
        with self._lock:
            return self._data.get(handle_id)

    def delete(self, handle_id: str) -> bool:
        with self._lock:
            return self._data.pop(handle_id, None) is not None

    def exists(self, handle_id: str) -> bool:
        with self._lock:
            return handle_id in self._data

    def clear(self) -> None:
        with self._lock:
            self._data.clear()

    def len(self) -> int:
        with self._lock:
            return len(self._data)


class RedisStore(BaseStore):
    """Redis-backed store with JSON serialization at boundary."""

    def __init__(
        self,
        *,
        redis_client: Any | None = None,
        redis_url: str = "redis://localhost:6379/0",
        key_prefix: str = "context-cutter:",
    ) -> None:
        if redis_client is None:
            try:
                import redis  # type: ignore
            except ModuleNotFoundError as exc:
                raise ModuleNotFoundError(
                    "redis package is required for RedisStore"
                ) from exc
            self._redis = redis.Redis.from_url(redis_url, decode_responses=True)
        else:
            self._redis = redis_client
        self._key_prefix = key_prefix

    def _key(self, handle_id: str) -> str:
        return f"{self._key_prefix}{handle_id}"

    def set(self, handle_id: str, payload: JsonValue) -> None:
        self._redis.set(self._key(handle_id), json.dumps(payload))

    def get(self, handle_id: str) -> JsonValue | None:
        raw = self._redis.get(self._key(handle_id))
        if raw is None:
            return None
        return json.loads(raw)

    def delete(self, handle_id: str) -> bool:
        return bool(self._redis.delete(self._key(handle_id)))

    def exists(self, handle_id: str) -> bool:
        return bool(self._redis.exists(self._key(handle_id)))

    def clear(self) -> None:
        # Best effort clear by prefix without flushing entire db.
        for key in self._redis.scan_iter(match=f"{self._key_prefix}*"):
            self._redis.delete(key)

    def len(self) -> int:
        count = 0
        for _ in self._redis.scan_iter(match=f"{self._key_prefix}*"):
            count += 1
        return count


_DEFAULT_STORE: BaseStore = InMemoryStore()


def get_default_store() -> BaseStore:
    """Get process-wide default store."""
    return _DEFAULT_STORE


def set_default_store(store: BaseStore) -> None:
    """Replace process-wide default store."""
    global _DEFAULT_STORE
    _DEFAULT_STORE = store
