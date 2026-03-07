# ContextCutter + Claude Desktop

This example shows how ContextCutter integrates with Claude Desktop to reduce the tokens Claude spends reading API responses during tool-use sessions.

## Configuration

Edit `~/Library/Application Support/Claude/claude_desktop_config.json` (macOS) or `%APPDATA%\Claude\claude_desktop_config.json` (Windows):

```json
{
  "mcpServers": {
    "context-cutter": {
      "command": "npx",
      "args": ["-y", "context-cutter-mcp"]
    }
  }
}
```

If you have the binary installed locally (faster startup, no Node.js required):

```json
{
  "mcpServers": {
    "context-cutter": {
      "command": "/usr/local/bin/context-cutter-mcp"
    }
  }
}
```

Restart Claude Desktop after saving. The `fetch_json_cutted` and `query_handle` tools will appear in Claude's tool list automatically — no system prompt changes needed.

## Example session

**User prompt:**

```
Use the Stripe API to list my recent payment intents and tell me the
total amount charged across all of them. My API key is sk_test_...
```

**Without ContextCutter**, Claude calls the Stripe API and the full response — potentially 50–100 payment intent objects, each with 40+ fields — lands directly in the context window:

```
[full Stripe API response, ~30,000 characters]
```

Claude reads all of it to sum three numbers. Every token of that JSON counts against the context budget and increases latency and cost.

**With ContextCutter**, the session looks like this:

---

*Claude calls `fetch_json_cutted`:*

```json
{
  "url": "https://api.stripe.com/v1/payment_intents?limit=10",
  "headers": {
    "Authorization": "Bearer sk_test_..."
  }
}
```

*ContextCutter server fetches the Stripe response, stores 28,400 chars, returns 289 chars:*

```json
{
  "handle_id": "hdl_7c4e1a93b2f0",
  "teaser": {
    "_teaser": true,
    "_type": "object",
    "keys": ["data", "has_more", "object", "url"],
    "structure": {
      "data": {
        "_type": "Array[10]",
        "item_keys": ["amount", "amount_capturable", "amount_details",
                      "amount_received", "application", "application_fee_amount",
                      "canceled_at", "cancellation_reason", "capture_method",
                      "charges", "client_secret", "confirmation_method",
                      "created", "currency", "customer", "description", "id",
                      "livemode", "metadata", "next_action", "object",
                      "payment_method", "payment_method_types", "receipt_email",
                      "review", "setup_future_usage", "shipping", "source",
                      "statement_descriptor", "status", "transfer_data",
                      "transfer_group"]
      },
      "has_more": false,
      "object": "list",
      "url": "/v1/payment_intents"
    }
  }
}
```

*Claude reads the teaser: there are 10 payment intents and each has an `amount` and `currency` field. It queries all amounts at once with a wildcard:*

```json
{
  "handle_id": "hdl_7c4e1a93b2f0",
  "json_path": "$.data[*].amount"
}
```

*Server returns:*

```json
[2000, 5000, 1500, 8999, 3500, 12000, 500, 7250, 4100, 9999]
```

*Claude confirms currency:*

```json
{
  "handle_id": "hdl_7c4e1a93b2f0",
  "json_path": "$.data[0].currency"
}
```

*Server returns:*

```json
"usd"
```

**Claude's answer:**

> Across your 10 most recent payment intents, the total amount charged is **$54,848 USD** (amounts are in cents: 2000 + 5000 + 1500 + 8999 + 3500 + 12000 + 500 + 7250 + 4100 + 9999 = 54,848).

---

**Token cost comparison:**

| Approach                  | Context consumed     |
|---------------------------|----------------------|
| Direct Stripe API response| ~28,400 chars        |
| ContextCutter teaser      | ~289 chars           |
| ContextCutter queries     | ~120 chars (×2)      |
| **Total with CC**         | **~409 chars**       |
| **Savings**               | **~99%**             |

## Tips for Claude Desktop

**Authenticated APIs:** Pass headers directly in the `fetch_json_cutted` call. Headers are not persisted with the stored payload — they are used only for the outbound request.

**Pagination:** Each page is fetched and stored as a separate handle. If you need data across pages, Claude can fetch each page individually and query them in turn.

**Error responses:** If the remote API returns a non-2xx status, ContextCutter still attempts to parse and store the JSON body (useful for structured error responses like `{"error": {"message": "...", "code": "..."}}`).

**Session length:** Handles expire after 1 hour by default. For long Claude Desktop sessions working with the same dataset, refetch if you get an "unknown handle" error.

## Troubleshooting

**Tool not appearing in Claude:** Check that Claude Desktop restarted after you edited the config. Open Settings → Developer → MCP Servers to verify the connection status.

**`npx` is slow:** Install `context-cutter-mcp` globally (`npm install -g context-cutter-mcp`) or download the binary from [Releases](https://github.com/nikitaclicks/context-cutter/releases) for instant startup.

**Non-JSON responses:** `fetch_json_cutted` only handles JSON APIs. If the remote endpoint returns HTML or plain text, the tool will return an error.
