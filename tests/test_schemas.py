from __future__ import annotations

import pytest
from pydantic import ValidationError

from context_cutter.schemas import LazyHandleResponse, QueryRequest


def test_query_request_requires_non_empty_fields() -> None:
    with pytest.raises(ValidationError):
        QueryRequest(handle_id="", json_path="$.x")
    with pytest.raises(ValidationError):
        QueryRequest(handle_id="hdl_123", json_path="")


def test_lazy_handle_response_requires_handle_id() -> None:
    with pytest.raises(ValidationError):
        LazyHandleResponse(handle_id="", teaser={})

