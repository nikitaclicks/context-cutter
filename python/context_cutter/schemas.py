"""Pydantic schemas for handle-and-path contracts."""

from __future__ import annotations

from typing import Any

from pydantic import BaseModel, Field


class HandleRef(BaseModel):
    """Reference to a stored payload."""

    handle_id: str = Field(min_length=1)


class LazyHandleResponse(BaseModel):
    """Return shape from lazy interception."""

    handle_id: str = Field(min_length=1)
    teaser: dict[str, Any]


class QueryRequest(BaseModel):
    """Query request for a stored payload."""

    handle_id: str = Field(min_length=1)
    json_path: str = Field(min_length=1)


class QueryResult(BaseModel):
    """Structured query response for tool-facing APIs."""

    handle_id: str = Field(min_length=1)
    json_path: str = Field(min_length=1)
    result: Any
