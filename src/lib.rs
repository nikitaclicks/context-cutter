//! ContextCutter library.
//!
//! The `engine` module contains all pure-Rust logic (store, teaser, JSONPath query).
//! The PyO3 bindings below are compiled only when the `python` feature is enabled —
//! Maturin passes `--features python` automatically via `pyproject.toml`.

mod parser;
mod store;
pub mod engine;

// ─── PyO3 bindings ────────────────────────────────────────────────────────────
#[cfg(feature = "python")]
mod python_bindings {
    use pyo3::exceptions::{PyKeyError, PyValueError};
    use pyo3::prelude::*;

    pub use crate::store::ContextStore;

    /// Stores a full JSON payload and returns a lightweight handle ID.
    #[pyfunction]
    pub fn store_response(json_str: &str) -> PyResult<String> {
        crate::engine::engine_store(json_str).map_err(PyValueError::new_err)
    }

    /// Generates a teaser representation for a stored payload.
    #[pyfunction]
    pub fn generate_teaser(handle_id: &str) -> PyResult<String> {
        crate::engine::engine_teaser(handle_id).map_err(PyKeyError::new_err)
    }

    /// Runs a JSONPath query against a stored payload.
    #[pyfunction]
    pub fn query_path(handle_id: &str, json_path: &str) -> PyResult<String> {
        crate::engine::engine_query(handle_id, json_path).map_err(|e| {
            if e.starts_with("unknown handle_id") {
                PyKeyError::new_err(e)
            } else {
                PyValueError::new_err(e)
            }
        })
    }

    /// Python module initializer for `context_cutter._lib`.
    #[pymodule]
    pub fn _lib(m: &Bound<'_, PyModule>) -> PyResult<()> {
        m.add_class::<ContextStore>()?;
        m.add_function(wrap_pyfunction!(store_response, m)?)?;
        m.add_function(wrap_pyfunction!(generate_teaser, m)?)?;
        m.add_function(wrap_pyfunction!(query_path, m)?)?;
        Ok(())
    }
}

#[cfg(feature = "python")]
pub use python_bindings::*;
