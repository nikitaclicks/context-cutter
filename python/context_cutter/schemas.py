"""Pydantic schemas for handle-and-path contracts."""

from __future__ import annotations

from typing import Any

from pydantic import BaseModel, Field


class LazyHandleResponse(BaseModel):
    """Return shape from lazy interception."""

    handle_id: str = Field(min_length=1)
    teaser: dict[str, Any]


class QueryRequest(BaseModel):
    """Query request for a stored payload."""

    handle_id: str = Field(min_length=1)
    json_path: str = Field(min_length=1)
