//! Pure-Rust engine functions — no PyO3 dependency.
//!
//! These are used by:
//! - The Rust MCP binary (`src/bin/mcp.rs`) directly.
//! - The PyO3 bindings in `src/lib.rs` (via thin wrappers that convert errors).

use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::error::ContextCutterError;
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
pub fn compute_handle_id(value: &Value) -> Result<String, ContextCutterError> {
    let canonical = canonicalize_json_value(value);
    let canonical_json = serde_json::to_string(&canonical)
        .map_err(|e| ContextCutterError::Serialize(e.to_string()))?;
    let digest = Sha256::digest(canonical_json.as_bytes());
    let digest_hex = format!("{digest:x}");
    Ok(format!("hdl_{}", &digest_hex[..12]))
}

/// Parses `json_str`, stores the value in the global store, and returns the handle_id.
pub fn engine_store(json_str: &str) -> Result<String, ContextCutterError> {
    if json_str.as_bytes().contains(&0) {
        return Err(ContextCutterError::Validation(
            "json payload must not contain null bytes".to_string(),
        ));
    }

    let value: Value = serde_json::from_str(json_str)
        .map_err(|e| ContextCutterError::InvalidJson(e.to_string()))?;
    let handle_id = compute_handle_id(&value)?;
    store::global_store_insert(handle_id.clone(), value);
    Ok(handle_id)
}

/// Returns the teaser JSON string for the given `handle_id`.
pub fn engine_teaser(handle_id: &str) -> Result<String, ContextCutterError> {
    if handle_id.trim().is_empty() {
        return Err(ContextCutterError::Validation(
            "handle_id must not be empty".to_string(),
        ));
    }
    let value = store::global_store_get(handle_id)
        .ok_or_else(|| ContextCutterError::UnknownHandle(handle_id.to_string()))?;
    Ok(parser::generate_teaser_from_value(&value))
}

/// Executes a JSONPath query against the stored payload for `handle_id`.
pub fn engine_query(handle_id: &str, json_path: &str) -> Result<String, ContextCutterError> {
    if handle_id.trim().is_empty() {
        return Err(ContextCutterError::Validation(
            "handle_id must not be empty".to_string(),
        ));
    }
    let value = store::global_store_get(handle_id)
        .ok_or_else(|| ContextCutterError::UnknownHandle(handle_id.to_string()))?;
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

    #[test]
    fn engine_store_rejects_null_bytes() {
        let json_with_null = "{\x00\"a\": 1}";
        let err = engine_store(json_with_null).expect_err("null bytes should fail");
        assert!(err.to_string().contains("null bytes"));
    }

    #[test]
    fn engine_store_rejects_invalid_json() {
        let err = engine_store("{not valid json}").expect_err("invalid json should fail");
        assert!(err.to_string().contains("invalid JSON"));
    }

    #[test]
    fn engine_teaser_rejects_empty_handle_id() {
        let err = engine_teaser("   ").expect_err("empty handle_id should fail");
        assert!(err.to_string().contains("empty"));
    }

    #[test]
    fn engine_teaser_returns_error_for_unknown_handle() {
        let err = engine_teaser("hdl_nonexistent").expect_err("unknown handle should fail");
        assert!(err.to_string().contains("unknown handle_id"));
    }

    #[test]
    fn engine_query_rejects_empty_handle_id() {
        let err = engine_query("", "$.x").expect_err("empty handle_id should fail");
        assert!(err.to_string().contains("empty"));
    }

    #[test]
    fn engine_query_returns_error_for_unknown_handle() {
        let err =
            engine_query("hdl_doesnotexist_xyz", "$.x").expect_err("unknown handle should fail");
        assert!(err.to_string().contains("unknown handle_id"));
    }

    #[test]
    fn canonicalize_json_value_handles_arrays() {
        let value = json!([{"b": 2, "a": 1}, {"y": 9, "x": 8}]);
        let canonical = canonicalize_json_value(&value);
        // Object keys inside arrays should be sorted.
        if let Value::Array(arr) = &canonical {
            if let Value::Object(obj) = &arr[0] {
                let keys: Vec<&String> = obj.keys().collect();
                assert_eq!(keys, vec!["a", "b"]);
            } else {
                panic!("expected object in array");
            }
        } else {
            panic!("expected array");
        }
    }
}
