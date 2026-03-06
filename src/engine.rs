//! Pure-Rust engine functions — no PyO3 dependency.
//!
//! These are used by:
//! - The Rust MCP binary (`src/bin/mcp.rs`) directly.
//! - The PyO3 bindings in `src/lib.rs` (via thin wrappers that convert errors).

use sha2::{Digest, Sha256};
use serde_json::Value;

use crate::parser;
use crate::store;

/// Recursively sort object keys so that the SHA-256 hash is key-order-independent.
pub fn canonicalize_json_value(value: &Value) -> Value {
    match value {
        Value::Object(obj) => {
            let mut keys: Vec<&String> = obj.keys().collect();
            keys.sort();
            let mut out = serde_json::Map::new();
            for key in keys {
                if let Some(inner) = obj.get(key) {
                    out.insert(key.clone(), canonicalize_json_value(inner));
                }
            }
            Value::Object(out)
        }
        Value::Array(arr) => Value::Array(arr.iter().map(canonicalize_json_value).collect()),
        _ => value.clone(),
    }
}

/// Returns `hdl_<12-hex>` for a given JSON value — deterministic and key-order-independent.
pub fn compute_handle_id(value: &Value) -> Result<String, String> {
    let canonical = canonicalize_json_value(value);
    let canonical_json = serde_json::to_string(&canonical)
        .map_err(|e| format!("failed to serialize payload: {e}"))?;
    let digest = Sha256::digest(canonical_json.as_bytes());
    let digest_hex = format!("{digest:x}");
    Ok(format!("hdl_{}", &digest_hex[..12]))
}

/// Parses `json_str`, stores the value in the global store, and returns the handle_id.
pub fn engine_store(json_str: &str) -> Result<String, String> {
    let value: Value = serde_json::from_str(json_str)
        .map_err(|e| format!("invalid JSON payload: {e}"))?;
    let handle_id = compute_handle_id(&value)?;
    store::global_store_insert(handle_id.clone(), value);
    Ok(handle_id)
}

/// Returns the teaser JSON string for the given `handle_id`.
pub fn engine_teaser(handle_id: &str) -> Result<String, String> {
    let value = store::global_store_get(handle_id)
        .ok_or_else(|| format!("unknown handle_id: {handle_id}"))?;
    Ok(parser::generate_teaser_from_value(&value))
}

/// Executes a JSONPath query against the stored payload for `handle_id`.
pub fn engine_query(handle_id: &str, json_path: &str) -> Result<String, String> {
    let value = store::global_store_get(handle_id)
        .ok_or_else(|| format!("unknown handle_id: {handle_id}"))?;
    parser::query_json_path(&value, json_path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn compute_handle_id_is_stable_for_same_payload() {
        let value = json!({"b": 2, "a": 1, "nested": {"z": true, "x": [1, 2]}});
        let h1 = compute_handle_id(&value).expect("handle must be generated");
        let h2 = compute_handle_id(&value).expect("handle must be generated");
        assert_eq!(h1, h2);
        assert!(h1.starts_with("hdl_"));
    }

    #[test]
    fn compute_handle_id_ignores_object_key_order() {
        let left = json!({"a": 1, "b": {"x": 1, "y": 2}});
        let right = json!({"b": {"y": 2, "x": 1}, "a": 1});
        let h1 = compute_handle_id(&left).expect("handle must be generated");
        let h2 = compute_handle_id(&right).expect("handle must be generated");
        assert_eq!(h1, h2);
    }

    #[test]
    fn compute_handle_id_differs_for_different_payloads() {
        let h1 = compute_handle_id(&json!({"a": 1})).expect("handle must be generated");
        let h2 = compute_handle_id(&json!({"a": 2})).expect("handle must be generated");
        assert_ne!(h1, h2);
    }

    #[test]
    fn engine_store_and_query_round_trip() {
        let json_str = r#"{"user": {"name": "alice", "id": 42}}"#;
        let handle_id = engine_store(json_str).expect("store should succeed");
        assert!(handle_id.starts_with("hdl_"));

        let result = engine_query(&handle_id, "$.user.name").expect("query should succeed");
        assert_eq!(result, "\"alice\"");
    }

    #[test]
    fn engine_teaser_returns_schema_shape() {
        let json_str = r#"{"contributors": [{"login": "octocat"}], "meta": {"total": 1}}"#;
        let handle_id = engine_store(json_str).expect("store should succeed");
        let teaser_str = engine_teaser(&handle_id).expect("teaser should succeed");
        let teaser: serde_json::Value =
            serde_json::from_str(&teaser_str).expect("teaser must be valid JSON");
        assert_eq!(teaser["_teaser"], json!(true));
        assert_eq!(teaser["_type"], json!("object"));
    }
}
