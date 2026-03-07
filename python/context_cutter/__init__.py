"""Public Python API for context-cutter."""

from ._lib import ContextStore
from .core import generate_teaser, query_path, store_response
from .interceptor import lazy_handle
from .query import query_handle
from .store import (
    BaseStore,
    InMemoryStore,
    RedisStore,
    get_default_store,
    set_default_store,
)
from .tools import generate_tool_manifest
from .wrapper import lazy_tool

__all__ = [
    "BaseStore",
    "InMemoryStore",
    "RedisStore",
    "get_default_store",
    "set_default_store",
    "ContextStore",
    "store_response",
    "generate_teaser",
    "query_path",
    "query_handle",
    "lazy_handle",
    "lazy_tool",
    "generate_tool_manifest",
]
