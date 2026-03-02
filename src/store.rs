//! Storage primitives for cached JSON responses.
//!
//! This module provides:
//! - A process-wide global store used by module-level PyO3 functions.
//! - A `ContextStore` class exposed to Python for explicit store instances.

use std::sync::OnceLock;

use dashmap::DashMap;
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use serde_json::Value;

static STORE: OnceLock<DashMap<String, Value>> = OnceLock::new();

/// Returns the process-wide store, initializing it on first use.
pub fn global_store() -> &'static DashMap<String, Value> {
    STORE.get_or_init(DashMap::new)
}

/// Inserts JSON content by handle into the process-wide store.
pub fn global_store_insert(handle_id: String, value: Value) {
    global_store().insert(handle_id, value);
}

/// Fetches JSON content by handle from the process-wide store.
pub fn global_store_get(handle_id: &str) -> Option<Value> {
    global_store().get(handle_id).map(|entry| entry.clone())
}

/// Python-facing explicit store wrapper.
#[pyclass]
pub struct ContextStore {
    inner: DashMap<String, Value>,
}

#[pymethods]
impl ContextStore {
    /// Creates a new empty in-memory context store.
    #[new]
    pub fn new() -> Self {
        Self {
            inner: DashMap::new(),
        }
    }

    /// Parses `json_str` and inserts it under `handle_id`.
    pub fn insert(&self, handle_id: &str, json_str: &str) -> PyResult<()> {
        let value: Value = serde_json::from_str(json_str)
            .map_err(|e| PyValueError::new_err(format!("invalid JSON payload: {e}")))?;
        self.inner.insert(handle_id.to_string(), value);
        Ok(())
    }

    /// Returns the serialized JSON string for `handle_id` or `None`.
    pub fn get(&self, handle_id: &str) -> PyResult<Option<String>> {
        let value = self.inner.get(handle_id).map(|entry| entry.clone());
        match value {
            Some(v) => serde_json::to_string(&v)
                .map(Some)
                .map_err(|e| PyValueError::new_err(format!("failed to serialize JSON: {e}"))),
            None => Ok(None),
        }
    }

    /// Returns the number of stored entries.
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    /// Clears all entries in the store.
    pub fn clear(&self) {
        self.inner.clear();
    }

    /// Removes a handle, returning true when an entry existed.
    pub fn remove(&self, handle_id: &str) -> bool {
        self.inner.remove(handle_id).is_some()
    }
}
