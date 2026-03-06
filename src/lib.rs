//! ContextCutter library.
//!
//! The `engine` module contains all pure-Rust logic (store, teaser, JSONPath query).
//! The PyO3 bindings below are compiled only when the `python` feature is enabled —
//! Maturin passes `--features python` automatically via `pyproject.toml`.

mod parser;
pub mod store;
pub mod error;
pub mod engine;

// ─── PyO3 bindings ────────────────────────────────────────────────────────────
#[cfg(feature = "python")]
mod python_bindings {
    use pyo3::exceptions::{PyKeyError, PyValueError};
    use pyo3::prelude::*;

    use crate::error::ContextCutterError;
    pub use crate::store::ContextStore;

    fn map_engine_error(err: ContextCutterError) -> PyErr {
        match err {
            ContextCutterError::UnknownHandle(_) => PyKeyError::new_err(err.to_string()),
            _ => PyValueError::new_err(err.to_string()),
        }
    }

    /// Stores a full JSON payload and returns a lightweight handle ID.
    #[pyfunction]
    pub fn store_response(json_str: &str) -> PyResult<String> {
        crate::engine::engine_store(json_str).map_err(map_engine_error)
    }

    /// Generates a teaser representation for a stored payload.
    #[pyfunction]
    pub fn generate_teaser(handle_id: &str) -> PyResult<String> {
        crate::engine::engine_teaser(handle_id).map_err(map_engine_error)
    }

    /// Runs a JSONPath query against a stored payload.
    #[pyfunction]
    pub fn query_path(handle_id: &str, json_path: &str) -> PyResult<String> {
        crate::engine::engine_query(handle_id, json_path).map_err(map_engine_error)
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
