"""Public Python API for context-cutter."""

from .core import ContextStore, generate_teaser, query_path, store_response
from .interceptor import lazy_handle
from .mcp_server import (
    create_mcp_server,
    fetch_json_cutted,
    query_handle_tool,
    run_mcp_server_stdio,
)
from .query import query_handle
from .store import BaseStore, InMemoryStore, RedisStore, get_default_store, set_default_store
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
    "fetch_json_cutted",
    "query_handle_tool",
    "create_mcp_server",
    "run_mcp_server_stdio",
    "generate_tool_manifest",
]
