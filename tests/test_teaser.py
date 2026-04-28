from __future__ import annotations

from context_cutter.teaser import _small_scalar, _summarize, generate_teaser_map


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


def test_generate_teaser_map_for_list_payload() -> None:
    payload = [{"login": "octocat"}, {"login": "hubot"}]
    teaser = generate_teaser_map(payload)
    assert teaser["_teaser"] is True
    assert teaser["_type"] == "Array[2]"
    assert "structure" in teaser


def test_generate_teaser_map_for_scalar_payload() -> None:
    teaser = generate_teaser_map(42)
    assert teaser["_teaser"] is True
    assert teaser["_type"] == "int"
    assert "structure" in teaser


def test_generate_teaser_map_for_empty_list() -> None:
    teaser = generate_teaser_map([])
    assert teaser["_teaser"] is True
    assert teaser["_type"] == "Array[0]"
    assert teaser["structure"] == "Array[0]"


def test_small_scalar_preserves_bool_and_none() -> None:
    assert _small_scalar(True) is True
    assert _small_scalar(False) is False
    assert _small_scalar(None) is None


def test_small_scalar_handles_float() -> None:
    assert _small_scalar(3.14) == 3.14
    assert _small_scalar(99999.9) == "float"


def test_small_scalar_returns_none_for_unknown_type() -> None:
    assert _small_scalar(object()) is None


def test_summarize_empty_list_at_depth() -> None:
    result = _summarize([], depth=0, max_depth=3)
    assert result == "Array[0]"


def test_summarize_list_of_lists() -> None:
    result = _summarize([[1, 2], [3, 4]], depth=0, max_depth=3)
    assert result["_type"] == "Array[2]"
    assert "item" in result


def test_summarize_at_max_depth_list() -> None:
    result = _summarize([1, 2, 3], depth=3, max_depth=3)
    assert result == "Array[3]"


def test_summarize_at_max_depth_scalar() -> None:
    result = _summarize(42, depth=3, max_depth=3)
    assert result == 42


def test_summarize_at_max_depth_large_scalar() -> None:
    result = _summarize(99999, depth=3, max_depth=3)
    assert result == "int"


def test_summarize_unknown_type_falls_back_to_class_name() -> None:
    class _CustomType:
        pass

    result = _summarize(_CustomType(), depth=0, max_depth=3)
    assert result == "_CustomType"

