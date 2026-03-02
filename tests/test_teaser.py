from __future__ import annotations

from context_cutter.teaser import generate_teaser_map


def test_generate_teaser_map_for_object_payload() -> None:
    teaser = generate_teaser_map({"meta": {"total": 2}, "items": [{"id": 1}]})
    assert teaser["_teaser"] is True
    assert teaser["_type"] == "object"
    assert "items" in teaser["keys"]
    assert teaser["structure"]["items"]["_type"] == "Array[1]"


def test_generate_teaser_map_truncates_large_scalars() -> None:
    teaser = generate_teaser_map({"n": 999999, "s": "x" * 40})
    assert teaser["structure"]["n"] == "int"
    assert teaser["structure"]["s"] == "string"


def test_generate_teaser_map_honors_max_depth() -> None:
    payload = {"a": {"b": {"c": {"d": 1}}}}
    teaser = generate_teaser_map(payload, max_depth=2)
    assert teaser["structure"]["a"]["b"] == "{...}"

