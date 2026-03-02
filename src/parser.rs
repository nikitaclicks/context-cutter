//! Parsing and query stubs.
//!
//! Real teaser schema extraction and JSONPath selection will be implemented later.

use pyo3::prelude::*;
use serde_json::Value;

/// Builds a lightweight teaser string for a stored JSON value.
pub fn generate_teaser_from_value(_value: &Value) -> String {
    r#"{"_teaser":true,"_type":"object","_keys":["stub"]}"#.to_string()
}

/// Executes a JSONPath query against a stored value (stub implementation).
pub fn query_json_path(_value: &Value, _json_path: &str) -> PyResult<String> {
    // TODO: Use `jsonpath_rust::JsonPath` to evaluate `json_path` on `_value`.
    Ok(r#"["stub_result"]"#.to_string())
}
