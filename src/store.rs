//! Storage primitives for cached JSON responses.
//!
//! This module provides:
//! - A process-wide bounded global store with TTL + LRU eviction.
//! - A `ContextStore` class exposed to Python for explicit store instances.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use dashmap::DashMap;
#[cfg(feature = "python")]
use pyo3::exceptions::PyValueError;
#[cfg(feature = "python")]
use pyo3::prelude::*;
use serde_json::Value;
use tracing::{debug, warn};

const DEFAULT_MAX_HANDLES: usize = 1_000;
const DEFAULT_TTL_SECS: u64 = 3_600;

#[derive(Clone, Debug)]
struct StoreEntry {
    value: Value,
    last_accessed: Instant,
}

#[derive(Clone, Debug)]
struct StoreConfig {
    max_handles: usize,
    ttl: Duration,
}

impl StoreConfig {
    fn from_env() -> Self {
        let max_handles = std::env::var("CONTEXT_CUTTER_MAX_HANDLES")
            .ok()
            .and_then(|s| s.parse::<usize>().ok())
            .filter(|v| *v > 0)
            .unwrap_or(DEFAULT_MAX_HANDLES);

        let ttl_secs = std::env::var("CONTEXT_CUTTER_TTL_SECS")
            .ok()
            .and_then(|s| s.parse::<u64>().ok())
            .filter(|v| *v > 0)
            .unwrap_or(DEFAULT_TTL_SECS);

        Self {
            max_handles,
            ttl: Duration::from_secs(ttl_secs),
        }
    }
}

struct GlobalStore {
    entries: DashMap<String, StoreEntry>,
    lru_order: Mutex<VecDeque<String>>,
    config: StoreConfig,
}

impl GlobalStore {
    fn new() -> Self {
        Self::new_with_config(StoreConfig::from_env())
    }

    fn new_with_config(config: StoreConfig) -> Self {
        Self {
            entries: DashMap::new(),
            lru_order: Mutex::new(VecDeque::new()),
            config,
        }
    }

    fn touch_lru(&self, handle_id: &str) {
        let mut order = self
            .lru_order
            .lock()
            .expect("global store LRU mutex must not be poisoned");
        if let Some(idx) = order.iter().position(|id| id == handle_id) {
            order.remove(idx);
        }
        order.push_back(handle_id.to_string());
    }

    fn is_expired(&self, entry: &StoreEntry, now: Instant) -> bool {
        now.duration_since(entry.last_accessed) > self.config.ttl
    }

    fn insert(&self, handle_id: String, value: Value) {
        let now = Instant::now();
        self.entries.insert(
            handle_id.clone(),
            StoreEntry {
                value,
                last_accessed: now,
            },
        );
        self.touch_lru(&handle_id);
        self.evict_if_needed();
    }

    fn get(&self, handle_id: &str) -> Option<Value> {
        let now = Instant::now();
        let mut entry = self.entries.get_mut(handle_id)?;
        if self.is_expired(&entry, now) {
            drop(entry);
            self.entries.remove(handle_id);
            debug!(handle_id = handle_id, "entry expired and removed on access");
            return None;
        }
        entry.last_accessed = now;
        let value = entry.value.clone();
        drop(entry);
        self.touch_lru(handle_id);
        Some(value)
    }

    fn evict_if_needed(&self) {
        while self.entries.len() > self.config.max_handles {
            let maybe_oldest = {
                let mut order = self
                    .lru_order
                    .lock()
                    .expect("global store LRU mutex must not be poisoned");
                order.pop_front()
            };

            if let Some(oldest) = maybe_oldest {
                if self.entries.remove(&oldest).is_some() {
                    debug!(handle_id = oldest.as_str(), "evicted LRU entry");
                }
            } else {
                break;
            }
        }
    }

    fn sweep_expired(&self) {
        let now = Instant::now();
        let expired: Vec<String> = self
            .entries
            .iter()
            .filter(|kv| self.is_expired(kv.value(), now))
            .map(|kv| kv.key().clone())
            .collect();

        if expired.is_empty() {
            return;
        }

        for handle_id in &expired {
            self.entries.remove(handle_id);
        }

        let mut order = self
            .lru_order
            .lock()
            .expect("global store LRU mutex must not be poisoned");
        order.retain(|id| !expired.contains(id));
        debug!(removed = expired.len(), "expired entries swept");
    }
}

static STORE: OnceLock<GlobalStore> = OnceLock::new();
static SWEEP_STARTED: AtomicBool = AtomicBool::new(false);

/// Returns the process-wide store, initializing it on first use.
fn global_store() -> &'static GlobalStore {
    STORE.get_or_init(GlobalStore::new)
}

/// Inserts JSON content by handle into the process-wide store.
pub fn global_store_insert(handle_id: String, value: Value) {
    global_store().insert(handle_id, value);
}

/// Fetches JSON content by handle from the process-wide store.
pub fn global_store_get(handle_id: &str) -> Option<Value> {
    global_store().get(handle_id)
}

/// Starts a periodic background sweep task for expired entries.
pub fn start_background_sweeper() {
    if SWEEP_STARTED.swap(true, Ordering::SeqCst) {
        return;
    }

    let sweep_interval = Duration::from_secs(60);
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(sweep_interval);
        loop {
            ticker.tick().await;
            global_store().sweep_expired();
        }
    });

    let cfg = &global_store().config;
    warn!(
        max_handles = cfg.max_handles,
        ttl_secs = cfg.ttl.as_secs(),
        "global store initialized with bounded memory settings"
    );
}

/// Python-facing explicit store wrapper.
#[cfg(feature = "python")]
#[pyclass]
pub struct ContextStore {
    inner: DashMap<String, Value>,
}

#[cfg(feature = "python")]
impl Default for ContextStore {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(feature = "python")]
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
    fn local_store_evicts_oldest_when_capacity_exceeded() {
        let store = GlobalStore::new_with_config(StoreConfig {
            max_handles: 2,
            ttl: Duration::from_secs(60),
        });

        store.insert("h1".to_string(), json!({"n": 1}));
        store.insert("h2".to_string(), json!({"n": 2}));
        store.insert("h3".to_string(), json!({"n": 3}));

        assert!(store.get("h1").is_none(), "oldest entry should be evicted");
        assert_eq!(store.get("h2"), Some(json!({"n": 2})));
        assert_eq!(store.get("h3"), Some(json!({"n": 3})));
    }

    #[test]
    fn local_store_get_returns_none_for_missing_key() {
        let store = GlobalStore::new_with_config(StoreConfig {
            max_handles: 10,
            ttl: Duration::from_secs(60),
        });
        assert!(store.get("nonexistent").is_none());
    }

    #[test]
    fn local_store_expires_entries_on_get() {
        let store = GlobalStore::new_with_config(StoreConfig {
            max_handles: 10,
            ttl: Duration::from_millis(1),
        });
        store.insert("h_expire".to_string(), json!({"x": 99}));
        std::thread::sleep(Duration::from_millis(5));
        assert!(
            store.get("h_expire").is_none(),
            "entry should be expired and removed on get"
        );
    }

    #[test]
    fn local_store_sweep_expired_removes_old_entries() {
        let store = GlobalStore::new_with_config(StoreConfig {
            max_handles: 10,
            ttl: Duration::from_millis(1),
        });
        store.insert("h_sweep".to_string(), json!({"y": 1}));
        std::thread::sleep(Duration::from_millis(5));
        store.sweep_expired();
        assert!(store.get("h_sweep").is_none());
    }

    #[cfg(feature = "python")]
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

    #[cfg(feature = "python")]
    #[test]
    fn context_store_clear_empties_entries() {
        let store = ContextStore::new();
        store
            .insert("a", r#"{"a":1}"#)
            .expect("insert should succeed");
        store
            .insert("b", r#"{"b":2}"#)
            .expect("insert should succeed");
        assert_eq!(store.len(), 2);
        store.clear();
        assert_eq!(store.len(), 0);
    }
}
