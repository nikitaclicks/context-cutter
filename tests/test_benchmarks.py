from __future__ import annotations

import json
from pathlib import Path
from typing import Any

import pytest

from context_cutter import generate_teaser, query_handle, store_response

FIXTURE_DIR = Path(__file__).parent / "fixtures" / "evals"


def _fixture_payload(name: str) -> Any:
    return json.loads((FIXTURE_DIR / name).read_text(encoding="utf-8"))


def _token_efficiency_ratio(payload: Any) -> float:
    handle = store_response(payload)
    teaser = json.loads(generate_teaser(handle))
    full_len = len(json.dumps(payload, separators=(",", ":")))
    teaser_len = len(json.dumps({"handle_id": handle, "teaser": teaser}, separators=(",", ":")))
    return teaser_len / full_len


@pytest.mark.benchmark
def test_benchmark_store_response_small(benchmark: Any) -> None:
    payload = _fixture_payload("mixed_scalar_and_arrays.json")
    benchmark(lambda: store_response(payload))


@pytest.mark.benchmark
def test_benchmark_generate_teaser_medium(benchmark: Any) -> None:
    payload = _fixture_payload("github_contributors.json")
    handle = store_response(payload)
    benchmark(lambda: generate_teaser(handle))


@pytest.mark.benchmark
def test_benchmark_query_handle_wildcard(benchmark: Any) -> None:
    payload = _fixture_payload("nested_metrics.json")
    handle = store_response(payload)
    benchmark(lambda: query_handle(handle, "$.regions[*].stats.p95_ms"))


@pytest.mark.benchmark
def test_benchmark_token_efficiency_guardrail() -> None:
    # Use a realistically large payload where teaser summarization should shine.
    payload = {
        "items": [
            {
                "id": i,
                "name": f"user-{i}",
                "bio": "x" * 300,
                "score": i * 17,
                "active": i % 2 == 0,
            }
            for i in range(200)
        ],
        "meta": {"total": 200, "source": "synthetic"},
    }
    ratio = _token_efficiency_ratio(payload)
    # Teaser + handle should be smaller than the full payload.
    assert ratio < 1.0
