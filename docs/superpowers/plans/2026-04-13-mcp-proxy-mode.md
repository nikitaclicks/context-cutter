# MCP Proxy Mode Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add `--proxy <url>` mode to `context-cutter-mcp` so it acts as a transparent stdio↔HTTP proxy that intercepts large tool responses before they reach Claude's context.

**Architecture:** `main()` parses args and branches into normal mode (unchanged) or proxy mode. Proxy mode initialises `ProxyServer` — a struct that implements rmcp's `ServerHandler` directly (no macros — tools are dynamic). Tool calls are forwarded to the upstream HTTP MCP via `ureq`; responses at or above the threshold are stored and replaced with a handle + preview.

**Tech Stack:** Rust, rmcp 1.1.0, ureq 2.12.1 (pinned), tokio, clap 4 (new dep), serde_json.

---

## File Map

| File | Change |
|---|---|
| `Cargo.toml` | Add `clap = { version = "4", features = ["derive"] }` |
| `src/bin/mcp.rs` | Add `Args`, `parse_proxy_headers`, `upstream_call`, `ProxyServer`, `intercept_response`, `format_preview`, `run_proxy_mode`, refactor `main()` into `run_normal_mode` + dispatch |

All changes are in `src/bin/mcp.rs`. No new files needed.

---

## Task 1: Add clap and branch main()

**Files:**
- Modify: `Cargo.toml`
- Modify: `src/bin/mcp.rs`

- [ ] **Step 1: Add clap to Cargo.toml**

In `Cargo.toml`, add after the `ureq` line:
```toml
clap = { version = "4", features = ["derive"] }
```

- [ ] **Step 2: Add Args struct and imports to mcp.rs**

Add these imports at the top of `src/bin/mcp.rs` (after existing `use` lines):
```rust
use clap::Parser;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
```

Add `Args` struct after the existing `validate_query_inputs` function (around line 79):
```rust
// ─── CLI args ─────────────────────────────────────────────────────────────────

#[derive(Parser, Debug)]
#[command(name = "context-cutter-mcp", about = "ContextCutter MCP server")]
struct Args {
    /// Run as a transparent proxy in front of an HTTP MCP server.
    /// Replaces the upstream MCP entry in your MCP config.
    #[arg(long)]
    proxy: Option<String>,

    /// Response size threshold in bytes. Responses at or above this size are
    /// intercepted and replaced with a handle + preview. Default: 2048.
    #[arg(long, default_value_t = 2048)]
    proxy_threshold: usize,

    /// Extra HTTP header to forward to the upstream MCP server.
    /// Format: "Key: Value". Repeat for multiple headers.
    /// Example: --proxy-header "Authorization: Bearer $TOKEN"
    #[arg(long)]
    proxy_header: Vec<String>,
}
```

- [ ] **Step 3: Refactor main() into run_normal_mode() and dispatch**

Replace the existing `main()` function with:
```rust
async fn run_normal_mode() {
    start_background_sweeper();
    let server = match ContextCutterServer::new().serve(stdio()).await {
        Ok(s) => s,
        Err(e) => {
            error!(error = %e, "startup error");
            eprintln!("context-cutter-mcp: startup error: {e}");
            std::process::exit(1);
        }
    };
    if let Err(e) = server.waiting().await {
        error!(error = %e, "server runtime error");
        eprintln!("context-cutter-mcp: server error: {e}");
        std::process::exit(1);
    }
}

#[tokio::main]
async fn main() {
    init_tracing();
    let args = Args::parse();

    if let Some(ref upstream_url) = args.proxy {
        // proxy mode — implemented in subsequent tasks
        eprintln!("context-cutter-mcp: proxy mode not yet implemented");
        std::process::exit(1);
    } else {
        run_normal_mode().await;
    }
}
```

- [ ] **Step 4: Verify it compiles in normal mode**

```bash
cd ~/dev/context-cutter && cargo build --bin context-cutter-mcp 2>&1
```
Expected: compiles with no errors. `--proxy` flag present but returns early with an error message.

- [ ] **Step 5: Commit**

```bash
cd ~/dev/context-cutter
git add Cargo.toml Cargo.lock src/bin/mcp.rs
git commit -m "feat(proxy): add --proxy CLI flag and refactor main() into run_normal_mode()"
```

---

## Task 2: Header parsing and upstream_call()

**Files:**
- Modify: `src/bin/mcp.rs`

- [ ] **Step 1: Write unit tests for parse_proxy_headers**

Add this test module at the end of `src/bin/mcp.rs`:
```rust
#[cfg(test)]
mod proxy_tests {
    use super::*;

    #[test]
    fn parse_proxy_headers_valid() {
        let raw = vec![
            "Authorization: Bearer abc123".to_string(),
            "X-Custom: value".to_string(),
        ];
        let result = parse_proxy_headers(&raw).unwrap();
        assert_eq!(result, vec![
            ("Authorization".to_string(), "Bearer abc123".to_string()),
            ("X-Custom".to_string(), "value".to_string()),
        ]);
    }

    #[test]
    fn parse_proxy_headers_malformed_returns_err() {
        let raw = vec!["no-colon-space".to_string()];
        assert!(parse_proxy_headers(&raw).is_err());
    }

    #[test]
    fn parse_proxy_headers_empty() {
        assert_eq!(parse_proxy_headers(&[]).unwrap(), vec![]);
    }
}
```

- [ ] **Step 2: Run tests to confirm they fail**

```bash
cd ~/dev/context-cutter && cargo test parse_proxy_headers 2>&1
```
Expected: FAIL — `parse_proxy_headers` not defined yet.

- [ ] **Step 3: Implement parse_proxy_headers and upstream_call**

Add these two functions to `src/bin/mcp.rs` after the `boundary_error` function (around line 123):

```rust
// ─── Proxy helpers ────────────────────────────────────────────────────────────

/// Parse `--proxy-header` strings of the form `"Key: Value"` into pairs.
fn parse_proxy_headers(raw: &[String]) -> Result<Vec<(String, String)>, String> {
    raw.iter()
        .map(|h| {
            h.split_once(": ")
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .ok_or_else(|| {
                    format!("invalid --proxy-header (expected 'Key: Value'): {h}")
                })
        })
        .collect()
}

/// POST a JSON-RPC 2.0 request to `url` and return the `result` field.
///
/// Must be called inside `tokio::task::spawn_blocking` — `ureq` is synchronous.
fn upstream_call(
    url: &str,
    method: &str,
    params: serde_json::Value,
    headers: &[(String, String)],
    id: u64,
) -> Result<serde_json::Value, ContextCutterError> {
    let body_str = serde_json::to_string(&serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": method,
        "params": params,
    }))
    .map_err(|e| ContextCutterError::Serialize(e.to_string()))?;

    let agent = ureq::AgentBuilder::new()
        .timeout(std::time::Duration::from_secs(30))
        .build();

    let mut req = agent
        .post(url)
        .set("Content-Type", "application/json")
        .set("Accept", "application/json, text/event-stream");
    for (k, v) in headers {
        req = req.set(k, v);
    }

    let response = req
        .send_string(&body_str)
        .map_err(|e| ContextCutterError::RequestFailed(e.to_string()))?;

    let response_str = read_response_with_limit(response, max_payload_bytes())?;
    let json: serde_json::Value = serde_json::from_str(&response_str)
        .map_err(|e| ContextCutterError::InvalidJson(format!("upstream response: {e}")))?;

    if let Some(err) = json.get("error") {
        return Err(ContextCutterError::RequestFailed(format!(
            "upstream MCP error: {err}"
        )));
    }

    json.get("result").cloned().ok_or_else(|| {
        ContextCutterError::RequestFailed(
            "upstream response missing 'result' field".to_string(),
        )
    })
}
```

- [ ] **Step 4: Run tests**

```bash
cd ~/dev/context-cutter && cargo test parse_proxy_headers 2>&1
```
Expected: all 3 tests PASS.

- [ ] **Step 5: Commit**

```bash
cd ~/dev/context-cutter
git add src/bin/mcp.rs
git commit -m "feat(proxy): add parse_proxy_headers and upstream_call helpers"
```

---

## Task 3: ProxyServer struct and upstream initialization

**Files:**
- Modify: `src/bin/mcp.rs`

- [ ] **Step 1: Write test for ProxyServer construction**

Add to `proxy_tests` module:
```rust
    #[test]
    fn proxy_server_exposes_query_handle_in_tool_list() {
        let server = ProxyServer::new(
            "https://example.com/mcp".to_string(),
            vec![],
            2048,
            vec![],
        );
        // query_handle should always be in the combined tool list
        let all_tools = server.all_tools();
        assert!(all_tools.iter().any(|t| t.name == "query_handle"));
    }
```

- [ ] **Step 2: Run test to confirm it fails**

```bash
cd ~/dev/context-cutter && cargo test proxy_server_exposes_query_handle 2>&1
```
Expected: FAIL — `ProxyServer` not defined.

- [ ] **Step 3: Add rmcp model imports**

Add to the existing `use rmcp` block at the top of `src/bin/mcp.rs`:
```rust
use rmcp::{
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{
        CallToolRequestParams, CallToolResult, Content, ListToolsResult,
        PaginatedRequestParams, ServerCapabilities, ServerInfo, Tool,
    },
    schemars, tool, tool_handler, tool_router,
    transport::stdio,
    Error as McpError, ServerHandler, ServiceExt,
};
```
Also add:
```rust
use rmcp::model::RawContent;
use rmcp::model::annotated::Annotated;
use rmcp::service::RequestContext;
use rmcp::handler::server::RoleServer;
```

- [ ] **Step 4: Add ProxyServer struct after the QueryParams struct (around line 147)**

```rust
// ─── Proxy server ─────────────────────────────────────────────────────────────

#[derive(Clone)]
struct ProxyServer {
    upstream_url: String,
    upstream_headers: Arc<Vec<(String, String)>>,
    threshold: usize,
    upstream_tools: Arc<Vec<Tool>>,
    next_id: Arc<AtomicU64>,
}

impl ProxyServer {
    fn new(
        upstream_url: String,
        upstream_headers: Vec<(String, String)>,
        threshold: usize,
        upstream_tools: Vec<Tool>,
    ) -> Self {
        Self {
            upstream_url,
            upstream_headers: Arc::new(upstream_headers),
            threshold,
            upstream_tools: Arc::new(upstream_tools),
            next_id: Arc::new(AtomicU64::new(3)), // 1 and 2 used at init
        }
    }

    fn next_id(&self) -> u64 {
        self.next_id.fetch_add(1, Ordering::Relaxed)
    }

    /// Returns upstream tools combined with the local query_handle tool.
    fn all_tools(&self) -> Vec<Tool> {
        let mut tools = (*self.upstream_tools).clone();
        tools.push(query_handle_tool_def());
        tools
    }
}

/// Build the query_handle Tool definition for advertising to Claude.
fn query_handle_tool_def() -> Tool {
    let schema: serde_json::Map<String, serde_json::Value> = serde_json::from_str(
        r#"{
            "type": "object",
            "properties": {
                "handle_id": {
                    "type": "string",
                    "description": "Handle ID returned by a proxied tool call."
                },
                "json_path": {
                    "type": "string",
                    "description": "JSONPath expression, e.g. $.user.name or user.name."
                }
            },
            "required": ["handle_id", "json_path"]
        }"#,
    )
    .expect("query_handle schema is valid JSON");

    Tool::new(
        "query_handle",
        "Extract a value from a previously stored JSON payload using JSONPath. \
         Accepts full JSONPath ($.foo.bar) or dot notation (foo.bar). \
         Returns the matched value as JSON or null.",
        Arc::new(schema),
    )
}
```

- [ ] **Step 5: Run test**

```bash
cd ~/dev/context-cutter && cargo test proxy_server_exposes_query_handle 2>&1
```
Expected: PASS.

- [ ] **Step 6: Add init_proxy_server async function**

Add after `ProxyServer` impl:
```rust
/// Connect to the upstream MCP, run the handshake, and return an initialised ProxyServer.
async fn init_proxy_server(
    upstream_url: &str,
    headers: &[(String, String)],
    threshold: usize,
) -> Result<ProxyServer, ContextCutterError> {
    let url = upstream_url.to_string();
    let hdrs = headers.to_vec();

    // Step 1: initialize
    let _init_result = tokio::task::spawn_blocking({
        let url = url.clone();
        let hdrs = hdrs.clone();
        move || {
            upstream_call(
                &url,
                "initialize",
                serde_json::json!({
                    "protocolVersion": "2024-11-05",
                    "capabilities": {},
                    "clientInfo": {
                        "name": "context-cutter-proxy",
                        "version": env!("CARGO_PKG_VERSION")
                    }
                }),
                &hdrs,
                1,
            )
        }
    })
    .await
    .map_err(|e| ContextCutterError::Internal(format!("spawn_blocking: {e}")))?
    .map_err(|e| ContextCutterError::RequestFailed(format!("initialize failed: {e}")))?;

    info!("upstream MCP initialized");

    // Step 2: tools/list
    let tools_result = tokio::task::spawn_blocking({
        let url = url.clone();
        let hdrs = hdrs.clone();
        move || upstream_call(&url, "tools/list", serde_json::json!({}), &hdrs, 2)
    })
    .await
    .map_err(|e| ContextCutterError::Internal(format!("spawn_blocking: {e}")))?
    .map_err(|e| ContextCutterError::RequestFailed(format!("tools/list failed: {e}")))?;

    let upstream_tools: Vec<Tool> = tools_result
        .get("tools")
        .and_then(|t| serde_json::from_value(t.clone()).ok())
        .unwrap_or_default();

    info!(tool_count = upstream_tools.len(), "upstream tools discovered");

    Ok(ProxyServer::new(url, hdrs, threshold, upstream_tools))
}
```

- [ ] **Step 7: Compile check**

```bash
cd ~/dev/context-cutter && cargo build --bin context-cutter-mcp 2>&1
```
Expected: compiles cleanly.

- [ ] **Step 8: Commit**

```bash
cd ~/dev/context-cutter
git add src/bin/mcp.rs
git commit -m "feat(proxy): add ProxyServer struct and init_proxy_server upstream handshake"
```

---

## Task 4: ProxyServer ServerHandler implementation

**Files:**
- Modify: `src/bin/mcp.rs`

- [ ] **Step 1: Implement ServerHandler for ProxyServer**

Add after `init_proxy_server`:

```rust
impl ServerHandler for ProxyServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_instructions(
                "Transparent MCP proxy with response interception. \
                 Large tool responses are stored as handles. \
                 Use query_handle(handle_id, \"$.field\") to extract specific fields.",
            )
    }

    fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> impl std::future::Future<Output = Result<ListToolsResult, McpError>> + Send + '_ {
        std::future::ready(Ok(ListToolsResult::with_all_items(self.all_tools())))
    }

    fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> impl std::future::Future<Output = Result<CallToolResult, McpError>> + Send + '_ {
        async move {
            let tool_name = request.name.as_ref().to_string();

            // ── Local: query_handle ────────────────────────────────────────────
            if tool_name == "query_handle" {
                let args = request.arguments.as_ref().ok_or_else(|| {
                    McpError::invalid_params("query_handle requires arguments", None)
                })?;
                let handle_id = args
                    .get("handle_id")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| McpError::invalid_params("missing handle_id", None))?;
                let json_path = args
                    .get("json_path")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| McpError::invalid_params("missing json_path", None))?;

                validate_query_inputs(handle_id, json_path)
                    .map_err(|e| McpError::invalid_params(e.to_string(), None))?;

                let result = engine_query(handle_id, json_path)
                    .map_err(|e| McpError::internal_error(e.to_string(), None))?;

                return Ok(CallToolResult::success(vec![text_content(result)]));
            }

            // ── Forward to upstream ────────────────────────────────────────────
            let url = self.upstream_url.clone();
            let headers = (*self.upstream_headers).clone();
            let id = self.next_id();
            let threshold = self.threshold;
            let arguments = request
                .arguments
                .map(serde_json::Value::Object)
                .unwrap_or_else(|| serde_json::Value::Object(Default::default()));

            let upstream_result = tokio::task::spawn_blocking(move || {
                upstream_call(
                    &url,
                    "tools/call",
                    serde_json::json!({ "name": tool_name, "arguments": arguments }),
                    &headers,
                    id,
                )
            })
            .await
            .map_err(|e| McpError::internal_error(format!("spawn_blocking: {e}"), None))?
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;

            // ── Intercept or pass through ──────────────────────────────────────
            match intercept_if_large(upstream_result, threshold) {
                Ok(result) => Ok(result),
                Err(e) => Err(McpError::internal_error(e.to_string(), None)),
            }
        }
    }
}

/// Convenience: wrap a string as a text Content item.
fn text_content(text: String) -> Content {
    Annotated::new(RawContent::text(text), None)
}
```

- [ ] **Step 2: Compile check**

```bash
cd ~/dev/context-cutter && cargo build --bin context-cutter-mcp 2>&1
```
Expected: compiles (with warning about `intercept_if_large` not defined yet — that's fine at this step).

- [ ] **Step 3: Commit**

```bash
cd ~/dev/context-cutter
git add src/bin/mcp.rs
git commit -m "feat(proxy): implement ServerHandler for ProxyServer"
```

---

## Task 5: Interception logic and preview formatting

**Files:**
- Modify: `src/bin/mcp.rs`

- [ ] **Step 1: Write unit tests for intercept_if_large and format_preview**

Add to `proxy_tests`:
```rust
    #[test]
    fn intercept_if_large_passes_through_small_result() {
        let result = serde_json::json!({ "content": [{"type":"text","text":"hi"}], "isError": false });
        let call_result = intercept_if_large(result, 2048).unwrap();
        // Small result — content passed through
        assert_eq!(call_result.content.len(), 1);
        let text = match &call_result.content[0].raw {
            RawContent::Text(t) => t.text.clone(),
            _ => panic!("expected text"),
        };
        assert!(!text.contains("[context-cutter]"));
    }

    #[test]
    fn intercept_if_large_replaces_big_result_with_handle() {
        let big_json = serde_json::json!({
            "content": [{
                "type": "text",
                "text": serde_json::to_string(&serde_json::json!({
                    "id": "abc",
                    "name": "Big Payload",
                    "items": (0..100).collect::<Vec<_>>()
                })).unwrap()
            }],
            "isError": false
        });
        let call_result = intercept_if_large(big_json, 10).unwrap(); // 10-byte threshold
        let text = match &call_result.content[0].raw {
            RawContent::Text(t) => t.text.clone(),
            _ => panic!("expected text"),
        };
        assert!(text.contains("[context-cutter]"));
        assert!(text.contains("hdl_"));
        assert!(text.contains("query_handle"));
    }

    #[test]
    fn format_preview_truncates_long_strings() {
        let long_str = "a".repeat(200);
        let result = format_preview_value(&serde_json::Value::String(long_str));
        assert!(result.len() <= 100); // truncated
        assert!(result.contains("(truncated)"));
    }

    #[test]
    fn format_preview_shows_array_length() {
        let arr = serde_json::json!([1, 2, 3]);
        assert_eq!(format_preview_value(&arr), "[3 items]");
    }

    #[test]
    fn format_preview_shows_object_key_count() {
        let obj = serde_json::json!({"a": 1, "b": 2});
        assert_eq!(format_preview_value(&obj), "{2 keys}");
    }
```

- [ ] **Step 2: Run tests to confirm they fail**

```bash
cd ~/dev/context-cutter && cargo test intercept_if_large 2>&1
cd ~/dev/context-cutter && cargo test format_preview 2>&1
```
Expected: FAIL — functions not defined.

- [ ] **Step 3: Implement intercept_if_large, format_preview_value, and build_preview_text**

Add these functions after `text_content`:

```rust
// ─── Interception ─────────────────────────────────────────────────────────────

/// If the upstream result is at or above `threshold` bytes, store it and return
/// a handle + preview. Otherwise pass the result through as-is.
fn intercept_if_large(
    upstream_result: serde_json::Value,
    threshold: usize,
) -> Result<CallToolResult, ContextCutterError> {
    // Serialise once to check size.
    let result_str = serde_json::to_string(&upstream_result)
        .map_err(|e| ContextCutterError::Serialize(e.to_string()))?;

    if result_str.len() < threshold {
        // Small — pass through. Deserialise back to CallToolResult.
        return serde_json::from_value(upstream_result).map_err(|_| {
            // Fallback: wrap raw string if structure is unexpected.
            ContextCutterError::Internal("unexpected upstream result shape".to_string())
        });
    }

    // Large — extract the inner JSON text if available; otherwise store the envelope.
    let payload_str = upstream_result
        .get("content")
        .and_then(|c| c.get(0))
        .and_then(|c| c.get("text"))
        .and_then(|t| t.as_str())
        .filter(|t| t.starts_with('{') || t.starts_with('['))
        .unwrap_or(&result_str);

    let handle_id = engine_store(payload_str)?;
    let teaser_str = engine_teaser(&handle_id)?;
    let preview = build_preview_text(&handle_id, payload_str.len(), &teaser_str)?;

    Ok(CallToolResult::success(vec![text_content(preview)]))
}

/// Render the preview text shown to Claude when a response is intercepted.
fn build_preview_text(
    handle_id: &str,
    original_bytes: usize,
    teaser_str: &str,
) -> Result<String, ContextCutterError> {
    let teaser: serde_json::Value = serde_json::from_str(teaser_str)
        .map_err(|e| ContextCutterError::InvalidJson(format!("teaser: {e}")))?;

    let kb = (original_bytes as f64) / 1024.0;
    let mut lines = vec![
        format!("[context-cutter] Response stored ({kb:.1} KB → handle: {handle_id})"),
        String::new(),
        "Preview:".to_string(),
    ];

    if let Some(structure) = teaser.get("structure").and_then(|s| s.as_object()) {
        for (key, val) in structure.iter().take(20) {
            lines.push(format!("  {key}: {}", format_preview_value(val)));
        }
    }

    lines.push(String::new());
    lines.push(format!(
        r#"Call query_handle("{handle_id}", "$.field") to extract specific fields."#
    ));

    Ok(lines.join("\n"))
}

/// Render a single teaser value for the preview block.
pub fn format_preview_value(val: &serde_json::Value) -> String {
    match val {
        serde_json::Value::String(s) => {
            if s.len() > 80 {
                format!("\"{}...\" (truncated)", &s[..80])
            } else {
                format!("\"{s}\"")
            }
        }
        serde_json::Value::Array(arr) => format!("[{} items]", arr.len()),
        serde_json::Value::Object(obj) => {
            // rmcp teaser objects use _type: "Array[N]" — show that directly.
            if let Some(t) = obj.get("_type").and_then(|v| v.as_str()) {
                if t.starts_with("Array[") {
                    return t.to_string();
                }
            }
            format!("{{{} keys}}", obj.len())
        }
        other => other.to_string(),
    }
}
```

- [ ] **Step 4: Run tests**

```bash
cd ~/dev/context-cutter && cargo test intercept_if_large 2>&1
cd ~/dev/context-cutter && cargo test format_preview 2>&1
```
Expected: all tests PASS.

- [ ] **Step 5: Commit**

```bash
cd ~/dev/context-cutter
git add src/bin/mcp.rs
git commit -m "feat(proxy): add interception logic and preview formatting"
```

---

## Task 6: Wire run_proxy_mode and full compile

**Files:**
- Modify: `src/bin/mcp.rs`

- [ ] **Step 1: Implement run_proxy_mode**

Replace the placeholder `run_proxy_mode` body in `main()` with a real function. Add this function before `main()`:

```rust
async fn run_proxy_mode(
    upstream_url: &str,
    threshold: usize,
    raw_headers: &[String],
) {
    let headers = match parse_proxy_headers(raw_headers) {
        Ok(h) => h,
        Err(e) => {
            eprintln!("context-cutter-mcp: {e}");
            std::process::exit(1);
        }
    };

    if !upstream_url.starts_with("https://") {
        eprintln!("context-cutter-mcp: --proxy URL must start with https://");
        std::process::exit(1);
    }

    info!(upstream_url, threshold, "starting proxy mode");

    start_background_sweeper();

    let proxy_server = match init_proxy_server(upstream_url, &headers, threshold).await {
        Ok(s) => s,
        Err(e) => {
            error!(error = %e, "failed to initialise upstream MCP");
            eprintln!("context-cutter-mcp: upstream init failed: {e}");
            std::process::exit(1);
        }
    };

    let server = match proxy_server.serve(stdio()).await {
        Ok(s) => s,
        Err(e) => {
            error!(error = %e, "startup error");
            eprintln!("context-cutter-mcp: startup error: {e}");
            std::process::exit(1);
        }
    };
    if let Err(e) = server.waiting().await {
        error!(error = %e, "proxy runtime error");
        eprintln!("context-cutter-mcp: proxy error: {e}");
        std::process::exit(1);
    }
}
```

Update `main()` to call it:
```rust
#[tokio::main]
async fn main() {
    init_tracing();
    let args = Args::parse();

    if let Some(ref upstream_url) = args.proxy {
        run_proxy_mode(upstream_url, args.proxy_threshold, &args.proxy_header).await;
    } else {
        run_normal_mode().await;
    }
}
```

- [ ] **Step 2: Full compile + all existing tests pass**

```bash
cd ~/dev/context-cutter && cargo build --bin context-cutter-mcp 2>&1
cd ~/dev/context-cutter && cargo test 2>&1
```
Expected: binary builds cleanly, all tests PASS (no regressions).

- [ ] **Step 3: Manual smoke test — normal mode unchanged**

```bash
cd ~/dev/context-cutter
echo '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"test","version":"0"}}}' \
  | cargo run --bin context-cutter-mcp 2>/dev/null
```
Expected: JSON response with `serverInfo` and `capabilities.tools`.

- [ ] **Step 4: Manual smoke test — proxy flag help**

```bash
cd ~/dev/context-cutter && cargo run --bin context-cutter-mcp -- --help 2>&1
```
Expected: help text showing `--proxy`, `--proxy-threshold`, `--proxy-header` flags.

- [ ] **Step 5: Commit**

```bash
cd ~/dev/context-cutter
git add src/bin/mcp.rs
git commit -m "feat(proxy): wire run_proxy_mode — proxy mode fully implemented"
```

---

## Task 7: Python integration test

**Files:**
- Create: `tests/test_proxy.py`

- [ ] **Step 1: Write the integration test**

Create `tests/test_proxy.py`:

```python
"""Integration test for context-cutter-mcp --proxy mode.

Spins up a minimal in-process HTTP MCP server, builds the binary, and
verifies that large tool responses are intercepted with a handle + preview.
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
LARGE_PAYLOAD = {"items": [{"id": i, "name": f"item-{i}", "data": "x" * 50} for i in range(50)]}

TOOLS = [
    {
        "name": "get_items",
        "description": "Returns a large list of items.",
        "inputSchema": {"type": "object", "properties": {}, "required": []},
    }
]


class MockMcpHandler(BaseHTTPRequestHandler):
    """Minimal HTTP MCP server that handles initialize, tools/list, tools/call."""

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

        response = json.dumps({"jsonrpc": "2.0", "id": req_id, "result": result}).encode()
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


def _send_mcp(proc: subprocess.Popen, message: dict) -> dict:
    line = json.dumps(message) + "\n"
    proc.stdin.write(line.encode())
    proc.stdin.flush()
    return json.loads(proc.stdout.readline())


@pytest.mark.integration
def test_proxy_intercepts_large_response():
    if not BINARY.exists():
        pytest.skip("binary not built — run `cargo build --bin context-cutter-mcp` first")

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
            # MCP handshake
            init_resp = _send_mcp(proc, {
                "jsonrpc": "2.0", "id": 1, "method": "initialize",
                "params": {"protocolVersion": "2024-11-05", "capabilities": {},
                           "clientInfo": {"name": "test", "version": "0"}},
            })
            assert "result" in init_resp

            # List tools — should include get_items + query_handle
            tools_resp = _send_mcp(proc, {
                "jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {},
            })
            tool_names = [t["name"] for t in tools_resp["result"]["tools"]]
            assert "get_items" in tool_names
            assert "query_handle" in tool_names

            # Call the large tool — should be intercepted
            call_resp = _send_mcp(proc, {
                "jsonrpc": "2.0", "id": 3, "method": "tools/call",
                "params": {"name": "get_items", "arguments": {}},
            })
            text = call_resp["result"]["content"][0]["text"]
            assert "[context-cutter]" in text
            assert "hdl_" in text
            assert "query_handle" in text

            # Extract handle_id and call query_handle
            handle_id = next(
                word for word in text.split() if word.startswith("hdl_")
            ).rstrip(")")
            query_resp = _send_mcp(proc, {
                "jsonrpc": "2.0", "id": 4, "method": "tools/call",
                "params": {"name": "query_handle",
                           "arguments": {"handle_id": handle_id, "json_path": "$.items[0].id"}},
            })
            assert query_resp["result"]["content"][0]["text"] == "0"

        finally:
            proc.terminate()
            proc.wait(timeout=5)

    finally:
        server.shutdown()


@pytest.mark.integration
def test_proxy_passes_through_small_response():
    if not BINARY.exists():
        pytest.skip("binary not built — run `cargo build --bin context-cutter-mcp` first")

    # Use a very high threshold so nothing gets intercepted.
    server, port = _start_mock_server()
    upstream_url = f"http://127.0.0.1:{port}/mcp"

    try:
        proc = subprocess.Popen(
            [str(BINARY), "--proxy", upstream_url, "--proxy-threshold", "999999"],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
        )

        try:
            _send_mcp(proc, {
                "jsonrpc": "2.0", "id": 1, "method": "initialize",
                "params": {"protocolVersion": "2024-11-05", "capabilities": {},
                           "clientInfo": {"name": "test", "version": "0"}},
            })
            call_resp = _send_mcp(proc, {
                "jsonrpc": "2.0", "id": 2, "method": "tools/call",
                "params": {"name": "get_items", "arguments": {}},
            })
            text = call_resp["result"]["content"][0]["text"]
            assert "[context-cutter]" not in text  # passed through

        finally:
            proc.terminate()
            proc.wait(timeout=5)

    finally:
        server.shutdown()
```

- [ ] **Step 2: Add `integration` marker to pytest config**

In `pyproject.toml`, update the markers list:
```toml
[tool.pytest.ini_options]
testpaths = ["tests"]
addopts = "-q --strict-markers"
markers = [
  "benchmark: performance benchmark tests",
  "integration: tests that require the compiled binary",
]
```

- [ ] **Step 3: Build binary and run integration tests**

```bash
cd ~/dev/context-cutter
cargo build --bin context-cutter-mcp 2>&1
python -m pytest tests/test_proxy.py -v -m integration 2>&1
```
Expected: both `test_proxy_intercepts_large_response` and `test_proxy_passes_through_small_response` PASS.

Note: if the upstream HTTP MCP server uses `http://` during testing but the binary validates `https://` only, temporarily relax the URL check in `run_proxy_mode` for testing or use a real `https` endpoint. The test uses `http://127.0.0.1` — adjust validation in `run_proxy_mode` to allow `http://` for localhost, or skip the protocol check for tests.

- [ ] **Step 4: Run full test suite**

```bash
cd ~/dev/context-cutter && cargo test 2>&1 && python -m pytest -q 2>&1
```
Expected: all Rust tests PASS, all Python tests PASS (integration tests pass with built binary).

- [ ] **Step 5: Commit**

```bash
cd ~/dev/context-cutter
git add tests/test_proxy.py pyproject.toml
git commit -m "test(proxy): add integration tests for --proxy mode interception"
```

---

## Task 8: Open draft PR

- [ ] **Step 1: Push branch to origin**

```bash
cd ~/dev/context-cutter
git push -u origin HEAD 2>&1
```

- [ ] **Step 2: Open draft PR**

```bash
cd ~/dev/context-cutter
gh pr create --draft \
  --title "feat: add --proxy mode for transparent HTTP MCP interception" \
  --body "$(cat <<'EOF'
## Summary

- Adds `--proxy <url>` flag to `context-cutter-mcp`
- When active, the binary acts as a stdio↔HTTP proxy in front of any HTTP MCP server
- Tool responses at or above `--proxy-threshold` bytes (default 2048) are stored as handles and replaced with a handle + preview teaser
- `query_handle` is injected as an additional tool alongside the proxied tools
- Normal mode (no flags) is unchanged

## Usage

Replace an HTTP MCP entry in your MCP config:
```json
{
  "clickup-staging": {
    "command": "npx",
    "args": ["-y", "context-cutter-mcp", "--proxy", "https://mcp.clickup-stg.com/mcp"]
  }
}
```

## Test plan

- [ ] `cargo test` — all Rust unit tests pass
- [ ] `pytest` — all Python tests pass including new integration tests
- [ ] Manual: normal mode (`context-cutter-mcp` with no flags) unchanged
- [ ] Manual: `--help` shows `--proxy`, `--proxy-threshold`, `--proxy-header`
- [ ] Manual: proxy mode against a real HTTP MCP endpoint

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF
)"
```

- [ ] **Step 3: Copy PR URL and share**

```bash
gh pr view --json url -q .url 2>&1
```

---

## Self-Review Notes

- **Spec coverage:** All sections covered — CLI args (Task 1), upstream HTTP calls (Task 2), ProxyServer struct (Task 3), ServerHandler (Task 4), interception + preview (Task 5), wire-up (Task 6), integration tests (Task 7), draft PR (Task 8).
- **`http://` vs `https://` in tests:** Task 7 uses `http://127.0.0.1` for the mock server. The `run_proxy_mode` URL check (Task 6) enforces `https://`. Either relax the check to `http://localhost`/`http://127.0.0.1`, or test with a self-signed cert. Simplest fix: skip the protocol check only in `#[cfg(test)]`, or make the check a warning instead of a hard exit during integration testing.
- **Type names to verify before building:** `PaginatedRequestParams`, `Annotated::new`, `RawContent` — confirm imports compile; adjust if rmcp exports these under different paths.
- **`ListToolsResult::with_all_items`** — verified from rmcp source (macro-generated).
- **`CallToolResult::success`** — verified from rmcp source.
