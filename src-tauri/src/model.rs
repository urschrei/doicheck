//! Shared data types passed between pipeline stages and to the UI.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FileKind {
    Pdf,
    Docx,
}

/// One reference as found in the bibliography.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReferenceEntry {
    pub ordinal: usize,
    pub raw_text: String,
    pub doi: Option<String>,
}

/// A single recorded mismatch between Crossref metadata and the reference text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Discrepancy {
    pub field: String,
    pub reference_value: String,
    pub crossref_value: String,
    #[serde(default)]
    pub dismissed: bool,
}

/// A DOI suggested for an entry that had none, found by title search.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SuggestedDoi {
    pub doi: String,
    /// Title-token match against the reference, 0-100.
    pub title_match: u8,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum EntryOutcome {
    Resolved {
        doi: String,
        discrepancies: Vec<Discrepancy>,
        from_cache: bool,
    },
    Unresolved {
        doi: String,
        network_error: bool,
    },
    NoDoi {
        suggested: Option<SuggestedDoi>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckedEntry {
    pub entry: ReferenceEntry,
    pub outcome: EntryOutcome,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Counts {
    pub total: usize,
    pub checkable: usize,
    pub resolved: usize,
    pub from_cache: usize,
    pub unresolved: usize,
    pub with_discrepancies: usize,
    pub dismissed: usize,
    pub missing_doi_flagged: usize,
    pub network_failed: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckResult {
    pub filename: String,
    pub fingerprint: String,
    pub run_at: String,
    pub bibliography_detected: bool,
    pub entries: Vec<CheckedEntry>,
}

/// Progress update emitted once per entry as it is checked.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Progress {
    pub done: usize,
    pub total: usize,
    pub cached: usize,
    pub fetched: usize,
}

impl CheckResult {
    /// Mark discrepancies dismissed where (resolved DOI, field) is in `set`.
    pub fn apply_dismissals(&mut self, set: &std::collections::HashSet<(String, String)>) {
        for ce in &mut self.entries {
            if let EntryOutcome::Resolved {
                doi, discrepancies, ..
            } = &mut ce.outcome
            {
                for d in discrepancies.iter_mut() {
                    d.dismissed = set.contains(&(doi.clone(), d.field.clone()));
                }
            }
        }
    }

    pub fn counts(&self) -> Counts {
        let mut c = Counts {
            total: self.entries.len(),
            ..Counts::default()
        };
        for e in &self.entries {
            match &e.outcome {
                EntryOutcome::Resolved {
                    discrepancies,
                    from_cache,
                    ..
                } => {
                    c.checkable += 1;
                    c.resolved += 1;
                    let active = discrepancies.iter().filter(|d| !d.dismissed).count();
                    if active > 0 {
                        c.with_discrepancies += 1;
                    }
                    c.dismissed += discrepancies.len() - active;
                    if *from_cache {
                        c.from_cache += 1;
                    }
                }
                EntryOutcome::Unresolved { network_error, .. } => {
                    c.checkable += 1;
                    c.unresolved += 1;
                    if *network_error {
                        c.network_failed += 1;
                    }
                }
                EntryOutcome::NoDoi { suggested } => {
                    if suggested.is_some() {
                        c.missing_doi_flagged += 1;
                    }
                }
            }
        }
        c
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_classify_each_outcome() {
        let result = CheckResult {
            filename: "x.pdf".into(),
            fingerprint: "abc".into(),
            run_at: "2026-05-20T00:00:00Z".into(),
            bibliography_detected: true,
            entries: vec![
                CheckedEntry {
                    entry: ReferenceEntry {
                        ordinal: 1,
                        raw_text: "a".into(),
                        doi: Some("10.1/a".into()),
                    },
                    outcome: EntryOutcome::Resolved {
                        doi: "10.1/a".into(),
                        discrepancies: vec![],
                        from_cache: true,
                    },
                },
                CheckedEntry {
                    entry: ReferenceEntry {
                        ordinal: 2,
                        raw_text: "b".into(),
                        doi: Some("10.1/b".into()),
                    },
                    outcome: EntryOutcome::Resolved {
                        doi: "10.1/b".into(),
                        discrepancies: vec![Discrepancy {
                            field: "title".into(),
                            reference_value: "r".into(),
                            crossref_value: "c".into(),
                            dismissed: false,
                        }],
                        from_cache: false,
                    },
                },
                CheckedEntry {
                    entry: ReferenceEntry {
                        ordinal: 3,
                        raw_text: "c".into(),
                        doi: Some("10.1/c".into()),
                    },
                    outcome: EntryOutcome::Unresolved {
                        doi: "10.1/c".into(),
                        network_error: false,
                    },
                },
                CheckedEntry {
                    entry: ReferenceEntry {
                        ordinal: 4,
                        raw_text: "d".into(),
                        doi: None,
                    },
                    outcome: EntryOutcome::NoDoi {
                        suggested: Some(SuggestedDoi {
                            doi: "10.1/d".into(),
                            title_match: 90,
                        }),
                    },
                },
            ],
        };
        let c = result.counts();
        assert_eq!(c.total, 4);
        assert_eq!(c.checkable, 3);
        assert_eq!(c.resolved, 2);
        assert_eq!(c.unresolved, 1);
        assert_eq!(c.with_discrepancies, 1);
        assert_eq!(c.missing_doi_flagged, 1);
        assert_eq!(c.from_cache, 1);
    }

    #[test]
    fn counts_track_network_failures() {
        let result = CheckResult {
            filename: "x.pdf".into(),
            fingerprint: "abc".into(),
            run_at: "now".into(),
            bibliography_detected: true,
            entries: vec![
                CheckedEntry {
                    entry: ReferenceEntry {
                        ordinal: 1,
                        raw_text: "a".into(),
                        doi: Some("10.1/a".into()),
                    },
                    outcome: EntryOutcome::Unresolved {
                        doi: "10.1/a".into(),
                        network_error: true,
                    },
                },
                CheckedEntry {
                    entry: ReferenceEntry {
                        ordinal: 2,
                        raw_text: "b".into(),
                        doi: Some("10.1/b".into()),
                    },
                    outcome: EntryOutcome::Unresolved {
                        doi: "10.1/b".into(),
                        network_error: false,
                    },
                },
            ],
        };
        let c = result.counts();
        assert_eq!(c.unresolved, 2);
        assert_eq!(c.network_failed, 1);
    }
}
