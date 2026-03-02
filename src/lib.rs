//! PyO3 entrypoint for `context_cutter._lib`.

mod parser;
mod store;

use pyo3::exceptions::{PyKeyError, PyValueError};
use pyo3::prelude::*;
use serde_json::Value;
use uuid::Uuid;

pub use store::ContextStore;

/// Stores a full JSON payload and returns a lightweight handle ID.
#[pyfunction]
fn store_response(json_str: &str) -> PyResult<String> {
    let value: Value = serde_json::from_str(json_str)
        .map_err(|e| PyValueError::new_err(format!("invalid JSON payload: {e}")))?;
    let handle_id = Uuid::new_v4().to_string();
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
