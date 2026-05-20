//! SQLite persistence for documents, checks, entries, discrepancies, settings.

use crate::model::{CheckResult, EntryOutcome};
use rusqlite::{Connection, params};

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error(transparent)]
    Sqlite(#[from] rusqlite::Error),
}

pub struct Store {
    conn: Connection,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct DocumentSummary {
    pub fingerprint: String,
    pub filename: String,
    pub last_checked: String,
    pub status: String,
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
            CREATE TABLE IF NOT EXISTS entries (
                id INTEGER PRIMARY KEY,
                check_id INTEGER NOT NULL REFERENCES checks(id),
                ordinal INTEGER NOT NULL,
                raw_text TEXT NOT NULL,
                doi TEXT,
                status TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS discrepancies (
                id INTEGER PRIMARY KEY,
                entry_id INTEGER NOT NULL REFERENCES entries(id),
                field TEXT NOT NULL,
                reference_value TEXT NOT NULL,
                crossref_value TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS settings (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS crossref_cache (
                doi TEXT PRIMARY KEY,
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

    /// The cached Crossref JSON for a DOI, if present.
    pub fn cache_get(&self, doi: &str) -> Result<Option<String>, StoreError> {
        let mut stmt = self
            .conn
            .prepare("SELECT json FROM crossref_cache WHERE doi = ?1")?;
        let mut rows = stmt.query(params![doi])?;
        if let Some(row) = rows.next()? {
            Ok(Some(row.get(0)?))
        } else {
            Ok(None)
        }
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

    /// Persist a check (and its document, entries, discrepancies). `kind` is the
    /// file kind as a short string ("pdf"/"docx"). `report_text` is the rendered
    /// report. Returns the new check id.
    pub fn save_check(
        &mut self,
        result: &CheckResult,
        kind: &str,
        report_text: &str,
    ) -> Result<i64, StoreError> {
        let counts = result.counts();
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
                serde_json::to_string(result).unwrap_or_default()
            ],
        )?;
        let check_id = tx.last_insert_rowid();
        for e in &result.entries {
            let status = match &e.outcome {
                EntryOutcome::Resolved { discrepancies, .. } if discrepancies.is_empty() => {
                    "resolved"
                }
                EntryOutcome::Resolved { .. } => "resolved_with_discrepancies",
                EntryOutcome::Unresolved {
                    network_error: true,
                    ..
                } => "network_error",
                EntryOutcome::Unresolved { .. } => "not_found",
                EntryOutcome::NoDoi { suggested: Some(_) } => "no_doi_suggested",
                EntryOutcome::NoDoi { suggested: None } => "no_doi",
            };
            tx.execute(
                "INSERT INTO entries(check_id, ordinal, raw_text, doi, status)
                 VALUES(?1,?2,?3,?4,?5)",
                params![
                    check_id,
                    e.entry.ordinal as i64,
                    e.entry.raw_text,
                    e.entry.doi,
                    status
                ],
            )?;
            let entry_id = tx.last_insert_rowid();
            if let EntryOutcome::Resolved { discrepancies, .. } = &e.outcome {
                for d in discrepancies {
                    tx.execute(
                        "INSERT INTO discrepancies(entry_id, field, reference_value, crossref_value)
                         VALUES(?1,?2,?3,?4)",
                        params![entry_id, d.field, d.reference_value, d.crossref_value],
                    )?;
                }
            }
        }
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
                    Err(_) => Ok(None),
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

    /// Delete a document and all its checks/entries/discrepancies. The shared
    /// DOI cache (`crossref_cache`) is deliberately left intact.
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
            tx.execute(
                "DELETE FROM discrepancies WHERE entry_id IN
                   (SELECT e.id FROM entries e JOIN checks c ON c.id = e.check_id
                    WHERE c.document_id = ?1)",
                params![doc_id],
            )?;
            tx.execute(
                "DELETE FROM entries WHERE check_id IN
                   (SELECT id FROM checks WHERE document_id = ?1)",
                params![doc_id],
            )?;
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
                    if c.network_failed > 0 {
                        "incomplete"
                    } else if c.with_discrepancies > 0 || c.unresolved > 0 {
                        "has-issues"
                    } else {
                        "clean"
                    }
                }
                None => "clean",
            }
            .to_string();
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
    use crate::model::{CheckedEntry, Discrepancy, ReferenceEntry};

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
        assert_eq!(docs[0].status, "has-issues");
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
        }];
        store.save_check(&r, "pdf", "T").unwrap();
        let docs = store.list_documents().unwrap();
        let d = docs.iter().find(|d| d.fingerprint == "sha256:net").unwrap();
        assert_eq!(d.status, "incomplete");
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
        assert_eq!(store.list_documents().unwrap()[0].status, "has-issues");
        store.add_dismissal("sha256:aaa", "10.1/a", "year").unwrap();
        let r = store.latest_result("sha256:aaa").unwrap().unwrap();
        // The discrepancy is now annotated dismissed, so no active issues.
        assert_eq!(r.counts().with_discrepancies, 0);
        assert_eq!(r.counts().dismissed, 1);
        assert_eq!(store.list_documents().unwrap()[0].status, "clean");
        store
            .remove_dismissal("sha256:aaa", "10.1/a", "year")
            .unwrap();
        assert_eq!(store.list_documents().unwrap()[0].status, "has-issues");
    }
}
