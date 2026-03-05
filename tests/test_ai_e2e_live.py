from __future__ import annotations

import json
import os
import urllib.error
import urllib.request
from typing import Any

import pytest

from context_cutter import InMemoryStore, lazy_tool, query_handle, set_default_store
from context_cutter.tools import generate_tool_manifest


@pytest.fixture(autouse=True)
def isolated_default_store() -> None:
    set_default_store(InMemoryStore())


def _openai_chat_completion(payload: dict[str, Any], api_key: str) -> dict[str, Any]:
    req = urllib.request.Request(
        "https://api.openai.com/v1/chat/completions",
        data=json.dumps(payload).encode("utf-8"),
        headers={
            "Authorization": f"Bearer {api_key}",
            "Content-Type": "application/json",
        },
        method="POST",
    )
    with urllib.request.urlopen(req, timeout=45) as response:
        return json.loads(response.read().decode("utf-8"))


def _gemini_generate_content(
    payload: dict[str, Any],
    *,
    api_key: str,
    model: str,
) -> dict[str, Any]:
    req = urllib.request.Request(
        f"https://generativelanguage.googleapis.com/v1beta/models/{model}:generateContent?key={api_key}",
        data=json.dumps(payload).encode("utf-8"),
        headers={"Content-Type": "application/json"},
        method="POST",
    )
    with urllib.request.urlopen(req, timeout=45) as response:
        return json.loads(response.read().decode("utf-8"))


def _manifest_to_gemini_function_declarations(
    tools: list[dict[str, Any]],
) -> list[dict[str, Any]]:
    declarations: list[dict[str, Any]] = []
    for tool in tools:
        fn = tool.get("function", {})
        declarations.append(
            {
                "name": fn.get("name"),
                "description": fn.get("description", ""),
                "parameters": fn.get("parameters", {"type": "object"}),
            }
        )
    return declarations


def _first_gemini_function_call(response: dict[str, Any]) -> dict[str, Any]:
    parts = response.get("candidates", [{}])[0].get("content", {}).get("parts", [])
    for part in parts:
        function_call = part.get("functionCall")
        if function_call:
            return function_call
    raise AssertionError("expected at least one functionCall part from Gemini")


def _gemini_text(response: dict[str, Any]) -> str:
    parts = response.get("candidates", [{}])[0].get("content", {}).get("parts", [])
    text_chunks = [part.get("text", "") for part in parts if "text" in part]
    return "\n".join(chunk for chunk in text_chunks if chunk).strip()


@pytest.mark.ai_e2e_live
def test_ai_e2e_live_tool_call_roundtrip() -> None:
    provider = os.getenv("CONTEXT_CUTTER_LIVE_PROVIDER", "openai").strip().lower()

    payload = {
        "contributors": [
            {"id": 1, "login": "octocat", "contributions": 500},
            {"id": 2, "login": "hubot", "contributions": 123},
        ],
        "meta": {"total": 2},
    }

    @lazy_tool
    def get_github_contributors() -> dict[str, object]:
        return payload

    envelope = get_github_contributors()
    handle_id = envelope["handle_id"]

    tools = generate_tool_manifest()
    system_prompt = (
        "You are a precise agent. Use the available tool to read data from the handle. "
        "Do not guess values."
    )
    user_prompt = (
        "Return the login of the top contributor only. "
        f"handle_id={handle_id}. Output just the login as plain text."
    )

    if provider == "openai":
        api_key = os.getenv("OPENAI_API_KEY")
        if not api_key:
            pytest.skip("OPENAI_API_KEY is required for live OpenAI e2e smoke test")

        model = os.getenv("CONTEXT_CUTTER_OPENAI_MODEL", "gpt-4.1-mini")
        first_request = {
            "model": model,
            "messages": [
                {"role": "system", "content": system_prompt},
                {"role": "user", "content": user_prompt},
            ],
            "tools": tools,
            "tool_choice": "required",
            "temperature": 0,
        }

        try:
            first = _openai_chat_completion(first_request, api_key)
        except urllib.error.HTTPError as exc:
            body = exc.read().decode("utf-8", errors="replace")
            pytest.fail(f"live provider request failed ({exc.code}): {body}")

        message = first["choices"][0]["message"]
        tool_calls = message.get("tool_calls", [])
        assert tool_calls, "expected at least one tool call from model"

        tool_call = tool_calls[0]
        function_name = tool_call["function"]["name"]
        tool_args = json.loads(tool_call["function"]["arguments"])

        assert function_name in {"query_handle", "query_json_path"}
        tool_output = query_handle(tool_args["handle_id"], tool_args["json_path"])

        second_request = {
            "model": model,
            "messages": [
                {"role": "system", "content": system_prompt},
                {"role": "user", "content": user_prompt},
                {
                    "role": "assistant",
                    "content": message.get("content") or "",
                    "tool_calls": tool_calls,
                },
                {
                    "role": "tool",
                    "tool_call_id": tool_call["id"],
                    "content": json.dumps(tool_output),
                },
            ],
            "temperature": 0,
        }

        try:
            second = _openai_chat_completion(second_request, api_key)
        except urllib.error.HTTPError as exc:
            body = exc.read().decode("utf-8", errors="replace")
            pytest.fail(f"live provider follow-up failed ({exc.code}): {body}")

        final_text = (second["choices"][0]["message"].get("content") or "").strip().lower()
    elif provider == "gemini":
        api_key = os.getenv("GEMINI_API_KEY")
        if not api_key:
            pytest.skip("GEMINI_API_KEY is required for live Gemini e2e smoke test")

        model = os.getenv("CONTEXT_CUTTER_GEMINI_MODEL", "gemini-1.5-pro")
        declarations = _manifest_to_gemini_function_declarations(tools)
        first_request = {
            "systemInstruction": {"parts": [{"text": system_prompt}]},
            "contents": [{"role": "user", "parts": [{"text": user_prompt}]}],
            "tools": [{"functionDeclarations": declarations}],
            "generationConfig": {"temperature": 0},
        }

        try:
            first = _gemini_generate_content(first_request, api_key=api_key, model=model)
        except urllib.error.HTTPError as exc:
            body = exc.read().decode("utf-8", errors="replace")
            pytest.fail(f"live provider request failed ({exc.code}): {body}")

        function_call = _first_gemini_function_call(first)
        function_name = function_call["name"]
        tool_args = function_call.get("args", {})

        assert function_name in {"query_handle", "query_json_path"}
        tool_output = query_handle(tool_args["handle_id"], tool_args["json_path"])

        second_request = {
            "systemInstruction": {"parts": [{"text": system_prompt}]},
            "contents": [
                {"role": "user", "parts": [{"text": user_prompt}]},
                {"role": "model", "parts": [{"functionCall": function_call}]},
                {
                    "role": "user",
                    "parts": [
                        {
                            "functionResponse": {
                                "name": function_name,
                                "response": {"result": tool_output},
                            }
                        }
                    ],
                },
            ],
            "tools": [{"functionDeclarations": declarations}],
            "generationConfig": {"temperature": 0},
        }

        try:
            second = _gemini_generate_content(second_request, api_key=api_key, model=model)
        except urllib.error.HTTPError as exc:
            body = exc.read().decode("utf-8", errors="replace")
            pytest.fail(f"live provider follow-up failed ({exc.code}): {body}")

        final_text = _gemini_text(second).lower()
    else:
        pytest.skip(f"unsupported CONTEXT_CUTTER_LIVE_PROVIDER: {provider}")

    assert "octocat" in final_text
