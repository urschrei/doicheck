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
                report_text TEXT NOT NULL
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
            "#,
        )?;
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
                 unresolved, with_discrepancies, missing_doi_flagged, report_text)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",
            params![
                document_id,
                result.run_at,
                counts.total as i64,
                counts.checkable as i64,
                counts.resolved as i64,
                counts.unresolved as i64,
                counts.with_discrepancies as i64,
                counts.missing_doi_flagged as i64,
                report_text
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

    /// Sidebar list: one row per document with its latest status.
    pub fn list_documents(&self) -> Result<Vec<DocumentSummary>, StoreError> {
        let mut stmt = self.conn.prepare(
            "SELECT d.fingerprint, d.filename, d.last_checked,
                 (SELECT CASE
                    WHEN c.with_discrepancies > 0 OR c.unresolved > 0 THEN 'has-issues'
                    ELSE 'clean' END
                  FROM checks c WHERE c.document_id = d.id ORDER BY c.id DESC LIMIT 1)
             FROM documents d ORDER BY d.last_checked DESC",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(DocumentSummary {
                fingerprint: r.get(0)?,
                filename: r.get(1)?,
                last_checked: r.get(2)?,
                status: r
                    .get::<_, Option<String>>(3)?
                    .unwrap_or_else(|| "clean".into()),
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(StoreError::from)
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
                    }],
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
}
