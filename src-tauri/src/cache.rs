//! A small DOI->JSON cache abstraction so the pipeline can be cache-aware
//! without depending directly on the SQLite store (and tested with an
//! in-memory implementation).

use crate::doi::Doi;
use crate::store::Store;
use std::collections::HashMap;
use std::sync::Mutex;

pub trait DoiCache {
    /// Cached Crossref JSON for a DOI, if present.
    fn get(&self, doi: &Doi) -> Option<String>;
    /// Store Crossref JSON for a DOI. Errors are swallowed (best-effort cache).
    fn put(&self, doi: &Doi, json: &str);
}

/// In-memory cache for tests.
#[derive(Default)]
pub struct MemoryCache {
    map: Mutex<HashMap<String, String>>,
}

impl DoiCache for MemoryCache {
    fn get(&self, doi: &Doi) -> Option<String> {
        self.map.lock().ok()?.get(doi.as_str()).cloned()
    }
    fn put(&self, doi: &Doi, json: &str) {
        if let Ok(mut m) = self.map.lock() {
            m.insert(doi.as_str().to_string(), json.to_string());
        }
    }
}

/// Cache backed by the SQLite store (locked per access; never held across an
/// await point). Holds a reference to the shared `Mutex<Store>`.
pub struct StoreCache<'a> {
    pub store: &'a Mutex<Store>,
}

impl DoiCache for StoreCache<'_> {
    fn get(&self, doi: &Doi) -> Option<String> {
        let store = match self.store.lock() {
            Ok(store) => store,
            Err(e) => {
                log::warn!("crossref cache: store lock poisoned on get: {e}");
                return None;
            }
        };
        match store.cache_get(doi.as_str()) {
            Ok(hit) => hit,
            Err(e) => {
                log::warn!("crossref cache: read failed for {}: {e}", doi.as_str());
                None
            }
        }
    }
    fn put(&self, doi: &Doi, json: &str) {
        let store = match self.store.lock() {
            Ok(store) => store,
            Err(e) => {
                log::warn!("crossref cache: store lock poisoned on put: {e}");
                return;
            }
        };
        if let Err(e) = store.cache_put(doi.as_str(), json) {
            log::warn!("crossref cache: write failed for {}: {e}", doi.as_str());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_cache_round_trips() {
        let c = MemoryCache::default();
        let doi = Doi::new("10.1/x");
        assert_eq!(c.get(&doi), None);
        c.put(&doi, "{}");
        assert_eq!(c.get(&doi).as_deref(), Some("{}"));
    }
}
