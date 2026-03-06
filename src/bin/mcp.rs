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
    model::{ServerCapabilities, ServerInfo},
    schemars, tool, tool_handler, tool_router,
    transport::stdio,
    ServerHandler, ServiceExt,
};
use serde::Deserialize;
use std::collections::HashMap;
use std::io::Read;
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
                            info!(status = code, "received non-2xx response; attempting body parse");
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

            info!(handle_id = handle_id.as_str(), "stored fetched JSON payload");

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

#[tokio::main]
async fn main() {
    init_tracing();
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
