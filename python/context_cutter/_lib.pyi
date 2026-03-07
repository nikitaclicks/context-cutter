"""
Type stubs for context_cutter._lib (Rust/PyO3 extension module).

These stubs enable IDE autocompletion and static type checking for the
compiled Rust extension. The runtime implementation lives in src/lib.rs
and src/store.rs, compiled via Maturin with --features python.
"""

class ContextStore:
    """Explicit in-memory store for JSON payloads.

    An alternative to the default process-wide DashMap store. Useful when
    you need isolated stores per session, custom eviction, or test isolation.

    Example::

        from context_cutter import ContextStore, store_response, query_path

        my_store = ContextStore()
        my_store.insert("hdl_abc123", '{"users": [{"id": 1, "name": "Alice"}]}')
        result = query_path("hdl_abc123", "$.users[0].name", store=my_store)
    """

    def __init__(self) -> None:
        """Create a new empty in-memory context store."""
        ...

    def insert(self, handle_id: str, json_str: str) -> None:
        """Parse ``json_str`` and store it under ``handle_id``.

        Args:
            handle_id: Identifier for the stored payload (e.g. ``"hdl_abc123"``).
            json_str: Valid JSON string to store.

        Raises:
            ValueError: If ``json_str`` is not valid JSON.
        """
        ...

    def get(self, handle_id: str) -> str | None:
        """Return the serialized JSON string for ``handle_id``, or ``None``.

        Args:
            handle_id: Identifier for the stored payload.

        Returns:
            Compact JSON string, or ``None`` if not found.

        Raises:
            ValueError: If the stored value cannot be re-serialized.
        """
        ...

    def len(self) -> int:
        """Return the number of stored entries."""
        ...

    def is_empty(self) -> bool:
        """Return ``True`` when the store contains no entries."""
        ...

    def clear(self) -> None:
        """Remove all entries from the store."""
        ...

    def remove(self, handle_id: str) -> bool:
        """Remove a handle from the store.

        Args:
            handle_id: Identifier to remove.

        Returns:
            ``True`` if an entry was removed, ``False`` if it did not exist.
        """
        ...

def store_response(json_str: str) -> str:
    """Store a JSON payload in the global in-memory store and return a handle ID.

    The handle ID is a deterministic SHA-256 digest of the canonicalized
    (key-sorted) JSON, prefixed ``hdl_``. Storing the same logical payload
    twice always returns the same handle ID.

    Args:
        json_str: A valid JSON string to store.

    Returns:
        A handle ID of the form ``"hdl_<12hex>"``.

    Raises:
        ValueError: If ``json_str`` is not valid JSON or contains null bytes.

    Example::

        import json
        from context_cutter._lib import store_response

        data = json.dumps({"users": [{"id": 1, "name": "Alice"}]})
        handle = store_response(data)
        # handle == "hdl_a3f9c2e18b04"
    """
    ...

def generate_teaser(handle_id: str) -> str:
    """Generate a structural teaser for a stored payload and return it as JSON.

    The teaser describes the *shape* of the data (keys, array lengths, scalar
    previews) without including raw values. It is designed to be small enough
    to include in an LLM context window as a planning aid.

    Args:
        handle_id: A handle returned by :func:`store_response`.

    Returns:
        A JSON string containing the teaser structure.

    Raises:
        KeyError: If ``handle_id`` does not exist in the store.
        ValueError: If ``handle_id`` is empty.

    Example::

        from context_cutter._lib import store_response, generate_teaser
        import json

        handle = store_response('[{"id": 1}, {"id": 2}]')
        teaser = json.loads(generate_teaser(handle))
        # {"_teaser": true, "_type": "Array[2]", "item_keys": ["id"]}
    """
    ...

def query_path(handle_id: str, json_path: str) -> str:
    """Run a JSONPath query against a stored payload and return matches as JSON.

    Args:
        handle_id: A handle returned by :func:`store_response`.
        json_path: A JSONPath expression (e.g. ``"$.users[0].email"``).
            Supports dot notation, bracket notation, and wildcards (``[*]``).

    Returns:
        A JSON string containing the matched value(s).
        Returns ``"null"`` when the path matches nothing.

    Raises:
        KeyError: If ``handle_id`` does not exist in the store.
        ValueError: If either argument is empty or the path is malformed.

    Example::

        from context_cutter._lib import store_response, query_path
        import json

        handle = store_response('{"users": [{"email": "a@b.com"}, {"email": "c@d.com"}]}')
        result = json.loads(query_path(handle, "$.users[*].email"))
        # ["a@b.com", "c@d.com"]
    """
    ...
