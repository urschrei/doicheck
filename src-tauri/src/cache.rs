//! A small DOI->JSON cache abstraction so the pipeline can be cache-aware
//! without depending directly on the SQLite store (and tested with an
//! in-memory implementation).

use crate::store::Store;
use std::collections::HashMap;
use std::sync::Mutex;

pub trait DoiCache {
    /// Cached Crossref JSON for a DOI, if present.
    fn get(&self, doi: &str) -> Option<String>;
    /// Store Crossref JSON for a DOI. Errors are swallowed (best-effort cache).
    fn put(&self, doi: &str, json: &str);
}

/// In-memory cache for tests.
#[derive(Default)]
pub struct MemoryCache {
    map: Mutex<HashMap<String, String>>,
}

impl DoiCache for MemoryCache {
    fn get(&self, doi: &str) -> Option<String> {
        self.map.lock().ok()?.get(doi).cloned()
    }
    fn put(&self, doi: &str, json: &str) {
        if let Ok(mut m) = self.map.lock() {
            m.insert(doi.to_string(), json.to_string());
        }
    }
}

/// Cache backed by the SQLite store (locked per access; never held across an
/// await point). Holds a reference to the shared `Mutex<Store>`.
pub struct StoreCache<'a> {
    pub store: &'a Mutex<Store>,
}

impl DoiCache for StoreCache<'_> {
    fn get(&self, doi: &str) -> Option<String> {
        let store = match self.store.lock() {
            Ok(store) => store,
            Err(e) => {
                log::warn!("crossref cache: store lock poisoned on get: {e}");
                return None;
            }
        };
        match store.cache_get(doi) {
            Ok(hit) => hit,
            Err(e) => {
                log::warn!("crossref cache: read failed for {doi}: {e}");
                None
            }
        }
    }
    fn put(&self, doi: &str, json: &str) {
        let store = match self.store.lock() {
            Ok(store) => store,
            Err(e) => {
                log::warn!("crossref cache: store lock poisoned on put: {e}");
                return;
            }
        };
        if let Err(e) = store.cache_put(doi, json) {
            log::warn!("crossref cache: write failed for {doi}: {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_cache_round_trips() {
        let c = MemoryCache::default();
        assert_eq!(c.get("10.1/x"), None);
        c.put("10.1/x", "{}");
        assert_eq!(c.get("10.1/x").as_deref(), Some("{}"));
    }
}
