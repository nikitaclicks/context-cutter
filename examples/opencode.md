# ContextCutter + OpenCode

This example shows how ContextCutter integrates with [OpenCode](https://opencode.ai) to dramatically reduce the tokens your agent spends reading API responses.

## Configuration

Add ContextCutter to your OpenCode config at `~/.config/opencode/config.json`:

```json
{
  "mcp": {
    "context-cutter": {
      "type": "local",
      "command": "npx",
      "args": ["-y", "context-cutter-mcp"]
    }
  }
}
```

If you have the binary on `PATH` (see [Releases](https://github.com/nikitaclicks/context-cutter/releases)), use that instead to avoid the `npx` startup overhead:

```json
{
  "mcp": {
    "context-cutter": {
      "type": "local",
      "command": "context-cutter-mcp"
    }
  }
}
```

Restart OpenCode after editing the config. The two tools — `fetch_json_cutted` and `query_handle` — will appear automatically in the tool list.

## How the agent uses it

Once connected, the agent can reference ContextCutter tools in natural language. You do not need to instruct the model about how to use handles — the server's tool descriptions guide it automatically.

## Example session

**Prompt:**

```
Fetch the GitHub contributors for the torvalds/linux repo and tell me
who has the most contributions, and what their login is.
```

**What the agent does (without ContextCutter):**

The agent calls `curl` or a fetch tool, receives the full JSON response — potentially hundreds of contributor objects — and the entire payload lands in the context window. For a repo with 200+ contributors, that is ~40,000 characters of JSON.

**What the agent does (with ContextCutter):**

---

*Agent calls `fetch_json_cutted`:*

```json
{
  "url": "https://api.github.com/repos/torvalds/linux/contributors?per_page=100"
}
```

*Server responds (235 chars instead of ~40,000):*

```json
{
  "handle_id": "hdl_a3f9c2e18b04",
  "teaser": {
    "_teaser": true,
    "_type": "Array[100]",
    "item_keys": ["avatar_url", "contributions", "events_url", "followers_url",
                  "following_url", "gists_url", "gravatar_id", "html_url", "id",
                  "login", "node_id", "organizations_url", "received_events_url",
                  "repos_url", "site_admin", "starred_url", "subscriptions_url",
                  "type", "url"]
  }
}
```

*Agent reads the teaser, understands the array has 100 items with a `contributions` field and a `login` field. It calls `query_handle` to find the top contributor:*

```json
{
  "handle_id": "hdl_a3f9c2e18b04",
  "json_path": "$[0].login"
}
```

*Server responds:*

```json
"torvalds"
```

*Agent calls one more query to confirm the count:*

```json
{
  "handle_id": "hdl_a3f9c2e18b04",
  "json_path": "$[0].contributions"
}
```

*Server responds:*

```json
26351
```

**Agent final answer:**

> The top contributor to `torvalds/linux` is **torvalds** with **26,351 contributions**.

---

**Token cost comparison:**

| Approach              | Context consumed      |
|-----------------------|-----------------------|
| Fetch full JSON       | ~40,000 chars         |
| ContextCutter teaser  | ~235 chars            |
| ContextCutter queries | ~60 chars (×2)        |
| **Total with CC**     | **~355 chars**        |
| **Savings**           | **~99%**              |

## Multi-step example

ContextCutter is especially useful in multi-step agent tasks where the same large response would otherwise be re-read at each reasoning step.

**Prompt:**

```
Check the GitHub API for the top 3 contributors to torvalds/linux.
For each, tell me their login and contribution count.
Then summarize in one sentence.
```

The agent fetches once (`fetch_json_cutted`), gets a single handle, and issues three `query_handle` calls — one per contributor — rather than re-reading 40,000 characters of JSON three times.

```
fetch_json_cutted  → hdl_a3f9c2e18b04  (called once)
query_handle       → $[0].login        → "torvalds"
query_handle       → $[0].contributions → 26351
query_handle       → $[1].login        → "gregkh"
query_handle       → $[1].contributions → 19847
query_handle       → $[2].login        → "davem330"
query_handle       → $[2].contributions → 14203
```

Total context for all seven calls: **~700 chars**. Without ContextCutter, each reasoning step would re-read the full payload.

## Notes

- Handles expire after 1 hour by default (`CONTEXT_CUTTER_TTL_SECS`). Long-running sessions may need to refetch.
- The same URL fetched twice returns the same `handle_id` (deterministic SHA-256 of payload content).
- Only HTTPS URLs are accepted — HTTP is rejected to prevent SSRF.
