"""Python UX decorator compatibility layer."""

from __future__ import annotations

from .interceptor import lazy_handle

# Backward-compatible alias.
lazy_tool = lazy_handle
