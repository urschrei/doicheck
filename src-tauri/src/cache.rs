//! Crossref caching abstractions so the pipeline can be cache-aware without
//! depending directly on the SQLite store (and tested with in-memory
//! implementations): a DOI->JSON cache, plus a bibliographic-search cache keyed
//! by a hash of the reference text.

use crate::doi::Doi;
use crate::model::Source;
use crate::store::Store;
use std::collections::HashMap;
use std::sync::Mutex;

pub trait DoiCache {
    /// Cached JSON for a DOI from `source` (Crossref/DataCite), if present.
    fn get(&self, source: Source, doi: &Doi) -> Option<String>;
    /// Store JSON for a DOI under `source`. Errors are swallowed (best-effort).
    fn put(&self, source: Source, doi: &Doi, json: &str);
}

/// A bibliographic-search cache key: a hash of the normalised reference text.
/// The single constructor guarantees the key is derived consistently, just as
/// [`Doi`] guarantees a normalised DOI.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct QueryKey(String);

impl QueryKey {
    /// Derive a key from a reference's raw text.
    pub fn new(reference: &str) -> Self {
        QueryKey(crate::ingest::fingerprint(
            crate::text::normalise(reference).as_bytes(),
        ))
    }

    /// The key as a string.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Cache for bibliographic-search results, keyed by the reference text (a
/// [`QueryKey`]) rather than a DOI.
pub trait SearchCache {
    /// Cached search-result JSON for a query key, if present.
    fn search_get(&self, key: &QueryKey) -> Option<String>;
    /// Store search-result JSON for a query key. Errors are swallowed.
    fn search_put(&self, key: &QueryKey, json: &str);
}

/// In-memory cache for tests.
#[derive(Default)]
pub struct MemoryCache {
    crossref: Mutex<HashMap<String, String>>,
    datacite: Mutex<HashMap<String, String>>,
    search: Mutex<HashMap<String, String>>,
}

impl MemoryCache {
    fn doi_map(&self, source: Source) -> &Mutex<HashMap<String, String>> {
        match source {
            Source::Crossref => &self.crossref,
            Source::DataCite => &self.datacite,
        }
    }
}

impl DoiCache for MemoryCache {
    fn get(&self, source: Source, doi: &Doi) -> Option<String> {
        self.doi_map(source).lock().ok()?.get(doi.as_str()).cloned()
    }
    fn put(&self, source: Source, doi: &Doi, json: &str) {
        if let Ok(mut m) = self.doi_map(source).lock() {
            m.insert(doi.as_str().to_string(), json.to_string());
        }
    }
}

impl SearchCache for MemoryCache {
    fn search_get(&self, key: &QueryKey) -> Option<String> {
        self.search.lock().ok()?.get(key.as_str()).cloned()
    }
    fn search_put(&self, key: &QueryKey, json: &str) {
        if let Ok(mut m) = self.search.lock() {
            m.insert(key.as_str().to_string(), json.to_string());
        }
    }
}

/// Cache backed by the SQLite store (locked per access; never held across an
/// await point). Holds a reference to the shared `Mutex<Store>`.
pub struct StoreCache<'a> {
    pub store: &'a Mutex<Store>,
}

impl DoiCache for StoreCache<'_> {
    fn get(&self, source: Source, doi: &Doi) -> Option<String> {
        let store = match self.store.lock() {
            Ok(store) => store,
            Err(e) => {
                log::warn!("doi cache: store lock poisoned on get: {e}");
                return None;
            }
        };
        let result = match source {
            Source::Crossref => store.cache_get(doi.as_str()),
            Source::DataCite => store.datacite_cache_get(doi.as_str()),
        };
        match result {
            Ok(hit) => hit,
            Err(e) => {
                log::warn!("doi cache: read failed for {}: {e}", doi.as_str());
                None
            }
        }
    }
    fn put(&self, source: Source, doi: &Doi, json: &str) {
        let store = match self.store.lock() {
            Ok(store) => store,
            Err(e) => {
                log::warn!("doi cache: store lock poisoned on put: {e}");
                return;
            }
        };
        let result = match source {
            Source::Crossref => store.cache_put(doi.as_str(), json),
            Source::DataCite => store.datacite_cache_put(doi.as_str(), json),
        };
        if let Err(e) = result {
            log::warn!("doi cache: write failed for {}: {e}", doi.as_str());
        }
    }
}

impl SearchCache for StoreCache<'_> {
    fn search_get(&self, key: &QueryKey) -> Option<String> {
        let store = match self.store.lock() {
            Ok(store) => store,
            Err(e) => {
                log::warn!("crossref search cache: store lock poisoned on get: {e}");
                return None;
            }
        };
        match store.search_cache_get(key.as_str()) {
            Ok(hit) => hit,
            Err(e) => {
                log::warn!("crossref search cache: read failed: {e}");
                None
            }
        }
    }
    fn search_put(&self, key: &QueryKey, json: &str) {
        let store = match self.store.lock() {
            Ok(store) => store,
            Err(e) => {
                log::warn!("crossref search cache: store lock poisoned on put: {e}");
                return;
            }
        };
        if let Err(e) = store.search_cache_put(key.as_str(), json) {
            log::warn!("crossref search cache: write failed: {e}");
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
        assert_eq!(c.get(Source::Crossref, &doi), None);
        c.put(Source::Crossref, &doi, "{}");
        assert_eq!(c.get(Source::Crossref, &doi).as_deref(), Some("{}"));
        // The same DOI under a different agency is a separate slot.
        assert_eq!(c.get(Source::DataCite, &doi), None);
    }

    #[test]
    fn query_key_is_stable_and_normalises() {
        // Case and whitespace differences normalise to the same key.
        assert_eq!(
            QueryKey::new("Smith, J. (2020). A Study.").as_str(),
            QueryKey::new("smith,  j.   (2020).  a study.").as_str()
        );
        assert_ne!(
            QueryKey::new("Smith 2020").as_str(),
            QueryKey::new("Jones 2021").as_str()
        );
    }

    #[test]
    fn memory_search_cache_round_trips() {
        let c = MemoryCache::default();
        let key = QueryKey::new("Some reference text");
        assert_eq!(c.search_get(&key), None);
        c.search_put(&key, "{\"doi\":\"10.1/x\"}");
        assert_eq!(c.search_get(&key).as_deref(), Some("{\"doi\":\"10.1/x\"}"));
    }
}
