use thiserror::Error;

/// Typed errors used across the ContextCutter Rust engine and MCP server.
#[derive(Debug, Error)]
pub enum ContextCutterError {
    #[error("validation error: {0}")]
    Validation(String),

    #[error("invalid JSON payload: {0}")]
    InvalidJson(String),

    #[error("failed to serialize payload: {0}")]
    Serialize(String),

    #[error("unknown handle_id: {0}")]
    UnknownHandle(String),

    #[error("invalid json path: {0}")]
    InvalidJsonPath(String),

    #[error("request failed: {0}")]
    RequestFailed(String),

    #[error("payload too large: {actual_bytes} bytes exceeds limit {max_bytes} bytes")]
    PayloadTooLarge { actual_bytes: usize, max_bytes: usize },

    #[error("internal error: {0}")]
    Internal(String),
}
