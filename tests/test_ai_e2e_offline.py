from __future__ import annotations

from concurrent.futures import ThreadPoolExecutor

import pytest

from context_cutter import InMemoryStore, lazy_tool, set_default_store
from tests.ai_harness import FinalAnswer, ToolCall, run_agent_loop, teaser_ratio


@pytest.fixture(autouse=True)
def isolated_default_store() -> None:
    set_default_store(InMemoryStore())


@pytest.mark.ai_e2e_offline
def test_ai_e2e_agent_loop_happy_path() -> None:
    payload = {
        "service": "billing",
        "regions": [
            {"name": "us-east-1", "stats": {"p95_ms": 80.1}},
            {"name": "eu-west-1", "stats": {"p95_ms": 96.7}},
        ],
    }

    @lazy_tool
    def get_metrics() -> dict[str, object]:
        return payload

    envelope = get_metrics()
    handle_id = envelope["handle_id"]

    def planner(transcript: list[dict[str, object]], _errors: list[Exception]) -> ToolCall | FinalAnswer:
        if not transcript:
            return ToolCall("query_handle", {"handle_id": handle_id, "json_path": "service"})
        if len(transcript) == 1:
            return ToolCall(
                "query_json_path",
                {"handle_id": handle_id, "json_path": "$.regions[*].stats.p95_ms"},
            )
        service = transcript[0]["output"]
        p95_values = transcript[1]["output"]
        return FinalAnswer({"service": service, "max_p95_ms": max(p95_values)})

    result = run_agent_loop(planner)

    assert result["final"] == {"service": "billing", "max_p95_ms": 96.7}
    assert len(result["transcript"]) == 2


@pytest.mark.ai_e2e_offline
def test_ai_e2e_agent_loop_recovers_after_invalid_path() -> None:
    payload = {
        "contributors": [
            {"login": "octocat"},
            {"login": "hubot"},
        ]
    }

    @lazy_tool
    def get_contributors() -> dict[str, object]:
        return payload

    envelope = get_contributors()
    handle_id = envelope["handle_id"]

    def planner(transcript: list[dict[str, object]], errors: list[Exception]) -> ToolCall | FinalAnswer:
        if not transcript and not errors:
            return ToolCall("query_handle", {"handle_id": handle_id, "json_path": "$.['"})
        if errors and not transcript:
            return ToolCall(
                "query_handle",
                {"handle_id": handle_id, "json_path": "$.contributors[0].login"},
            )
        return FinalAnswer({"top_contributor": transcript[0]["output"]})

    result = run_agent_loop(planner)

    assert result["final"] == {"top_contributor": "octocat"}
    assert len(result["errors"]) == 1
    assert isinstance(result["errors"][0], ValueError)


@pytest.mark.ai_e2e_offline
def test_ai_e2e_agent_loop_store_parity_default_and_custom() -> None:
    payload = {
        "flags": {"enabled": True},
        "owner": {"name": "team-core"},
    }

    @lazy_tool
    def default_store_tool() -> dict[str, object]:
        return payload

    custom_store = InMemoryStore()

    @lazy_tool(store=custom_store)
    def custom_store_tool() -> dict[str, object]:
        return payload

    default_envelope = default_store_tool()
    custom_envelope = custom_store_tool()

    def plan_for(handle_id: str):
        def planner(transcript: list[dict[str, object]], _errors: list[Exception]) -> ToolCall | FinalAnswer:
            if not transcript:
                return ToolCall("query_handle", {"handle_id": handle_id, "json_path": "owner.name"})
            return FinalAnswer(transcript[0]["output"])

        return planner

    default_result = run_agent_loop(plan_for(default_envelope["handle_id"]))
    custom_result = run_agent_loop(plan_for(custom_envelope["handle_id"]), store=custom_store)

    assert default_result["final"] == "team-core"
    assert custom_result["final"] == "team-core"


@pytest.mark.ai_e2e_offline
def test_ai_e2e_parallel_sessions_are_isolated() -> None:
    @lazy_tool
    def get_payload(login: str) -> dict[str, object]:
        return {"user": {"login": login}, "events": [1, 2, 3]}

    first = get_payload("alpha")
    second = get_payload("beta")

    def fetch_login(handle_id: str) -> str:
        def planner(transcript: list[dict[str, object]], _errors: list[Exception]) -> ToolCall | FinalAnswer:
            if not transcript:
                return ToolCall("query_handle", {"handle_id": handle_id, "json_path": "user.login"})
            return FinalAnswer(transcript[0]["output"])

        return run_agent_loop(planner)["final"]

    with ThreadPoolExecutor(max_workers=2) as pool:
        first_future = pool.submit(fetch_login, first["handle_id"])
        second_future = pool.submit(fetch_login, second["handle_id"])

    assert first["handle_id"] != second["handle_id"]
    assert first_future.result() == "alpha"
    assert second_future.result() == "beta"


@pytest.mark.ai_e2e_offline
def test_ai_e2e_token_efficiency_threshold() -> None:
    payload = {
        "items": [
            {
                "id": i,
                "name": f"user-{i}",
                "bio": "x" * 300,
                "score": i * 7,
                "active": i % 2 == 0,
            }
            for i in range(120)
        ],
        "meta": {"source": "synthetic", "total": 120},
    }

    @lazy_tool
    def get_large_payload() -> dict[str, object]:
        return payload

    envelope = get_large_payload()
    ratio = teaser_ratio(payload, envelope)

    assert ratio <= 0.50
