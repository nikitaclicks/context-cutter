//! PyO3 entrypoint for `context_cutter._lib`.

mod parser;
mod store;

use pyo3::exceptions::{PyKeyError, PyValueError};
use pyo3::prelude::*;
use sha2::{Digest, Sha256};
use serde_json::Value;

pub use store::ContextStore;

fn canonicalize_json_value(value: &Value) -> Value {
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

fn deterministic_handle_id(value: &Value) -> PyResult<String> {
    let canonical = canonicalize_json_value(value);
    let canonical_json = serde_json::to_string(&canonical)
        .map_err(|e| PyValueError::new_err(format!("failed to serialize payload: {e}")))?;
    let digest = Sha256::digest(canonical_json.as_bytes());
    let digest_hex = format!("{digest:x}");
    Ok(format!("hdl_{}", &digest_hex[..12]))
}

/// Stores a full JSON payload and returns a lightweight handle ID.
#[pyfunction]
fn store_response(json_str: &str) -> PyResult<String> {
    let value: Value = serde_json::from_str(json_str)
        .map_err(|e| PyValueError::new_err(format!("invalid JSON payload: {e}")))?;
    let handle_id = deterministic_handle_id(&value)?;
    store::global_store_insert(handle_id.clone(), value);
    Ok(handle_id)
}

/// Generates a teaser representation for a stored payload.
#[pyfunction]
fn generate_teaser(handle_id: &str) -> PyResult<String> {
    let value = store::global_store_get(handle_id)
        .ok_or_else(|| PyKeyError::new_err(format!("unknown handle_id: {handle_id}")))?;
    Ok(parser::generate_teaser_from_value(&value))
}

/// Runs a JSONPath query against a stored payload.
#[pyfunction]
fn query_path(handle_id: &str, json_path: &str) -> PyResult<String> {
    let value = store::global_store_get(handle_id)
        .ok_or_else(|| PyKeyError::new_err(format!("unknown handle_id: {handle_id}")))?;
    parser::query_json_path(&value, json_path)
}

/// Python module initialization for `context_cutter._lib`.
#[pymodule]
fn _lib(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<ContextStore>()?;
    m.add_function(wrap_pyfunction!(store_response, m)?)?;
    m.add_function(wrap_pyfunction!(generate_teaser, m)?)?;
    m.add_function(wrap_pyfunction!(query_path, m)?)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn deterministic_handle_stable_for_same_payload() {
        let value = json!({"b": 2, "a": 1, "nested": {"z": true, "x": [1, 2]}});
        let h1 = deterministic_handle_id(&value).expect("handle must be generated");
        let h2 = deterministic_handle_id(&value).expect("handle must be generated");
        assert_eq!(h1, h2);
        assert!(h1.starts_with("hdl_"));
    }

    #[test]
    fn deterministic_handle_ignores_object_key_order() {
        let left = json!({"a": 1, "b": {"x": 1, "y": 2}});
        let right = json!({"b": {"y": 2, "x": 1}, "a": 1});
        let h1 = deterministic_handle_id(&left).expect("handle must be generated");
        let h2 = deterministic_handle_id(&right).expect("handle must be generated");
        assert_eq!(h1, h2);
    }

    #[test]
    fn deterministic_handle_differs_for_different_payloads() {
        let h1 = deterministic_handle_id(&json!({"a": 1})).expect("handle must be generated");
        let h2 = deterministic_handle_id(&json!({"a": 2})).expect("handle must be generated");
        assert_ne!(h1, h2);
    }
}
