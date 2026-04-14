//! `context-cutter-mcp` — Rust MCP stdio server.
//!
//! Exposes two tools over the Model Context Protocol:
//!
//! - `fetch_json_cutted`: fetch a JSON endpoint, store it, return `{handle_id, teaser}`.
//! - `query_handle`: extract a specific value from a stored payload via JSONPath.

use clap::Parser;
use context_cutter::engine::{engine_query, engine_store, engine_teaser};
use context_cutter::error::ContextCutterError;
use context_cutter::store::start_background_sweeper;
use rmcp::service::RequestContext;
use rmcp::service::RoleServer;
use rmcp::{
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{
        Annotated, CallToolRequestParams, CallToolResult, ListToolsResult, PaginatedRequestParams,
        RawContent, ServerCapabilities, ServerInfo, Tool,
    },
    schemars, tool, tool_handler, tool_router,
    transport::stdio,
    ErrorData as McpError, ServerHandler, ServiceExt,
};
use serde::Deserialize;
use std::collections::HashMap;
use std::io::Read;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::sync::{Notify, RwLock};
use tracing::{error, info, instrument, warn};
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

    /// Path to a file containing a Bearer token for the upstream MCP server.
    /// The file is re-read on every request, so token refreshes work automatically.
    /// Claude Code saves tokens to ~/.claude/<server-name>-token after OAuth login.
    /// Example: --proxy-token-file ~/.claude/clickup-status-token
    #[arg(long)]
    proxy_token_file: Option<String>,
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
    // IMPORTANT: MCP servers communicate over stdio — logs MUST go to stderr,
    // never stdout. stdout is reserved for JSON-RPC protocol messages.
    let format = std::env::var("CONTEXT_CUTTER_LOG_FORMAT").unwrap_or_else(|_| "plain".to_string());
    let env_filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

    if format == "json" {
        tracing_subscriber::fmt()
            .json()
            .with_writer(std::io::stderr)
            .with_env_filter(env_filter)
            .with_current_span(false)
            .with_target(false)
            .init();
    } else {
        tracing_subscriber::fmt()
            .with_writer(std::io::stderr)
            .with_env_filter(env_filter)
            .with_target(false)
            .init();
    }
}

fn boundary_error(err: ContextCutterError) -> String {
    err.to_string()
}

// ─── Proxy helpers ────────────────────────────────────────────────────────────

/// Expand a leading `~` to the user's home directory.
fn expand_tilde(path: &str) -> std::path::PathBuf {
    if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = std::env::var_os("HOME") {
            return std::path::PathBuf::from(home).join(rest);
        }
    }
    std::path::PathBuf::from(path)
}

/// Read a Bearer token from a file, trimming whitespace.
/// Returns None if the file doesn't exist or can't be read.
fn read_token_file(path: &std::path::Path) -> Option<String> {
    std::fs::read_to_string(path)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

// ─── OAuth helpers ────────────────────────────────────────────────────────────

/// Read `n` cryptographically random bytes from /dev/urandom.
fn random_bytes(n: usize) -> Vec<u8> {
    let mut buf = vec![0u8; n];
    if let Ok(mut f) = std::fs::File::open("/dev/urandom") {
        let _ = f.read_exact(&mut buf);
    }
    buf
}

/// Base64url encode without padding (RFC 4648 §5, used for PKCE).
fn base64url_encode(input: &[u8]) -> String {
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let b0 = chunk[0] as usize;
        let b1 = if chunk.len() > 1 {
            chunk[1] as usize
        } else {
            0
        };
        let b2 = if chunk.len() > 2 {
            chunk[2] as usize
        } else {
            0
        };
        out.push(CHARS[b0 >> 2] as char);
        out.push(CHARS[((b0 & 3) << 4) | (b1 >> 4)] as char);
        if chunk.len() > 1 {
            out.push(CHARS[((b1 & 15) << 2) | (b2 >> 6)] as char);
        }
        if chunk.len() > 2 {
            out.push(CHARS[b2 & 63] as char);
        }
    }
    out
}

/// Decode base64 / base64url with optional padding.
fn base64url_decode(input: &str) -> Option<Vec<u8>> {
    let decode_char = |c: u8| -> Option<u8> {
        match c {
            b'A'..=b'Z' => Some(c - b'A'),
            b'a'..=b'z' => Some(c - b'a' + 26),
            b'0'..=b'9' => Some(c - b'0' + 52),
            b'+' | b'-' => Some(62),
            b'/' | b'_' => Some(63),
            b'=' => Some(0),
            _ => None,
        }
    };
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len() * 3 / 4);
    for chunk in bytes.chunks(4) {
        let a = decode_char(chunk[0])?;
        let b = decode_char(*chunk.get(1).unwrap_or(&b'='))?;
        let c = decode_char(*chunk.get(2).unwrap_or(&b'='))?;
        let d = decode_char(*chunk.get(3).unwrap_or(&b'='))?;
        out.push((a << 2) | (b >> 4));
        if chunk.get(2).filter(|&&x| x != b'=').is_some() {
            out.push((b << 4) | (c >> 2));
        }
        if chunk.get(3).filter(|&&x| x != b'=').is_some() {
            out.push((c << 6) | d);
        }
    }
    Some(out)
}

/// Returns true only if the token is a parseable JWT whose `exp` is in the past.
///
/// Returns **false** (treat as valid) for:
/// - JWE tokens (encrypted — payload is ciphertext, not JSON)
/// - Opaque / non-JWT tokens
/// - Tokens with no `exp` claim
///
/// These token types are used as-is; if the server rejects them with 401,
/// the OAuth flow will re-run reactively.
fn is_jwt_expired(token: &str) -> bool {
    let payload = token.split('.').nth(1).unwrap_or("");
    let decoded = match base64url_decode(payload) {
        Some(d) => d,
        None => return false, // can't decode → assume valid
    };
    let json: serde_json::Value = match serde_json::from_slice(&decoded) {
        Ok(v) => v,
        Err(_) => return false, // not JSON (e.g. JWE ciphertext) → assume valid
    };
    let exp = match json.get("exp").and_then(|v| v.as_u64()) {
        Some(e) => e,
        None => return false, // no exp claim → assume valid
    };
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    now >= exp
}

/// Percent-encode a string for use in query parameters.
fn url_encode(s: &str) -> String {
    s.bytes()
        .flat_map(|b| {
            if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
                vec![b as char]
            } else {
                format!("%{b:02X}").chars().collect::<Vec<_>>()
            }
        })
        .collect()
}

struct OAuthMeta {
    authorization_endpoint: String,
    token_endpoint: String,
    registration_endpoint: Option<String>,
}

/// Fetch OAuth server metadata from `<base_url>/.well-known/oauth-authorization-server`.
fn fetch_oauth_meta_sync(base_url: &str) -> Result<OAuthMeta, ContextCutterError> {
    let url = format!("{base_url}/.well-known/oauth-authorization-server");
    let agent = ureq::AgentBuilder::new()
        .timeout(std::time::Duration::from_secs(10))
        .build();
    let body = agent
        .get(&url)
        .call()
        .map_err(|e| ContextCutterError::RequestFailed(format!("OAuth discovery failed: {e}")))?
        .into_string()
        .map_err(|e| ContextCutterError::RequestFailed(format!("OAuth discovery read: {e}")))?;
    let json: serde_json::Value = serde_json::from_str(&body)
        .map_err(|e| ContextCutterError::InvalidJson(format!("OAuth discovery JSON: {e}")))?;
    Ok(OAuthMeta {
        authorization_endpoint: json["authorization_endpoint"]
            .as_str()
            .ok_or_else(|| {
                ContextCutterError::RequestFailed("OAuth: missing authorization_endpoint".into())
            })?
            .to_string(),
        token_endpoint: json["token_endpoint"]
            .as_str()
            .ok_or_else(|| {
                ContextCutterError::RequestFailed("OAuth: missing token_endpoint".into())
            })?
            .to_string(),
        registration_endpoint: json["registration_endpoint"].as_str().map(String::from),
    })
}

/// Dynamically register a public OAuth client, returning the assigned client_id.
fn register_oauth_client_sync(
    reg_endpoint: &str,
    redirect_uri: &str,
) -> Result<String, ContextCutterError> {
    let agent = ureq::AgentBuilder::new()
        .timeout(std::time::Duration::from_secs(10))
        .build();
    let body = serde_json::json!({
        "client_name": "context-cutter-proxy",
        "redirect_uris": [redirect_uri],
        "grant_types": ["authorization_code"],
        "response_types": ["code"],
        "token_endpoint_auth_method": "none",
    })
    .to_string();
    let resp_str = agent
        .post(reg_endpoint)
        .set("Content-Type", "application/json")
        .send_string(&body)
        .map_err(|e| ContextCutterError::RequestFailed(format!("client registration failed: {e}")))?
        .into_string()
        .map_err(|e| ContextCutterError::RequestFailed(e.to_string()))?;
    let json: serde_json::Value = serde_json::from_str(&resp_str)
        .map_err(|e| ContextCutterError::InvalidJson(e.to_string()))?;
    json["client_id"].as_str().map(String::from).ok_or_else(|| {
        ContextCutterError::RequestFailed(format!(
            "client registration: missing client_id in: {resp_str}"
        ))
    })
}

/// Exchange an authorization code for an access token using PKCE.
fn exchange_code_sync(
    token_endpoint: &str,
    code: &str,
    client_id: &str,
    code_verifier: &str,
    redirect_uri: &str,
) -> Result<String, ContextCutterError> {
    let agent = ureq::AgentBuilder::new()
        .timeout(std::time::Duration::from_secs(15))
        .build();
    let body = format!(
        "grant_type=authorization_code&code={}&client_id={}&code_verifier={}&redirect_uri={}",
        url_encode(code),
        url_encode(client_id),
        url_encode(code_verifier),
        url_encode(redirect_uri),
    );
    let resp_str = agent
        .post(token_endpoint)
        .set("Content-Type", "application/x-www-form-urlencoded")
        .send_string(&body)
        .map_err(|e| ContextCutterError::RequestFailed(format!("token exchange failed: {e}")))?
        .into_string()
        .map_err(|e| ContextCutterError::RequestFailed(e.to_string()))?;
    let json: serde_json::Value = serde_json::from_str(&resp_str)
        .map_err(|e| ContextCutterError::InvalidJson(e.to_string()))?;
    json["access_token"]
        .as_str()
        .map(String::from)
        .ok_or_else(|| {
            ContextCutterError::RequestFailed(format!(
                "token exchange: missing access_token (response: {resp_str})"
            ))
        })
}

/// Start a one-shot local HTTP server, wait for the OAuth callback, and return the `code`.
async fn wait_for_oauth_callback(
    listener: tokio::net::TcpListener,
) -> Result<String, ContextCutterError> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let (mut stream, _) =
        tokio::time::timeout(std::time::Duration::from_secs(300), listener.accept())
            .await
            .map_err(|_| {
                ContextCutterError::RequestFailed(
                    "OAuth: timed out waiting for browser login (5 min)".into(),
                )
            })?
            .map_err(|e| {
                ContextCutterError::RequestFailed(format!("OAuth callback accept: {e}"))
            })?;

    let mut buf = [0u8; 8192];
    let n = stream
        .read(&mut buf)
        .await
        .map_err(|e| ContextCutterError::RequestFailed(format!("OAuth callback read: {e}")))?;

    // Parse code from "GET /callback?code=xxx&state=yyy HTTP/1.1"
    let request = String::from_utf8_lossy(&buf[..n]);
    let first_line = request.lines().next().unwrap_or("");
    let path = first_line.split_whitespace().nth(1).unwrap_or("");
    let query = path.split_once('?').map(|(_, q)| q).unwrap_or("");

    let mut code = String::new();
    for param in query.split('&') {
        if let Some(v) = param.strip_prefix("code=") {
            code = v.to_string();
            break;
        }
    }

    // Respond with a success page so the browser doesn't hang.
    let _ = stream
        .write_all(
            b"HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nConnection: close\r\n\r\n\
          <html><body style='font-family:sans-serif;text-align:center;padding:60px'>\
          <h2>\xe2\x9c\x93 Authentication successful</h2>\
          <p>You can close this tab and return to Claude.</p>\
          </body></html>",
        )
        .await;

    if code.is_empty() {
        return Err(ContextCutterError::RequestFailed(
            "OAuth callback: missing `code` parameter".into(),
        ));
    }
    Ok(code)
}

/// Run the full OAuth 2.0 + PKCE browser flow, save the token, and return it.
async fn run_oauth_flow(
    upstream_url: &str,
    token_file: &std::path::Path,
) -> Result<String, ContextCutterError> {
    // Derive base URL (scheme + host only).
    let base_url = {
        let after_scheme = upstream_url.find("://").map(|i| i + 3).unwrap_or(0);
        let host_end = upstream_url[after_scheme..]
            .find('/')
            .map(|i| i + after_scheme)
            .unwrap_or(upstream_url.len());
        upstream_url[..host_end].to_string()
    };

    // Discover OAuth endpoints.
    let meta = tokio::task::spawn_blocking({
        let base = base_url.clone();
        move || fetch_oauth_meta_sync(&base)
    })
    .await
    .map_err(|e| ContextCutterError::Internal(format!("spawn_blocking: {e}")))?
    .map_err(|e| ContextCutterError::RequestFailed(format!("OAuth discovery: {e}")))?;

    // Bind local callback server on a random port.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|e| ContextCutterError::RequestFailed(format!("callback server: {e}")))?;
    let port = listener
        .local_addr()
        .map_err(|e| ContextCutterError::RequestFailed(format!("callback port: {e}")))?
        .port();
    let redirect_uri = format!("http://127.0.0.1:{port}/callback");

    // Register a public client to get a client_id.
    let client_id = if let Some(ref reg_ep) = meta.registration_endpoint {
        let ep = reg_ep.clone();
        let ru = redirect_uri.clone();
        tokio::task::spawn_blocking(move || register_oauth_client_sync(&ep, &ru))
            .await
            .map_err(|e| ContextCutterError::Internal(e.to_string()))??
    } else {
        "context-cutter-proxy".to_string()
    };

    // Generate PKCE code verifier + S256 challenge.
    let code_verifier = base64url_encode(&random_bytes(32));
    let code_challenge = {
        use sha2::{Digest, Sha256};
        base64url_encode(&Sha256::digest(code_verifier.as_bytes()))
    };
    let state = base64url_encode(&random_bytes(16));

    // Build authorization URL.
    let auth_url = format!(
        "{}?client_id={}&response_type=code&redirect_uri={}&scope=read+write\
         &state={}&code_challenge={}&code_challenge_method=S256",
        meta.authorization_endpoint,
        url_encode(&client_id),
        url_encode(&redirect_uri),
        url_encode(&state),
        url_encode(&code_challenge),
    );

    // Open browser. Fall back to printing the URL if `open` isn't available.
    eprintln!("\n[context-cutter] Authentication required for upstream MCP.");
    let opened = std::process::Command::new("open")
        .arg(&auth_url)
        .spawn()
        .is_ok();
    if !opened {
        eprintln!("[context-cutter] Could not open browser automatically.");
    }
    eprintln!("[context-cutter] Open this URL to authenticate:\n\n  {auth_url}\n");

    // Wait for the browser to complete the OAuth flow.
    info!(port, "waiting for OAuth callback");
    let code = wait_for_oauth_callback(listener).await?;

    // Exchange authorization code for access token.
    let token = {
        let te = meta.token_endpoint.clone();
        let ci = client_id.clone();
        let cv = code_verifier.clone();
        let ru = redirect_uri.clone();
        let c = code.clone();
        tokio::task::spawn_blocking(move || exchange_code_sync(&te, &c, &ci, &cv, &ru))
            .await
            .map_err(|e| ContextCutterError::Internal(e.to_string()))??
    };

    // Persist token to the same file Claude Code uses.
    std::fs::write(token_file, &token)
        .map_err(|e| ContextCutterError::RequestFailed(format!("failed to save token: {e}")))?;

    info!("OAuth flow complete, token saved");
    eprintln!("[context-cutter] Authentication successful!\n");

    Ok(token)
}

/// Resolve the Authorization header for upstream calls.
///
/// If a token file is configured:
/// - Returns the stored token if still valid.
/// - Runs the OAuth browser flow if the token is missing or expired.
///
/// Called at proxy startup (init handshake) and before each tool call.
async fn get_authed_headers(
    upstream_url: &str,
    extra_headers: &[(String, String)],
    token_file: Option<&std::path::Path>,
) -> Vec<(String, String)> {
    let mut headers = extra_headers.to_vec();
    let Some(path) = token_file else {
        return headers;
    };

    let existing = read_token_file(path);
    let needs_refresh = existing.as_deref().map(is_jwt_expired).unwrap_or(true);

    let token = if needs_refresh {
        match run_oauth_flow(upstream_url, path).await {
            Ok(t) => {
                info!("OAuth token refreshed");
                Some(t)
            }
            Err(e) => {
                error!(error = %e, "OAuth flow failed");
                if existing.is_some() {
                    warn!("using expired token as fallback");
                }
                existing
            }
        }
    } else {
        existing
    };

    if let Some(t) = token {
        headers.push(("Authorization".to_string(), format!("Bearer {t}")));
    }
    headers
}

/// Parse `--proxy-header` strings of the form `"Key: Value"` into pairs.
fn parse_proxy_headers(raw: &[String]) -> Result<Vec<(String, String)>, String> {
    raw.iter()
        .map(|h| {
            h.split_once(": ")
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .ok_or_else(|| format!("invalid --proxy-header (expected 'Key: Value'): {h}"))
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

    // Some HTTP MCPs return SSE (text/event-stream) even for synchronous requests.
    let is_sse = response
        .header("Content-Type")
        .map(|ct| ct.contains("event-stream"))
        .unwrap_or(false);

    let raw = read_response_with_limit(response, max_payload_bytes())?;

    // If SSE, extract the JSON payload from the first "data: {...}" line.
    let response_str = if is_sse {
        raw.lines()
            .find_map(|line| line.strip_prefix("data: ").map(|d| d.trim().to_string()))
            .filter(|d| !d.is_empty() && d != "[DONE]")
            .ok_or_else(|| {
                ContextCutterError::RequestFailed("SSE response contained no data event".into())
            })?
    } else {
        raw
    };

    let json: serde_json::Value = serde_json::from_str(&response_str)
        .map_err(|e| ContextCutterError::InvalidJson(format!("upstream response: {e}")))?;

    if let Some(err) = json.get("error") {
        return Err(ContextCutterError::RequestFailed(format!(
            "upstream MCP error: {err}"
        )));
    }

    json.get("result").cloned().ok_or_else(|| {
        ContextCutterError::RequestFailed("upstream response missing 'result' field".to_string())
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

/// ProxyServer starts immediately (so Claude Code gets an MCP response right away),
/// then populates upstream tools asynchronously once OAuth + handshake complete.
#[derive(Clone)]
struct ProxyServer {
    upstream_url: String,
    upstream_headers: Arc<Vec<(String, String)>>,
    threshold: usize,
    /// Populated after upstream init; guarded so call_tool can wait if needed.
    upstream_tools: Arc<RwLock<Vec<Tool>>>,
    /// Set to 1 (Release) before notify_waiters(). Durable — late callers still see 1.
    init_complete: Arc<std::sync::atomic::AtomicUsize>,
    /// Wakes up waiters when init completes. Not durable on its own — use with init_complete.
    init_done: Arc<Notify>,
    next_id: Arc<AtomicU64>,
    token_file: Option<std::path::PathBuf>,
}

impl ProxyServer {
    /// Create a server that is ready to accept MCP messages but has no upstream tools yet.
    /// Call `set_upstream_tools` once the upstream handshake (and OAuth) completes.
    fn new_pending(
        upstream_url: String,
        upstream_headers: Vec<(String, String)>,
        threshold: usize,
        token_file: Option<std::path::PathBuf>,
    ) -> Self {
        Self {
            upstream_url,
            upstream_headers: Arc::new(upstream_headers),
            threshold,
            upstream_tools: Arc::new(RwLock::new(Vec::new())),
            init_complete: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            init_done: Arc::new(Notify::new()),
            next_id: Arc::new(AtomicU64::new(3)),
            token_file,
        }
    }

    /// Populate upstream tools and signal readiness. Called once after OAuth + tools/list.
    async fn set_upstream_tools(&self, tools: Vec<Tool>) {
        *self.upstream_tools.write().await = tools;
        self.init_complete
            .store(1, std::sync::atomic::Ordering::Release);
        self.init_done.notify_waiters();
    }

    fn next_id(&self) -> u64 {
        self.next_id.fetch_add(1, Ordering::Relaxed)
    }

    /// Wait until the upstream handshake is done, then return all tools.
    async fn ready_tools(&self) -> Vec<Tool> {
        // Create the notified future BEFORE the flag check to avoid missing a notification
        // fired between the check and the await.
        let notified = self.init_done.notified();
        tokio::pin!(notified);

        if self
            .init_complete
            .load(std::sync::atomic::Ordering::Acquire)
            == 0
        {
            notified.await;
        }

        let mut tools = self.upstream_tools.read().await.clone();
        tools.push(query_handle_tool_def());
        tools
    }

    /// Forward a tool call to the upstream MCP.
    /// On 401, runs the OAuth flow to get a fresh token and retries once.
    async fn call_upstream(
        &self,
        url: &str,
        tool_name: &str,
        arguments: serde_json::Value,
        threshold: usize,
    ) -> Result<CallToolResult, ContextCutterError> {
        let headers = self.authed_headers().await;
        let result = self
            .try_upstream_call(url, tool_name, arguments.clone(), &headers)
            .await;

        match result {
            // 401 — token rejected by the server; re-run OAuth and retry once.
            Err(ref e) if e.to_string().contains("401") => {
                warn!("upstream returned 401, triggering OAuth re-auth");
                if let Some(path) = &self.token_file {
                    match run_oauth_flow(url, path).await {
                        Ok(_) => {
                            let fresh_headers = self.authed_headers().await;
                            self.try_upstream_call(url, tool_name, arguments, &fresh_headers)
                                .await
                        }
                        Err(oauth_err) => {
                            error!(error = %oauth_err, "OAuth re-auth failed");
                            result
                        }
                    }
                } else {
                    result
                }
            }
            other => other,
        }
        .and_then(|upstream_result| intercept_if_large(upstream_result, threshold))
    }

    async fn try_upstream_call(
        &self,
        url: &str,
        tool_name: &str,
        arguments: serde_json::Value,
        headers: &[(String, String)],
    ) -> Result<serde_json::Value, ContextCutterError> {
        let url = url.to_string();
        let tool_name = tool_name.to_string();
        let headers = headers.to_vec();
        let id = self.next_id();
        tokio::task::spawn_blocking(move || {
            upstream_call(
                &url,
                "tools/call",
                serde_json::json!({ "name": tool_name, "arguments": arguments }),
                &headers,
                id,
            )
        })
        .await
        .map_err(|e| ContextCutterError::Internal(format!("spawn_blocking: {e}")))?
    }

    /// Build headers for an upstream call, refreshing the OAuth token if expired.
    async fn authed_headers(&self) -> Vec<(String, String)> {
        get_authed_headers(
            &self.upstream_url,
            &self.upstream_headers,
            self.token_file.as_deref(),
        )
        .await
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

/// Perform the upstream handshake (OAuth if needed + initialize + tools/list).
/// Returns the list of upstream tools. Runs concurrently with the MCP stdio server.
async fn upstream_handshake(
    upstream_url: &str,
    extra_headers: &[(String, String)],
    token_file: Option<&std::path::Path>,
) -> Result<Vec<Tool>, ContextCutterError> {
    let url = upstream_url.to_string();

    // Auth — triggers the OAuth browser flow if the token is missing/expired.
    let hdrs = get_authed_headers(upstream_url, extra_headers, token_file).await;

    // MCP initialize
    let _init = tokio::task::spawn_blocking({
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

    // MCP tools/list
    let tools_result = tokio::task::spawn_blocking({
        let url = url.clone();
        let hdrs = hdrs.clone();
        move || upstream_call(&url, "tools/list", serde_json::json!({}), &hdrs, 2)
    })
    .await
    .map_err(|e| ContextCutterError::Internal(format!("spawn_blocking: {e}")))?
    .map_err(|e| ContextCutterError::RequestFailed(format!("tools/list failed: {e}")))?;

    let tools: Vec<Tool> = tools_result
        .get("tools")
        .and_then(|t| serde_json::from_value(t.clone()).ok())
        .unwrap_or_default();

    info!(tool_count = tools.len(), "upstream tools discovered");
    Ok(tools)
}

// ─── ServerHandler for ProxyServer ───────────────────────────────────────────

impl ServerHandler for ProxyServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build()).with_instructions(
            "Transparent MCP proxy with response interception. \
                 Large tool responses are stored as handles. \
                 Use query_handle(handle_id, \"$.field\") to extract specific fields.",
        )
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, McpError> {
        // Wait up to 10 s for the upstream handshake (OAuth + tools/list).
        // Claude Code's list_tools timeout is well above 10 s so this is safe.
        // Falls back to just [query_handle] if the handshake takes longer.
        let tools = match tokio::time::timeout(
            std::time::Duration::from_secs(10),
            self.ready_tools(),
        )
        .await
        {
            Ok(t) => t,
            Err(_) => vec![query_handle_tool_def()],
        };
        Ok(ListToolsResult::with_all_items(tools))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let tool_name = request.name.as_ref().to_string();

        // ── Local: query_handle ────────────────────────────────────────────
        if tool_name == "query_handle" {
            let args = request
                .arguments
                .as_ref()
                .ok_or_else(|| McpError::invalid_params("query_handle requires arguments", None))?;
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
        let threshold = self.threshold;
        let arguments = request
            .arguments
            .map(serde_json::Value::Object)
            .unwrap_or_else(|| serde_json::Value::Object(Default::default()));

        let upstream_result = self
            .call_upstream(&url, &tool_name, arguments, threshold)
            .await
            .map_err(|e| McpError::internal_error(e.to_string(), None));
        upstream_result
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
        assert_eq!(
            result,
            vec![
                ("Authorization".to_string(), "Bearer abc123".to_string()),
                ("X-Custom".to_string(), "value".to_string()),
            ]
        );
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

    #[tokio::test]
    async fn proxy_server_exposes_query_handle_in_tool_list() {
        let server =
            ProxyServer::new_pending("https://example.com/mcp".to_string(), vec![], 2048, None);
        // Populate tools (simulates upstream handshake completing with zero upstream tools).
        server.set_upstream_tools(vec![]).await;
        let tools = server.ready_tools().await;
        assert!(tools.iter().any(|t| t.name == "query_handle"));
    }

    #[test]
    fn intercept_if_large_passes_through_small_result() {
        let result =
            serde_json::json!({ "content": [{"type":"text","text":"hi"}], "isError": false });
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
    token_file_arg: Option<&str>,
) {
    let headers = match parse_proxy_headers(raw_headers) {
        Ok(h) => h,
        Err(e) => {
            eprintln!("context-cutter-mcp: {e}");
            std::process::exit(1);
        }
    };

    let token_file = token_file_arg.map(expand_tilde);

    if !upstream_url.starts_with("https://") && !is_localhost_proxy_url(upstream_url) {
        eprintln!(
            "context-cutter-mcp: --proxy URL must use https:// \
             (or http://localhost / http://127.0.0.1 for local testing)"
        );
        std::process::exit(1);
    }

    if let Some(ref path) = token_file {
        if !path.exists() {
            eprintln!(
                "context-cutter-mcp: --proxy-token-file not found: {}",
                path.display()
            );
            std::process::exit(1);
        }
        info!(path = %path.display(), "using token file for upstream auth");
    }

    info!(upstream_url, threshold, "starting proxy mode");

    start_background_sweeper();

    let proxy = ProxyServer::new_pending(
        upstream_url.to_string(),
        headers.clone(),
        threshold,
        token_file.clone(),
    );

    // Start serving immediately so Claude Code gets an initialize response right away.
    // list_tools will block (up to 10 s) until the background handshake completes.
    let server = match proxy.clone().serve(stdio()).await {
        Ok(s) => s,
        Err(e) => {
            error!(error = %e, "startup error");
            eprintln!("context-cutter-mcp: startup error: {e}");
            std::process::exit(1);
        }
    };

    // Run OAuth + upstream handshake in background.
    // list_tools blocks on init_done until this completes (10 s timeout).
    let proxy_bg = proxy.clone();
    let url_bg = upstream_url.to_string();
    let hdrs_bg = headers.clone();
    let tf_bg = token_file.clone();
    tokio::spawn(async move {
        match upstream_handshake(&url_bg, &hdrs_bg, tf_bg.as_deref()).await {
            Ok(tools) => {
                proxy_bg.set_upstream_tools(tools).await;
                info!("proxy ready — upstream tools loaded");
            }
            Err(e) => {
                error!(error = %e, "upstream handshake failed");
                proxy_bg.set_upstream_tools(vec![]).await;
            }
        }
    });

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
        run_proxy_mode(
            upstream_url,
            args.proxy_threshold,
            &args.proxy_header,
            args.proxy_token_file.as_deref(),
        )
        .await;
    } else {
        run_normal_mode().await;
    }
}
