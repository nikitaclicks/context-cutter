"""Integration test for context-cutter-mcp --proxy mode.

Spins up a minimal in-process HTTP MCP server, runs the binary in proxy
mode, and verifies that large tool responses are intercepted with a
handle + preview while small responses pass through.
"""
from __future__ import annotations

import json
import subprocess
import threading
from http.server import BaseHTTPRequestHandler, HTTPServer
from pathlib import Path

import pytest

REPO_ROOT = Path(__file__).parent.parent
BINARY = REPO_ROOT / "target" / "debug" / "context-cutter-mcp"

# A large payload that will definitely exceed the 2048-byte default threshold.
LARGE_PAYLOAD = {
    "items": [{"id": i, "name": f"item-{i}", "data": "x" * 50} for i in range(50)]
}

TOOLS = [
    {
        "name": "get_items",
        "description": "Returns a large list of items.",
        "inputSchema": {"type": "object", "properties": {}, "required": []},
    }
]


class MockMcpHandler(BaseHTTPRequestHandler):
    """Minimal HTTP MCP server handling initialize, tools/list, tools/call."""

    def log_message(self, *args):  # silence HTTP server logs during tests
        pass

    def do_POST(self):
        length = int(self.headers.get("Content-Length", 0))
        body = json.loads(self.rfile.read(length))
        method = body.get("method")
        req_id = body.get("id")

        if method == "initialize":
            result = {
                "protocolVersion": "2024-11-05",
                "capabilities": {"tools": {}},
                "serverInfo": {"name": "mock-mcp", "version": "0.0.1"},
            }
        elif method == "tools/list":
            result = {"tools": TOOLS}
        elif method == "tools/call":
            result = {
                "content": [{"type": "text", "text": json.dumps(LARGE_PAYLOAD)}],
                "isError": False,
            }
        else:
            result = {}

        response = json.dumps(
            {"jsonrpc": "2.0", "id": req_id, "result": result}
        ).encode()
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(response)))
        self.end_headers()
        self.wfile.write(response)


def _start_mock_server() -> tuple[HTTPServer, int]:
    server = HTTPServer(("127.0.0.1", 0), MockMcpHandler)
    port = server.server_address[1]
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    return server, port


def _send_notification(proc: subprocess.Popen, method: str, params: dict | None = None) -> None:
    """Send a JSON-RPC notification (no id, no response expected)."""
    msg: dict = {"jsonrpc": "2.0", "method": method}
    if params is not None:
        msg["params"] = params
    line = json.dumps(msg) + "\n"
    proc.stdin.write(line.encode())
    proc.stdin.flush()


def _send_mcp(proc: subprocess.Popen, message: dict) -> dict:
    line = json.dumps(message) + "\n"
    proc.stdin.write(line.encode())
    proc.stdin.flush()
    # The binary may emit log lines to stdout before the JSON response;
    # skip any lines that don't start with '{'.
    while True:
        raw = proc.stdout.readline()
        if not raw:
            raise EOFError("Binary closed stdout without sending a response")
        decoded = raw.decode(errors="replace").strip()
        if decoded.startswith("{"):
            return json.loads(decoded)


def _do_handshake(proc: subprocess.Popen) -> dict:
    """Send initialize + initialized notification, return initialize response."""
    init_resp = _send_mcp(
        proc,
        {
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": {"name": "test", "version": "0"},
            },
        },
    )
    # MCP spec requires an `initialized` notification after the response.
    _send_notification(proc, "notifications/initialized")
    return init_resp


@pytest.mark.integration
def test_proxy_intercepts_large_response():
    if not BINARY.exists():
        pytest.skip(
            "binary not built — run `cargo build --bin context-cutter-mcp` first"
        )

    server, port = _start_mock_server()
    upstream_url = f"http://127.0.0.1:{port}/mcp"

    try:
        proc = subprocess.Popen(
            [str(BINARY), "--proxy", upstream_url, "--proxy-threshold", "100"],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
        )

        try:
            # MCP handshake (initialize + initialized notification)
            init_resp = _do_handshake(proc)
            assert "result" in init_resp

            # List tools — should include get_items + query_handle
            tools_resp = _send_mcp(
                proc,
                {"jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {}},
            )
            tool_names = [t["name"] for t in tools_resp["result"]["tools"]]
            assert "get_items" in tool_names
            assert "query_handle" in tool_names

            # Call the large tool — should be intercepted
            call_resp = _send_mcp(
                proc,
                {
                    "jsonrpc": "2.0",
                    "id": 3,
                    "method": "tools/call",
                    "params": {"name": "get_items", "arguments": {}},
                },
            )
            text = call_resp["result"]["content"][0]["text"]
            assert "[context-cutter]" in text
            assert "hdl_" in text
            assert "query_handle" in text

            # Extract handle_id and call query_handle locally
            handle_id = next(
                word for word in text.split() if word.startswith("hdl_")
            ).rstrip(")")
            query_resp = _send_mcp(
                proc,
                {
                    "jsonrpc": "2.0",
                    "id": 4,
                    "method": "tools/call",
                    "params": {
                        "name": "query_handle",
                        "arguments": {
                            "handle_id": handle_id,
                            "json_path": "$.items[0].id",
                        },
                    },
                },
            )
            assert query_resp["result"]["content"][0]["text"] == "0"

        finally:
            proc.terminate()
            proc.wait(timeout=5)

    finally:
        server.shutdown()


@pytest.mark.integration
def test_proxy_passes_through_small_response():
    if not BINARY.exists():
        pytest.skip(
            "binary not built — run `cargo build --bin context-cutter-mcp` first"
        )

    server, port = _start_mock_server()
    upstream_url = f"http://127.0.0.1:{port}/mcp"

    try:
        proc = subprocess.Popen(
            [
                str(BINARY),
                "--proxy",
                upstream_url,
                "--proxy-threshold",
                "999999",
            ],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
        )

        try:
            _do_handshake(proc)
            call_resp = _send_mcp(
                proc,
                {
                    "jsonrpc": "2.0",
                    "id": 2,
                    "method": "tools/call",
                    "params": {"name": "get_items", "arguments": {}},
                },
            )
            text = call_resp["result"]["content"][0]["text"]
            assert "[context-cutter]" not in text  # passed through

        finally:
            proc.terminate()
            proc.wait(timeout=5)

    finally:
        server.shutdown()
