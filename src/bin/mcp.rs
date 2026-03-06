//! `context-cutter-mcp` — Rust MCP stdio server.
//!
//! Exposes two tools over the Model Context Protocol:
//!
//! - `fetch_json_cutted`: fetch a JSON endpoint, store it, return `{handle_id, teaser}`.
//! - `query_handle`: extract a specific value from a stored payload via JSONPath.

use context_cutter::engine::{engine_query, engine_store, engine_teaser};
use rmcp::{
    ServerHandler, ServiceExt,
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{ServerCapabilities, ServerInfo},
    schemars, tool, tool_handler, tool_router,
    transport::stdio,
};
use serde::Deserialize;
use std::collections::HashMap;

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
        let url = params.url.clone();
        let method = params
            .method
            .clone()
            .unwrap_or_else(|| "GET".to_string())
            .to_uppercase();
        let headers = params.headers.clone();
        let body = params.body.clone();
        let timeout = params.timeout_seconds.unwrap_or(45.0);

        // ureq is synchronous — run it off the async executor.
        let json_str = tokio::task::spawn_blocking(move || -> Result<String, String> {
            let duration = std::time::Duration::from_secs_f64(timeout);
            let agent = ureq::AgentBuilder::new().timeout(duration).build();

            let mut req = agent.request(&method, &url);
            if let Some(ref hdrs) = headers {
                for (k, v) in hdrs {
                    req = req.set(k, v);
                }
            }

            // Pre-serialize body so the HTTP branches have the same return type.
            let body_str: Option<String> = if let Some(ref b) = body {
                Some(
                    serde_json::to_string(b)
                        .map_err(|e| format!("failed to serialize request body: {e}"))?,
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
                Ok(r) => r
                    .into_string()
                    .map_err(|e| format!("failed to read body: {e}")),
                // Non-2xx: still try to read the body — it may contain useful JSON.
                Err(ureq::Error::Status(_, r)) => r
                    .into_string()
                    .map_err(|e| format!("failed to read error body: {e}")),
                Err(e) => Err(format!("request failed: {e}")),
            }
        })
        .await
        .map_err(|e| format!("spawn_blocking panic: {e}"))?;

        let json_str = json_str?;

        // Validate JSON before storing.
        serde_json::from_str::<serde_json::Value>(&json_str)
            .map_err(|e| format!("response is not valid JSON: {e}"))?;

        let handle_id = engine_store(&json_str)?;
        let teaser_str = engine_teaser(&handle_id)?;
        let teaser: serde_json::Value =
            serde_json::from_str(&teaser_str).unwrap_or(serde_json::Value::Null);

        let out = serde_json::json!({
            "handle_id": handle_id,
            "teaser": teaser,
        });
        Ok(out.to_string())
    }

    /// Extract a specific value from a stored JSON payload using a JSONPath expression.
    #[tool(
        name = "query_handle",
        description = "Extract a value from a previously stored JSON payload using JSONPath. \
                       Accepts full JSONPath ($.foo.bar) or dot notation (foo.bar). \
                       Returns the matched value as JSON or null."
    )]
    fn query_handle(
        &self,
        Parameters(params): Parameters<QueryParams>,
    ) -> Result<String, String> {
        engine_query(&params.handle_id, &params.json_path)
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

#[tokio::main]
async fn main() {
    let server = match ContextCutterServer::new().serve(stdio()).await {
        Ok(s) => s,
        Err(e) => {
            eprintln!("context-cutter-mcp: startup error: {e}");
            std::process::exit(1);
        }
    };
    if let Err(e) = server.waiting().await {
        eprintln!("context-cutter-mcp: server error: {e}");
        std::process::exit(1);
    }
}
