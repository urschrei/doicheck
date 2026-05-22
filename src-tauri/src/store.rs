//! SQLite persistence for documents, checks, entries, discrepancies, settings.

use crate::model::CheckResult;
use rusqlite::{Connection, params};

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error(transparent)]
    Sqlite(#[from] rusqlite::Error),
    #[error("failed to serialise check result: {0}")]
    Serde(#[from] serde_json::Error),
}

/// Cached Crossref records older than this are treated as a miss and re-fetched,
/// so a later retraction or correction is eventually picked up rather than
/// masked by a permanently cached record.
const CACHE_TTL_DAYS: i64 = 30;

/// Whether a cache row's RFC 3339 `fetched_at` is within the TTL. A stale or
/// unparseable timestamp counts as not fresh, so the row is treated as a miss.
fn fresh(fetched_at: &str) -> bool {
    chrono::DateTime::parse_from_rfc3339(fetched_at).is_ok_and(|t| {
        chrono::Utc::now().signed_duration_since(t.with_timezone(&chrono::Utc))
            < chrono::Duration::days(CACHE_TTL_DAYS)
    })
}

pub struct Store {
    conn: Connection,
}

/// Sidebar status for a document. The serialised (kebab-case) form is the wire
/// contract consumed by the frontend.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum DocumentStatus {
    Incomplete,
    Failed,
    HasIssues,
    Clean,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct DocumentSummary {
    pub fingerprint: String,
    pub filename: String,
    pub last_checked: String,
    pub status: DocumentStatus,
}

impl Store {
    pub fn open(path: &std::path::Path) -> Result<Self, StoreError> {
        let conn = Connection::open(path)?;
        let mut store = Self { conn };
        store.migrate()?;
        Ok(store)
    }

    #[cfg(test)]
    pub fn open_in_memory() -> Result<Self, StoreError> {
        let conn = Connection::open_in_memory()?;
        let mut store = Self { conn };
        store.migrate()?;
        Ok(store)
    }

    fn migrate(&mut self) -> Result<(), StoreError> {
        self.conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS documents (
                id INTEGER PRIMARY KEY,
                fingerprint TEXT NOT NULL UNIQUE,
                filename TEXT NOT NULL,
                kind TEXT NOT NULL,
                first_seen TEXT NOT NULL,
                last_checked TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS checks (
                id INTEGER PRIMARY KEY,
                document_id INTEGER NOT NULL REFERENCES documents(id),
                run_at TEXT NOT NULL,
                total INTEGER NOT NULL,
                checkable INTEGER NOT NULL,
                resolved INTEGER NOT NULL,
                unresolved INTEGER NOT NULL,
                with_discrepancies INTEGER NOT NULL,
                missing_doi_flagged INTEGER NOT NULL,
                network_failed INTEGER NOT NULL DEFAULT 0,
                report_text TEXT NOT NULL,
                result_json TEXT NOT NULL DEFAULT ''
            );
            -- The per-entry `entries`/`discrepancies` tables were write-only
            -- (nothing read them back; result_json is the source of truth), so
            -- drop them on databases created before this change.
            DROP TABLE IF EXISTS discrepancies;
            DROP TABLE IF EXISTS entries;
            CREATE TABLE IF NOT EXISTS settings (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS crossref_cache (
                doi TEXT PRIMARY KEY,
                json TEXT NOT NULL,
                fetched_at TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS crossref_search_cache (
                query TEXT PRIMARY KEY,
                json TEXT NOT NULL,
                fetched_at TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS dismissals (
                fingerprint TEXT NOT NULL,
                doi TEXT NOT NULL,
                field TEXT NOT NULL,
                PRIMARY KEY (fingerprint, doi, field)
            );
            "#,
        )?;
        // Backfill columns for databases created before they were added to the
        // CREATE TABLE above; on a fresh DB these checks find the column and the
        // ALTERs are skipped.
        let has_result_json: bool = self.conn.query_row(
            "SELECT COUNT(*) FROM pragma_table_info('checks') WHERE name = 'result_json'",
            [],
            |r| r.get::<_, i64>(0),
        )? > 0;
        if !has_result_json {
            self.conn.execute(
                "ALTER TABLE checks ADD COLUMN result_json TEXT NOT NULL DEFAULT ''",
                [],
            )?;
        }
        let has_network_failed: bool = self.conn.query_row(
            "SELECT COUNT(*) FROM pragma_table_info('checks') WHERE name = 'network_failed'",
            [],
            |r| r.get::<_, i64>(0),
        )? > 0;
        if !has_network_failed {
            self.conn.execute(
                "ALTER TABLE checks ADD COLUMN network_failed INTEGER NOT NULL DEFAULT 0",
                [],
            )?;
        }
        Ok(())
    }

    pub fn get_setting(&self, key: &str) -> Result<Option<String>, StoreError> {
        let mut stmt = self
            .conn
            .prepare("SELECT value FROM settings WHERE key = ?1")?;
        let mut rows = stmt.query(params![key])?;
        if let Some(row) = rows.next()? {
            Ok(Some(row.get(0)?))
        } else {
            Ok(None)
        }
    }

    pub fn set_setting(&self, key: &str, value: &str) -> Result<(), StoreError> {
        self.conn.execute(
            "INSERT INTO settings(key, value) VALUES(?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )?;
        Ok(())
    }

    /// Number of concurrent Crossref requests. Defaults to 5 if absent or
    /// invalid; clamped to 1..=20. A DB read error is intentionally treated as
    /// "absent" so a transient settings-read failure falls back to the default.
    pub fn concurrency(&self) -> usize {
        self.get_setting("concurrency")
            .ok()
            .flatten()
            .and_then(|s| s.parse::<usize>().ok())
            .map(|n| n.clamp(1, 20))
            .unwrap_or(5)
    }

    pub fn set_concurrency(&self, value: usize) -> Result<(), StoreError> {
        self.set_setting("concurrency", &value.to_string())
    }

    /// The cached Crossref JSON for a DOI, if present and not older than the TTL.
    /// A stale or unparseable `fetched_at` is treated as a miss so the record is
    /// re-fetched.
    pub fn cache_get(&self, doi: &str) -> Result<Option<String>, StoreError> {
        let mut stmt = self
            .conn
            .prepare("SELECT json, fetched_at FROM crossref_cache WHERE doi = ?1")?;
        let mut rows = stmt.query(params![doi])?;
        let Some(row) = rows.next()? else {
            return Ok(None);
        };
        let json: String = row.get(0)?;
        let fetched_at: String = row.get(1)?;
        Ok(fresh(&fetched_at).then_some(json))
    }

    /// Store (or replace) the Crossref JSON for a DOI.
    pub fn cache_put(&self, doi: &str, json: &str) -> Result<(), StoreError> {
        let now = chrono::Utc::now().to_rfc3339();
        self.conn.execute(
            "INSERT INTO crossref_cache(doi, json, fetched_at) VALUES(?1, ?2, ?3)
             ON CONFLICT(doi) DO UPDATE SET json = excluded.json, fetched_at = excluded.fetched_at",
            params![doi, json, now],
        )?;
        Ok(())
    }

    /// Cached bibliographic-search result for a query key, if present and within
    /// the TTL. Keyed by a hash of the normalised reference text (see
    /// `cache::QueryKey`), separate from the DOI cache.
    pub fn search_cache_get(&self, query: &str) -> Result<Option<String>, StoreError> {
        let mut stmt = self
            .conn
            .prepare("SELECT json, fetched_at FROM crossref_search_cache WHERE query = ?1")?;
        let mut rows = stmt.query(params![query])?;
        let Some(row) = rows.next()? else {
            return Ok(None);
        };
        let json: String = row.get(0)?;
        let fetched_at: String = row.get(1)?;
        Ok(fresh(&fetched_at).then_some(json))
    }

    /// Store (or replace) a bibliographic-search result for a query key.
    pub fn search_cache_put(&self, query: &str, json: &str) -> Result<(), StoreError> {
        let now = chrono::Utc::now().to_rfc3339();
        self.conn.execute(
            "INSERT INTO crossref_search_cache(query, json, fetched_at) VALUES(?1, ?2, ?3)
             ON CONFLICT(query) DO UPDATE SET json = excluded.json, fetched_at = excluded.fetched_at",
            params![query, json, now],
        )?;
        Ok(())
    }

    /// Persist a check and its document. `kind` is the file kind as a short
    /// string ("pdf"/"docx"). `report_text` is the rendered report. The full
    /// structured result is stored as JSON in `result_json`. Returns the new
    /// check id.
    pub fn save_check(
        &mut self,
        result: &CheckResult,
        kind: &str,
        report_text: &str,
    ) -> Result<i64, StoreError> {
        let counts = result.counts();
        // Serialise before opening the transaction so a failure aborts the write
        // rather than committing a row with an empty, unreadable result_json.
        let result_json = serde_json::to_string(result)?;
        let tx = self.conn.transaction()?;
        tx.execute(
            "INSERT INTO documents(fingerprint, filename, kind, first_seen, last_checked)
             VALUES(?1, ?2, ?3, ?4, ?4)
             ON CONFLICT(fingerprint) DO UPDATE SET last_checked = excluded.last_checked,
                 filename = excluded.filename",
            params![result.fingerprint, result.filename, kind, result.run_at],
        )?;
        let document_id: i64 = tx.query_row(
            "SELECT id FROM documents WHERE fingerprint = ?1",
            params![result.fingerprint],
            |r| r.get(0),
        )?;
        tx.execute(
            "INSERT INTO checks(document_id, run_at, total, checkable, resolved,
                 unresolved, with_discrepancies, missing_doi_flagged, network_failed,
                 report_text, result_json)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",
            params![
                document_id,
                result.run_at,
                counts.total as i64,
                counts.checkable as i64,
                counts.resolved as i64,
                counts.unresolved as i64,
                counts.with_discrepancies as i64,
                counts.missing_doi_flagged as i64,
                counts.network_failed as i64,
                report_text,
                result_json
            ],
        )?;
        let check_id = tx.last_insert_rowid();
        tx.commit()?;
        Ok(check_id)
    }

    pub fn add_dismissal(
        &self,
        fingerprint: &str,
        doi: &str,
        field: &str,
    ) -> Result<(), StoreError> {
        self.conn.execute(
            "INSERT OR IGNORE INTO dismissals(fingerprint, doi, field) VALUES(?1, ?2, ?3)",
            params![fingerprint, doi, field],
        )?;
        Ok(())
    }

    pub fn remove_dismissal(
        &self,
        fingerprint: &str,
        doi: &str,
        field: &str,
    ) -> Result<(), StoreError> {
        self.conn.execute(
            "DELETE FROM dismissals WHERE fingerprint = ?1 AND doi = ?2 AND field = ?3",
            params![fingerprint, doi, field],
        )?;
        Ok(())
    }

    pub fn dismissals_for(
        &self,
        fingerprint: &str,
    ) -> Result<std::collections::HashSet<(String, String)>, StoreError> {
        let mut stmt = self
            .conn
            .prepare("SELECT doi, field FROM dismissals WHERE fingerprint = ?1")?;
        let rows = stmt.query_map(params![fingerprint], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })?;
        rows.collect::<Result<std::collections::HashSet<_>, _>>()
            .map_err(StoreError::from)
    }

    /// The most recent report text for a document, by fingerprint.
    pub fn latest_report(&self, fingerprint: &str) -> Result<Option<String>, StoreError> {
        let mut stmt = self.conn.prepare(
            "SELECT c.report_text FROM checks c
             JOIN documents d ON d.id = c.document_id
             WHERE d.fingerprint = ?1
             ORDER BY c.id DESC LIMIT 1",
        )?;
        let mut rows = stmt.query(params![fingerprint])?;
        if let Some(row) = rows.next()? {
            Ok(Some(row.get(0)?))
        } else {
            Ok(None)
        }
    }

    /// The most recent structured result for a document, by fingerprint.
    /// Dismissals are applied before returning, so callers always see annotated results.
    pub fn latest_result(
        &self,
        fingerprint: &str,
    ) -> Result<Option<crate::model::CheckResult>, StoreError> {
        let mut stmt = self.conn.prepare(
            "SELECT c.result_json FROM checks c
             JOIN documents d ON d.id = c.document_id
             WHERE d.fingerprint = ?1
             ORDER BY c.id DESC LIMIT 1",
        )?;
        let mut rows = stmt.query(params![fingerprint])?;
        match rows.next()? {
            Some(row) => {
                let json: String = row.get(0)?;
                match serde_json::from_str::<crate::model::CheckResult>(&json) {
                    Ok(mut result) => {
                        let set = self.dismissals_for(fingerprint)?;
                        result.apply_dismissals(&set);
                        Ok(Some(result))
                    }
                    // An old row written by a previous model version may no longer
                    // parse. Log it and report "no result" so the document can be
                    // re-checked, rather than failing the whole call.
                    Err(e) => {
                        log::warn!(
                            "store: stored result for {fingerprint} did not parse ({e}); treating as no result"
                        );
                        Ok(None)
                    }
                }
            }
            None => Ok(None),
        }
    }

    /// The stored file kind ("pdf"/"docx") for a document, by fingerprint.
    pub fn kind_for(&self, fingerprint: &str) -> Result<Option<String>, StoreError> {
        let mut stmt = self
            .conn
            .prepare("SELECT kind FROM documents WHERE fingerprint = ?1")?;
        let mut rows = stmt.query(params![fingerprint])?;
        if let Some(row) = rows.next()? {
            Ok(Some(row.get(0)?))
        } else {
            Ok(None)
        }
    }

    /// Delete a document and all its checks. The shared DOI cache
    /// (`crossref_cache`) is deliberately left intact.
    pub fn delete_document(&mut self, fingerprint: &str) -> Result<(), StoreError> {
        use rusqlite::OptionalExtension;
        let tx = self.conn.transaction()?;
        let doc_id: Option<i64> = tx
            .query_row(
                "SELECT id FROM documents WHERE fingerprint = ?1",
                params![fingerprint],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(doc_id) = doc_id {
            tx.execute("DELETE FROM checks WHERE document_id = ?1", params![doc_id])?;
            tx.execute("DELETE FROM documents WHERE id = ?1", params![doc_id])?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Sidebar list: one row per document with its latest status.
    /// Status is derived from the dismissal-annotated latest result so that
    /// dismissing all mismatches on a document turns it green.
    pub fn list_documents(&self) -> Result<Vec<DocumentSummary>, StoreError> {
        let mut stmt = self.conn.prepare(
            "SELECT fingerprint, filename, last_checked FROM documents ORDER BY last_checked DESC",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        })?;
        let docs: Vec<(String, String, String)> = rows.collect::<Result<Vec<_>, _>>()?;
        let mut out = Vec::with_capacity(docs.len());
        for (fingerprint, filename, last_checked) in docs {
            let status = match self.latest_result(&fingerprint)? {
                Some(result) => {
                    let c = result.counts();
                    let not_found = c.unresolved.saturating_sub(c.network_failed);
                    if c.network_failed > 0 {
                        DocumentStatus::Incomplete
                    } else if not_found > 0 {
                        DocumentStatus::Failed
                    } else if c.with_discrepancies > 0 {
                        DocumentStatus::HasIssues
                    } else {
                        DocumentStatus::Clean
                    }
                }
                None => DocumentStatus::Clean,
            };
            out.push(DocumentSummary {
                fingerprint,
                filename,
                last_checked,
                status,
            });
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{CheckedEntry, Discrepancy, EntryOutcome, ReferenceEntry};

    fn sample() -> CheckResult {
        CheckResult {
            filename: "a.pdf".into(),
            fingerprint: "sha256:aaa".into(),
            run_at: "2026-05-20T10:00:00Z".into(),
            bibliography_detected: true,
            entries: vec![CheckedEntry {
                entry: ReferenceEntry {
                    ordinal: 1,
                    raw_text: "ref".into(),
                    doi: Some("10.1/a".into()),
                },
                outcome: EntryOutcome::Resolved {
                    doi: "10.1/a".into(),
                    discrepancies: vec![Discrepancy {
                        field: "year".into(),
                        reference_value: "(year not found)".into(),
                        crossref_value: "2020".into(),
                        dismissed: false,
                    }],
                    from_cache: false,
                },
                llm_source: None,
            }],
        }
    }

    #[test]
    fn save_then_retrieve_latest_report() {
        let mut store = Store::open_in_memory().unwrap();
        store.save_check(&sample(), "pdf", "REPORT TEXT").unwrap();
        assert_eq!(
            store.latest_report("sha256:aaa").unwrap().as_deref(),
            Some("REPORT TEXT")
        );
        let docs = store.list_documents().unwrap();
        assert_eq!(docs.len(), 1);
        assert_eq!(docs[0].status, DocumentStatus::HasIssues);
    }

    #[test]
    fn settings_round_trip_with_default_absent() {
        let store = Store::open_in_memory().unwrap();
        assert_eq!(store.get_setting("crossref_email").unwrap(), None);
        store
            .set_setting("crossref_email", "me@example.com")
            .unwrap();
        assert_eq!(
            store.get_setting("crossref_email").unwrap().as_deref(),
            Some("me@example.com")
        );
    }

    #[test]
    fn concurrency_defaults_to_5_and_round_trips() {
        let store = Store::open_in_memory().unwrap();
        // Default when absent.
        assert_eq!(store.concurrency(), 5);
        // Stored value is returned.
        store.set_concurrency(8).unwrap();
        assert_eq!(store.concurrency(), 8);
        // Values are clamped.
        store.set_concurrency(0).unwrap();
        assert_eq!(store.concurrency(), 1);
        store.set_concurrency(25).unwrap();
        assert_eq!(store.concurrency(), 20);
    }

    #[test]
    fn save_then_retrieve_structured_result() {
        let mut store = Store::open_in_memory().unwrap();
        let r = sample();
        store.save_check(&r, "pdf", "REPORT TEXT").unwrap();
        let got = store.latest_result("sha256:aaa").unwrap();
        assert_eq!(got, Some(r));
        assert_eq!(store.latest_result("sha256:none").unwrap(), None);
    }

    #[test]
    fn doi_cache_round_trips() {
        let store = Store::open_in_memory().unwrap();
        assert_eq!(store.cache_get("10.1/x").unwrap(), None);
        store.cache_put("10.1/x", "{\"message\":{}}").unwrap();
        assert_eq!(
            store.cache_get("10.1/x").unwrap().as_deref(),
            Some("{\"message\":{}}")
        );
        // Replacing updates the value.
        store.cache_put("10.1/x", "{\"v\":2}").unwrap();
        assert_eq!(
            store.cache_get("10.1/x").unwrap().as_deref(),
            Some("{\"v\":2}")
        );
    }

    #[test]
    fn cache_entries_expire_after_ttl() {
        let store = Store::open_in_memory().unwrap();
        store.cache_put("10.1/x", "{}").unwrap();
        assert!(store.cache_get("10.1/x").unwrap().is_some());
        // Backdate the entry beyond the TTL; it must then read as a miss.
        let stale = (chrono::Utc::now() - chrono::Duration::days(CACHE_TTL_DAYS + 1)).to_rfc3339();
        store
            .conn
            .execute(
                "UPDATE crossref_cache SET fetched_at = ?1 WHERE doi = ?2",
                params![stale, "10.1/x"],
            )
            .unwrap();
        assert_eq!(store.cache_get("10.1/x").unwrap(), None);
    }

    #[test]
    fn search_cache_round_trips_and_is_separate_from_doi_cache() {
        let store = Store::open_in_memory().unwrap();
        assert_eq!(store.search_cache_get("qk1").unwrap(), None);
        store
            .search_cache_put("qk1", "{\"doi\":\"10.1/x\"}")
            .unwrap();
        assert_eq!(
            store.search_cache_get("qk1").unwrap().as_deref(),
            Some("{\"doi\":\"10.1/x\"}")
        );
        // The search cache must not be visible to the DOI cache and vice versa.
        assert_eq!(store.cache_get("qk1").unwrap(), None);
    }

    #[test]
    fn search_cache_entries_expire_after_ttl() {
        let store = Store::open_in_memory().unwrap();
        store.search_cache_put("qk1", "{}").unwrap();
        assert!(store.search_cache_get("qk1").unwrap().is_some());
        let stale = (chrono::Utc::now() - chrono::Duration::days(CACHE_TTL_DAYS + 1)).to_rfc3339();
        store
            .conn
            .execute(
                "UPDATE crossref_search_cache SET fetched_at = ?1 WHERE query = ?2",
                params![stale, "qk1"],
            )
            .unwrap();
        assert_eq!(store.search_cache_get("qk1").unwrap(), None);
    }

    #[test]
    fn status_incomplete_when_network_failed() {
        let mut store = Store::open_in_memory().unwrap();
        let mut r = sample();
        r.fingerprint = "sha256:net".into();
        r.entries = vec![CheckedEntry {
            entry: ReferenceEntry {
                ordinal: 1,
                raw_text: "x".into(),
                doi: Some("10.1/a".into()),
            },
            outcome: EntryOutcome::Unresolved {
                doi: "10.1/a".into(),
                network_error: true,
            },
            llm_source: None,
        }];
        store.save_check(&r, "pdf", "T").unwrap();
        let docs = store.list_documents().unwrap();
        let d = docs.iter().find(|d| d.fingerprint == "sha256:net").unwrap();
        assert_eq!(d.status, DocumentStatus::Incomplete);
    }

    #[test]
    fn status_failed_when_doi_not_found() {
        let mut store = Store::open_in_memory().unwrap();
        let mut r = sample();
        r.fingerprint = "sha256:nf".into();
        r.entries = vec![CheckedEntry {
            entry: ReferenceEntry {
                ordinal: 1,
                raw_text: "x".into(),
                doi: Some("10.1/a".into()),
            },
            outcome: EntryOutcome::Unresolved {
                doi: "10.1/a".into(),
                network_error: false,
            },
            llm_source: None,
        }];
        store.save_check(&r, "pdf", "T").unwrap();
        let docs = store.list_documents().unwrap();
        let d = docs.iter().find(|d| d.fingerprint == "sha256:nf").unwrap();
        assert_eq!(d.status, DocumentStatus::Failed);
    }

    #[test]
    fn document_status_serialises_to_frontend_strings() {
        let pairs = [
            (DocumentStatus::Incomplete, "incomplete"),
            (DocumentStatus::Failed, "failed"),
            (DocumentStatus::HasIssues, "has-issues"),
            (DocumentStatus::Clean, "clean"),
        ];
        for (status, expected) in pairs {
            assert_eq!(serde_json::to_value(status).unwrap(), expected);
        }
    }

    #[test]
    fn delete_document_keeps_doi_cache() {
        let mut store = Store::open_in_memory().unwrap();
        store.save_check(&sample(), "pdf", "T").unwrap();
        store.cache_put("10.1/a", "{\"message\":{}}").unwrap();
        store.delete_document("sha256:aaa").unwrap();
        assert!(store.latest_result("sha256:aaa").unwrap().is_none());
        assert!(store.list_documents().unwrap().is_empty());
        // The DOI cache must survive.
        assert_eq!(
            store.cache_get("10.1/a").unwrap().as_deref(),
            Some("{\"message\":{}}")
        );
    }

    #[test]
    fn migrate_is_idempotent_on_a_persisted_db() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("d.sqlite3");
        {
            let mut s = Store::open(&path).unwrap();
            s.save_check(&sample(), "pdf", "T").unwrap();
        }
        // Reopen: migrate must run again without error and data persists.
        let s = Store::open(&path).unwrap();
        assert!(s.latest_result("sha256:aaa").unwrap().is_some());
    }

    #[test]
    fn dismissal_clears_issue_and_status() {
        let mut store = Store::open_in_memory().unwrap();
        store.save_check(&sample(), "pdf", "T").unwrap();
        // sample() has one resolved entry with a 'year' discrepancy on DOI 10.1/a.
        assert_eq!(
            store.list_documents().unwrap()[0].status,
            DocumentStatus::HasIssues
        );
        store.add_dismissal("sha256:aaa", "10.1/a", "year").unwrap();
        let r = store.latest_result("sha256:aaa").unwrap().unwrap();
        // The discrepancy is now annotated dismissed, so no active issues.
        assert_eq!(r.counts().with_discrepancies, 0);
        assert_eq!(r.counts().dismissed, 1);
        assert_eq!(
            store.list_documents().unwrap()[0].status,
            DocumentStatus::Clean
        );
        store
            .remove_dismissal("sha256:aaa", "10.1/a", "year")
            .unwrap();
        assert_eq!(
            store.list_documents().unwrap()[0].status,
            DocumentStatus::HasIssues
        );
    }
}
