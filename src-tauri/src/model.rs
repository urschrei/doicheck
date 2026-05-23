//! Shared data types passed between pipeline stages and to the UI.

use serde::{Deserialize, Serialize};
use std::collections::HashSet;

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

/// Which DOI registration agency a result came from. Defaults to Crossref so
/// results stored before this field existed still deserialise.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Source {
    #[default]
    Crossref,
    DataCite,
}

impl Source {
    /// Human-readable agency name for reports and the UI.
    pub fn label(self) -> &'static str {
        match self {
            Source::Crossref => "Crossref",
            Source::DataCite => "DataCite",
        }
    }
}

/// A DOI suggested for an entry that had none, found by title search.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SuggestedDoi {
    pub doi: String,
    /// Title-token match against the reference, 0-100.
    pub title_match: u8,
    /// Which agency the suggestion was found in.
    #[serde(default)]
    pub source: Source,
}

/// What the doi.org registration-agency lookup reported for an unresolved DOI.
/// Carried on `EntryOutcome::Unresolved` so the UI can tell a DOI that is not
/// registered anywhere from a valid DOI that Crossref and DataCite simply do not
/// index. `Unknown` covers a DOI that was not checked (a network failure, or a
/// result stored before this field existed).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Registration {
    #[default]
    Unknown,
    /// doi.org has no record of the DOI string.
    Unregistered,
    /// Registered with the named agency (e.g. "mEDRA", "JaLC"; or Crossref or
    /// DataCite when their REST metadata is missing despite registration).
    Agency(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum EntryOutcome {
    Resolved {
        doi: String,
        discrepancies: Vec<Discrepancy>,
        from_cache: bool,
        /// Which agency resolved the DOI.
        #[serde(default)]
        source: Source,
        /// True when the entry had no cited DOI and was matched by a full-title
        /// bibliographic search rather than a DOI in the reference. Defaults to
        /// false so results stored before this field existed still deserialise.
        #[serde(default)]
        via_search: bool,
    },
    Unresolved {
        doi: String,
        network_error: bool,
        /// What the doi.org registration-agency lookup reported. Defaults to
        /// `Unknown` so results stored before this field existed still
        /// deserialise.
        #[serde(default)]
        registration: Registration,
        /// A bibliographic-search match for the reference text, found when the
        /// cited DOI did not resolve, so the UI can offer the likely correct DOI
        /// or note that no record matches. Defaults to `None`.
        #[serde(default)]
        suggested: Option<SuggestedDoi>,
    },
    NoDoi {
        suggested: Option<SuggestedDoi>,
        /// Whether the bibliographic search that produced `suggested` was served
        /// from the search cache rather than fetched from Crossref. Defaults to
        /// false so results stored before this field existed still deserialise.
        #[serde(default)]
        from_cache: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckedEntry {
    pub entry: ReferenceEntry,
    pub outcome: EntryOutcome,
    #[serde(default)]
    pub llm_source: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Counts {
    pub total: usize,
    pub checkable: usize,
    pub resolved: usize,
    pub from_cache: usize,
    /// Bibliographic searches performed for no-DOI references (each is one
    /// Crossref or DataCite lookup), and how many of those were served from the
    /// search cache. Covers both `NoDoi` entries and no-DOI references promoted
    /// to `Resolved` by a full-title match.
    pub searched: usize,
    pub searched_from_cache: usize,
    pub unresolved: usize,
    pub with_discrepancies: usize,
    pub dismissed: usize,
    pub missing_doi_flagged: usize,
    /// No-DOI references confirmed by a full-title bibliographic search.
    pub matched_via_search: usize,
    pub network_failed: usize,
    pub llm_flagged: usize,
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
    pub fn apply_dismissals(&mut self, set: &HashSet<(String, String)>) {
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
            if e.llm_source.is_some() {
                c.llm_flagged += 1;
            }
            match &e.outcome {
                EntryOutcome::Resolved {
                    discrepancies,
                    from_cache,
                    via_search,
                    ..
                } => {
                    let active = discrepancies.iter().filter(|d| !d.dismissed).count();
                    if active > 0 {
                        c.with_discrepancies += 1;
                    }
                    c.dismissed += discrepancies.len() - active;
                    if *via_search {
                        c.matched_via_search += 1;
                        c.searched += 1;
                        if *from_cache {
                            c.searched_from_cache += 1;
                        }
                    } else {
                        c.checkable += 1;
                        c.resolved += 1;
                        if *from_cache {
                            c.from_cache += 1;
                        }
                    }
                }
                EntryOutcome::Unresolved { network_error, .. } => {
                    c.checkable += 1;
                    c.unresolved += 1;
                    if *network_error {
                        c.network_failed += 1;
                    }
                }
                EntryOutcome::NoDoi {
                    suggested,
                    from_cache,
                } => {
                    c.searched += 1;
                    if *from_cache {
                        c.searched_from_cache += 1;
                    }
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
                        source: Default::default(),
                        via_search: false,
                    },
                    llm_source: None,
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
                        source: Default::default(),
                        via_search: false,
                    },
                    llm_source: None,
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
                        registration: Registration::Unknown,
                        suggested: None,
                    },
                    llm_source: None,
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
                            source: Default::default(),
                        }),
                        from_cache: false,
                    },
                    llm_source: None,
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
                        registration: Registration::Unknown,
                        suggested: None,
                    },
                    llm_source: None,
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
                        registration: Registration::Unknown,
                        suggested: None,
                    },
                    llm_source: None,
                },
            ],
        };
        let c = result.counts();
        assert_eq!(c.unresolved, 2);
        assert_eq!(c.network_failed, 1);
    }

    #[test]
    fn counts_via_search_match_is_separate_from_cited_dois() {
        let entry = |ordinal, via_search, discrepancies, from_cache| CheckedEntry {
            entry: ReferenceEntry {
                ordinal,
                raw_text: "x".into(),
                doi: None,
            },
            outcome: EntryOutcome::Resolved {
                doi: "10.1/m".into(),
                discrepancies,
                from_cache,
                source: Default::default(),
                via_search,
            },
            llm_source: None,
        };
        let result = CheckResult {
            filename: "x.pdf".into(),
            fingerprint: "fp".into(),
            run_at: "now".into(),
            bibliography_detected: true,
            entries: vec![
                entry(1, true, vec![], true),
                entry(
                    2,
                    true,
                    vec![Discrepancy {
                        field: "year".into(),
                        reference_value: "1999".into(),
                        crossref_value: "2020".into(),
                        dismissed: false,
                    }],
                    false,
                ),
            ],
        };
        let c = result.counts();
        assert_eq!(c.total, 2);
        // Via-search matches are not cited-DOI entries.
        assert_eq!(c.checkable, 0);
        assert_eq!(c.resolved, 0);
        // Both count as confirmed search matches and as search lookups.
        assert_eq!(c.matched_via_search, 2);
        assert_eq!(c.searched, 2);
        assert_eq!(c.searched_from_cache, 1);
        // The mismatched one still counts as a discrepancy.
        assert_eq!(c.with_discrepancies, 1);
    }

    #[test]
    fn counts_track_search_cache_hits() {
        let no_doi = |ordinal, from_cache| CheckedEntry {
            entry: ReferenceEntry {
                ordinal,
                raw_text: "x".into(),
                doi: None,
            },
            outcome: EntryOutcome::NoDoi {
                suggested: None,
                from_cache,
            },
            llm_source: None,
        };
        let result = CheckResult {
            filename: "x.pdf".into(),
            fingerprint: "abc".into(),
            run_at: "now".into(),
            bibliography_detected: true,
            entries: vec![no_doi(1, true), no_doi(2, false), no_doi(3, true)],
        };
        let c = result.counts();
        // Every no-DOI entry is one bibliographic-search lookup.
        assert_eq!(c.searched, 3);
        // Two of them were served from the search cache.
        assert_eq!(c.searched_from_cache, 2);
    }
}
