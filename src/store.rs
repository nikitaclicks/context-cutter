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

impl Default for ContextStore {
    fn default() -> Self {
        Self::new()
    }
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

    /// Returns true when the store contains no entries.
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn global_store_insert_and_get_round_trip() {
        global_store_insert("h_test".to_string(), json!({"x": 1}));
        let got = global_store_get("h_test");
        assert_eq!(got, Some(json!({"x": 1})));
    }

    #[test]
    fn context_store_basic_operations() {
        let store = ContextStore::new();
        assert_eq!(store.len(), 0);
        assert!(store.is_empty());

        store
            .insert("h1", r#"{"value": 42}"#)
            .expect("insert should succeed");
        assert_eq!(store.len(), 1);
        assert!(!store.is_empty());

        let payload = store
            .get("h1")
            .expect("get should not fail")
            .expect("value should exist");
        assert_eq!(payload, r#"{"value":42}"#);

        assert!(store.remove("h1"));
        assert_eq!(store.len(), 0);
        assert!(!store.remove("h1"));
    }

    #[test]
    fn context_store_clear_empties_entries() {
        let store = ContextStore::new();
        store.insert("a", r#"{"a":1}"#).expect("insert should succeed");
        store.insert("b", r#"{"b":2}"#).expect("insert should succeed");
        assert_eq!(store.len(), 2);
        store.clear();
        assert_eq!(store.len(), 0);
    }
}
