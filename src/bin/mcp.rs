//! `context-cutter-mcp` — Rust MCP stdio server.
//!
//! Exposes two tools over the Model Context Protocol:
//!
//! - `fetch_json_cutted`: fetch a JSON endpoint, store it, return `{handle_id, teaser}`.
//! - `query_handle`: extract a specific value from a stored payload via JSONPath.

use context_cutter::engine::{engine_query, engine_store, engine_teaser};
use context_cutter::error::ContextCutterError;
use context_cutter::store::start_background_sweeper;
use rmcp::{
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{
        Annotated, CallToolRequestParams, CallToolResult, ListToolsResult,
        PaginatedRequestParams, RawContent, ServerCapabilities, ServerInfo, Tool,
    },
    schemars, tool, tool_handler, tool_router,
    transport::stdio,
    ErrorData as McpError, ServerHandler, ServiceExt,
};
use rmcp::service::RequestContext;
use rmcp::service::RoleServer;
use clap::Parser;
use serde::Deserialize;
use std::collections::HashMap;
use std::io::Read;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use tracing::{error, info, instrument};
use tracing_subscriber::EnvFilter;

const DEFAULT_MAX_PAYLOAD_BYTES: usize = 10 * 1024 * 1024;
const MAX_JSON_PATH_LEN: usize = 4096;

fn max_payload_bytes() -> usize {
    std::env::var("CONTEXT_CUTTER_MAX_PAYLOAD_BYTES")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .filter(|v| *v > 0)
        .unwrap_or(DEFAULT_MAX_PAYLOAD_BYTES)
}

fn validate_https_url(url: &str) -> Result<(), ContextCutterError> {
    if url.as_bytes().contains(&0) {
        return Err(ContextCutterError::Validation(
            "url must not contain null bytes".to_string(),
        ));
    }
    if !url.starts_with("https://") {
        return Err(ContextCutterError::Validation(
            "only https URLs are allowed".to_string(),
        ));
    }
    Ok(())
}

fn validate_query_inputs(handle_id: &str, json_path: &str) -> Result<(), ContextCutterError> {
    if handle_id.trim().is_empty() {
        return Err(ContextCutterError::Validation(
            "handle_id must not be empty".to_string(),
        ));
    }
    if handle_id.as_bytes().contains(&0) {
        return Err(ContextCutterError::Validation(
            "handle_id must not contain null bytes".to_string(),
        ));
    }
    if json_path.trim().is_empty() {
        return Err(ContextCutterError::Validation(
            "json path must not be empty".to_string(),
        ));
    }
    if json_path.len() > MAX_JSON_PATH_LEN {
        return Err(ContextCutterError::Validation(format!(
            "json path too long: {} bytes (max {})",
            json_path.len(),
            MAX_JSON_PATH_LEN
        )));
    }
    if json_path.as_bytes().contains(&0) {
        return Err(ContextCutterError::Validation(
            "json path must not contain null bytes".to_string(),
        ));
    }
    Ok(())
}

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

fn read_response_with_limit(
    response: ureq::Response,
    max_bytes: usize,
) -> Result<String, ContextCutterError> {
    let mut reader = response.into_reader();
    let mut limited = reader.by_ref().take((max_bytes + 1) as u64);
    let mut buf = Vec::with_capacity(max_bytes.min(64 * 1024));
    limited
        .read_to_end(&mut buf)
        .map_err(|e| ContextCutterError::RequestFailed(format!("failed to read body: {e}")))?;

    if buf.len() > max_bytes {
        return Err(ContextCutterError::PayloadTooLarge {
            actual_bytes: buf.len(),
            max_bytes,
        });
    }

    String::from_utf8(buf)
        .map_err(|e| ContextCutterError::RequestFailed(format!("non-utf8 body: {e}")))
}

fn init_tracing() {
    let format = std::env::var("CONTEXT_CUTTER_LOG_FORMAT").unwrap_or_else(|_| "plain".to_string());
    let env_filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

    if format == "json" {
        tracing_subscriber::fmt()
            .json()
            .with_env_filter(env_filter)
            .with_current_span(false)
            .with_target(false)
            .init();
    } else {
        tracing_subscriber::fmt()
            .with_env_filter(env_filter)
            .with_target(false)
            .init();
    }
}

fn boundary_error(err: ContextCutterError) -> String {
    err.to_string()
}

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

// ─── Tool parameter types ─────────────────────────────────────────────────────

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct FetchParams {
    /// URL to fetch JSON from.
    url: String,
    /// HTTP method (default: "GET").
    method: Option<String>,
    /// Optional HTTP headers as key-value string pairs.
    headers: Option<HashMap<String, String>>,
    /// Optional request body sent as JSON.
    body: Option<serde_json::Value>,
    /// Timeout in seconds (default: 45).
    timeout_seconds: Option<f64>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct QueryParams {
    /// Handle ID returned by `fetch_json_cutted`.
    handle_id: String,
    /// JSONPath expression, e.g. `$.user.name` or `user.name`.
    json_path: String,
}

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

// ─── ServerHandler for ProxyServer ───────────────────────────────────────────

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
fn text_content(text: String) -> Annotated<RawContent> {
    Annotated::new(RawContent::text(text), None)
}

// ─── Interception ─────────────────────────────────────────────────────────────

/// If the upstream result is at or above `threshold` bytes, store it and return
/// a handle + preview. Otherwise pass the result through as-is.
fn intercept_if_large(
    upstream_result: serde_json::Value,
    threshold: usize,
) -> Result<CallToolResult, ContextCutterError> {
    let result_str = serde_json::to_string(&upstream_result)
        .map_err(|e| ContextCutterError::Serialize(e.to_string()))?;

    if result_str.len() < threshold {
        // Small — pass through as a text response.
        return Ok(CallToolResult::success(vec![text_content(result_str)]));
    }

    // Large — extract inner JSON text if available; otherwise store the envelope.
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
                let truncated: String = s.chars().take(80).collect();
                format!("\"{truncated}...\" (truncated)")
            } else {
                format!("\"{s}\"")
            }
        }
        serde_json::Value::Array(arr) => format!("[{} items]", arr.len()),
        serde_json::Value::Object(obj) => {
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

// ─── MCP server ───────────────────────────────────────────────────────────────

#[derive(Clone)]
struct ContextCutterServer {
    tool_router: ToolRouter<Self>,
}

#[tool_router]
impl ContextCutterServer {
    fn new() -> Self {
        Self {
            tool_router: Self::tool_router(),
        }
    }

    /// Fetch a JSON URL and store the response. Returns `{handle_id, teaser}` for
    /// follow-up queries. Never dumps the full JSON into context.
    #[tool(
        name = "fetch_json_cutted",
        description = "Fetch a JSON URL and store the response. Returns {handle_id, teaser}. \
                       Use query_handle with the returned handle_id to extract specific fields \
                       without loading the full payload into context."
    )]
    async fn fetch_json_cutted(
        &self,
        Parameters(params): Parameters<FetchParams>,
    ) -> Result<String, String> {
        #[instrument(skip(params), fields(url = %params.url, method = ?params.method))]
        async fn inner(params: FetchParams) -> Result<String, ContextCutterError> {
            validate_https_url(&params.url)?;

            let url = params.url.clone();
            let method = params
                .method
                .clone()
                .unwrap_or_else(|| "GET".to_string())
                .to_uppercase();
            if method.as_bytes().contains(&0) {
                return Err(ContextCutterError::Validation(
                    "method must not contain null bytes".to_string(),
                ));
            }
            let headers = params.headers.clone();
            let body = params.body.clone();
            let timeout = params.timeout_seconds.unwrap_or(45.0);
            let max_bytes = max_payload_bytes();

            // ureq is synchronous — run it off the async executor.
            let json_str =
                tokio::task::spawn_blocking(move || -> Result<String, ContextCutterError> {
                    let duration = std::time::Duration::from_secs_f64(timeout);
                    let agent = ureq::AgentBuilder::new().timeout(duration).build();

                    let mut req = agent.request(&method, &url);
                    if let Some(ref hdrs) = headers {
                        for (k, v) in hdrs {
                            if k.as_bytes().contains(&0) || v.as_bytes().contains(&0) {
                                return Err(ContextCutterError::Validation(
                                    "headers must not contain null bytes".to_string(),
                                ));
                            }
                            req = req.set(k, v);
                        }
                    }

                    // Pre-serialize body so the HTTP branches have the same return type.
                    let body_str: Option<String> = if let Some(ref b) = body {
                        Some(
                            serde_json::to_string(b)
                                .map_err(|e| ContextCutterError::Serialize(e.to_string()))?,
                        )
                    } else {
                        None
                    };

                    // Perform the HTTP call.
                    let call_result = if let Some(ref s) = body_str {
                        req.set("Content-Type", "application/json").send_string(s)
                    } else {
                        req.call()
                    };

                    match call_result {
                        Ok(r) => read_response_with_limit(r, max_bytes),
                        // Non-2xx: still try to read the body — it may contain useful JSON.
                        Err(ureq::Error::Status(code, r)) => {
                            info!(
                                status = code,
                                "received non-2xx response; attempting body parse"
                            );
                            read_response_with_limit(r, max_bytes)
                        }
                        Err(e) => Err(ContextCutterError::RequestFailed(e.to_string())),
                    }
                })
                .await
                .map_err(|e| ContextCutterError::Internal(format!("spawn_blocking panic: {e}")))?;

            let json_str = json_str?;

            // Validate JSON before storing.
            serde_json::from_str::<serde_json::Value>(&json_str).map_err(|e| {
                ContextCutterError::InvalidJson(format!("response is not valid JSON: {e}"))
            })?;

            let handle_id = engine_store(&json_str)?;
            let teaser_str = engine_teaser(&handle_id)?;
            let teaser: serde_json::Value =
                serde_json::from_str(&teaser_str).unwrap_or(serde_json::Value::Null);

            info!(
                handle_id = handle_id.as_str(),
                "stored fetched JSON payload"
            );

            let out = serde_json::json!({
                "handle_id": handle_id,
                "teaser": teaser,
            });
            Ok(out.to_string())
        }

        inner(params).await.map_err(boundary_error)
    }

    /// Extract a specific value from a stored JSON payload using a JSONPath expression.
    #[tool(
        name = "query_handle",
        description = "Extract a value from a previously stored JSON payload using JSONPath. \
                       Accepts full JSONPath ($.foo.bar) or dot notation (foo.bar). \
                       Returns the matched value as JSON or null."
    )]
    fn query_handle(&self, Parameters(params): Parameters<QueryParams>) -> Result<String, String> {
        #[instrument(skip(params), fields(handle_id = %params.handle_id))]
        fn inner(params: QueryParams) -> Result<String, ContextCutterError> {
            validate_query_inputs(&params.handle_id, &params.json_path)?;
            let result = engine_query(&params.handle_id, &params.json_path)?;
            info!("query executed successfully");
            Ok(result)
        }

        inner(params).map_err(boundary_error)
    }
}

#[tool_handler]
impl ServerHandler for ContextCutterServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build()).with_instructions(
            "ContextCutter eliminates JSON bloat in LLM agentic workflows. \
             Call fetch_json_cutted to retrieve and store a JSON API response, \
             then call query_handle with the returned handle_id to extract only the \
             fields you need — without ever loading the full payload into context.",
        )
    }
}

// ─── Entry point ─────────────────────────────────────────────────────────────

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

    #[test]
    fn proxy_server_exposes_query_handle_in_tool_list() {
        let server = ProxyServer::new(
            "https://example.com/mcp".to_string(),
            vec![],
            2048,
            vec![],
        );
        let all_tools = server.all_tools();
        assert!(all_tools.iter().any(|t| t.name == "query_handle"));
    }

    #[test]
    fn intercept_if_large_passes_through_small_result() {
        let result = serde_json::json!({ "content": [{"type":"text","text":"hi"}], "isError": false });
        let call_result = intercept_if_large(result, 2048).unwrap();
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
        assert!(result.len() <= 100);
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

    #[test]
    fn localhost_proxy_url_acceptance() {
        // Accepted URLs
        assert!(is_localhost_proxy_url("http://localhost"));
        assert!(is_localhost_proxy_url("http://localhost:8080"));
        assert!(is_localhost_proxy_url("http://localhost/mcp"));
        assert!(is_localhost_proxy_url("http://localhost:3000/mcp"));
        assert!(is_localhost_proxy_url("http://127.0.0.1"));
        assert!(is_localhost_proxy_url("http://127.0.0.1:9090/mcp"));

        // Rejected — subdomain bypass attempt
        assert!(!is_localhost_proxy_url("http://localhost.attacker.com"));
        assert!(!is_localhost_proxy_url("http://127.0.0.1.evil.com"));

        // Rejected — non-http schemes are handled by the outer https check, not this fn
        assert!(!is_localhost_proxy_url("https://localhost:8080"));

        // Rejected — completely unrelated
        assert!(!is_localhost_proxy_url("http://example.com"));
    }
}

/// Returns true if `url` is a localhost URL safe for proxy use in tests.
/// Only accepts `http://localhost`, `http://localhost:<port>`, `http://localhost/<path>`,
/// and equivalents for `http://127.0.0.1`.
fn is_localhost_proxy_url(url: &str) -> bool {
    let tail_ok = |prefix: &str| {
        matches!(
            url[prefix.len()..].bytes().next(),
            None | Some(b':') | Some(b'/') | Some(b'?') | Some(b'#')
        )
    };
    if url.starts_with("http://127.0.0.1") {
        tail_ok("http://127.0.0.1")
    } else if url.starts_with("http://localhost") {
        tail_ok("http://localhost")
    } else {
        false
    }
}

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

    if !upstream_url.starts_with("https://") && !is_localhost_proxy_url(upstream_url) {
        eprintln!(
            "context-cutter-mcp: --proxy URL must use https:// \
             (or http://localhost / http://127.0.0.1 for local testing)"
        );
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
        run_proxy_mode(upstream_url, args.proxy_threshold, &args.proxy_header).await;
    } else {
        run_normal_mode().await;
    }
}
