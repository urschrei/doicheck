//! Rendering a CheckResult to the canonical plain-text report.

use crate::model::{CheckResult, CheckedEntry, EntryOutcome};
use std::fmt::Write;

/// A single-line, length-limited identifier for a reference, derived from its
/// raw text: internal whitespace collapsed and truncated to `MAX` characters
/// with a trailing ellipsis when truncated.
fn snippet(raw: &str) -> String {
    const MAX: usize = 80;
    let collapsed = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.chars().count() <= MAX {
        collapsed
    } else {
        let truncated: String = collapsed.chars().take(MAX).collect();
        format!("{truncated}…")
    }
}

/// Indent that aligns continuation lines under the "  [{ordinal}] " entry prefix.
fn entry_indent(ordinal: usize) -> String {
    // Width of "  [" (3) + the ordinal's digits + "] " (2).
    let digits = ordinal.checked_ilog10().map_or(1, |d| d as usize + 1);
    " ".repeat(3 + digits + 2)
}

/// Append the "possible AI source" marker line when the entry carries one.
fn write_marker(s: &mut String, indent: &str, entry: &CheckedEntry) {
    if let Some(marker) = &entry.llm_source {
        let _ = writeln!(
            s,
            "{indent}** POSSIBLE AI SOURCE - reference URL contains \"{marker}\" **"
        );
    }
}

pub fn render(result: &CheckResult) -> String {
    let c = result.counts();
    let mut s = String::new();
    let _ = writeln!(s, "DOI Check Report");
    let _ = writeln!(s, "Document:     {}", result.filename);
    let _ = writeln!(s, "Fingerprint:  {}", result.fingerprint);
    let _ = writeln!(s, "Date / Time:  {}", result.run_at);
    let _ = writeln!(s);
    let _ = writeln!(s, "Summary");
    if result.bibliography_detected {
        let _ = writeln!(s, "  Bibliography entries:        {}", c.total);
    } else {
        let _ = writeln!(
            s,
            "  Bibliography entries:        n/a (no bibliography detected)"
        );
    }
    let _ = writeln!(s, "  Checkable (with DOI):        {}", c.checkable);
    let _ = writeln!(s, "  Resolved (Crossref/DataCite):{}", c.resolved);
    // Lookups (DOI resolves + bibliographic searches, across Crossref and
    // DataCite) served from cache versus the total made, so the figure reflects
    // every avoided API call.
    let from_cache = c.from_cache + c.searched_from_cache;
    let lookups = c.resolved + c.searched;
    let _ = writeln!(
        s,
        "  Lookups from cache:          {from_cache} of {lookups}"
    );
    let _ = writeln!(s, "  Not resolved:                {}", c.unresolved);
    let _ = writeln!(s, "  Entries with discrepancies:  {}", c.with_discrepancies);
    let _ = writeln!(s, "  Dismissed (false positives): {}", c.dismissed);
    let _ = writeln!(
        s,
        "  No-DOI entries flagged:      {}",
        c.missing_doi_flagged
    );
    let _ = writeln!(s, "  Matched via search:          {}", c.matched_via_search);
    if c.llm_flagged > 0 {
        let _ = writeln!(s, "  Possible AI sources flagged: {}", c.llm_flagged);
    }
    let retry_ords: Vec<String> = result
        .entries
        .iter()
        .filter_map(|e| {
            if let EntryOutcome::Unresolved {
                network_error: true,
                ..
            } = &e.outcome
            {
                Some(format!("[{}]", e.entry.ordinal))
            } else {
                None
            }
        })
        .collect();
    if !retry_ords.is_empty() {
        let noun = if retry_ords.len() == 1 {
            "entry"
        } else {
            "entries"
        };
        let _ = writeln!(
            s,
            "  Note: {} {} could not be checked (network or capacity) and should be re-checked: {}",
            retry_ords.len(),
            noun,
            retry_ords.join(", ")
        );
    }
    let _ = writeln!(s);

    let _ = writeln!(s, "Discrepancies");
    let mut any_disc = false;
    for e in &result.entries {
        match &e.outcome {
            EntryOutcome::Resolved {
                doi,
                discrepancies,
                source,
                via_search,
                ..
            } if discrepancies.iter().any(|d| !d.dismissed) => {
                any_disc = true;
                let indent = entry_indent(e.entry.ordinal);
                let _ = writeln!(s, "  [{}] {}", e.entry.ordinal, snippet(&e.entry.raw_text));
                if *via_search {
                    let _ = writeln!(s, "{indent}no DOI; matched via {} search", source.label());
                }
                for d in discrepancies.iter().filter(|d| !d.dismissed) {
                    let _ = writeln!(
                        s,
                        "{indent}{}  {}: ref \"{}\" vs {} \"{}\"",
                        doi,
                        d.field,
                        d.reference_value,
                        source.label(),
                        d.crossref_value
                    );
                }
                write_marker(&mut s, &indent, e);
            }
            EntryOutcome::Unresolved { doi, network_error } => {
                any_disc = true;
                let reason = if *network_error {
                    "could not be checked — retry needed"
                } else {
                    "DOI not found on Crossref or DataCite"
                };
                let indent = entry_indent(e.entry.ordinal);
                let _ = writeln!(s, "  [{}] {}", e.entry.ordinal, snippet(&e.entry.raw_text));
                let _ = writeln!(s, "{indent}{}  {}", doi, reason);
                write_marker(&mut s, &indent, e);
            }
            // A resolved-but-clean entry or a no-DOI entry only appears in this
            // section when it carries an AI-source marker.
            EntryOutcome::Resolved { .. } | EntryOutcome::NoDoi { .. } => {
                if e.llm_source.is_some() {
                    any_disc = true;
                    let indent = entry_indent(e.entry.ordinal);
                    let _ = writeln!(s, "  [{}] {}", e.entry.ordinal, snippet(&e.entry.raw_text));
                    write_marker(&mut s, &indent, e);
                }
            }
        }
    }
    if !any_disc {
        let _ = writeln!(s, "  (none)");
    }
    let _ = writeln!(s);

    let _ = writeln!(s, "Possibly missing DOIs");
    let mut any_missing = false;
    for e in &result.entries {
        match &e.outcome {
            EntryOutcome::NoDoi {
                suggested: Some(sug),
                ..
            } => {
                any_missing = true;
                let indent = entry_indent(e.entry.ordinal);
                let _ = writeln!(s, "  [{}] {}", e.entry.ordinal, snippet(&e.entry.raw_text));
                let _ = writeln!(
                    s,
                    "{indent}no DOI; closest {} match {} (title match {}%)",
                    sug.source.label(),
                    sug.doi,
                    sug.title_match
                );
            }
            EntryOutcome::Resolved {
                doi,
                discrepancies,
                source,
                via_search: true,
                ..
            } if !discrepancies.iter().any(|d| !d.dismissed) => {
                any_missing = true;
                let indent = entry_indent(e.entry.ordinal);
                let _ = writeln!(s, "  [{}] {}", e.entry.ordinal, snippet(&e.entry.raw_text));
                let _ = writeln!(
                    s,
                    "{indent}no DOI; matched via {} search: {}",
                    source.label(),
                    doi
                );
            }
            EntryOutcome::NoDoi {
                suggested: None, ..
            }
            | EntryOutcome::Resolved { .. }
            | EntryOutcome::Unresolved { .. } => {}
        }
    }
    if !any_missing {
        let _ = writeln!(s, "  (none)");
    }

    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{CheckedEntry, Discrepancy, ReferenceEntry, Source, SuggestedDoi};

    #[test]
    fn renders_summary_discrepancies_and_missing() {
        let result = CheckResult {
            filename: "thesis.pdf".into(),
            fingerprint: "sha256:a3f1".into(),
            run_at: "2026-05-20 18:40:12".into(),
            bibliography_detected: true,
            entries: vec![
                CheckedEntry {
                    entry: ReferenceEntry {
                        ordinal: 12,
                        raw_text: "Smith, J. (2020). Neural things. Journal.".into(),
                        doi: Some("10.1/yyy".into()),
                    },
                    outcome: EntryOutcome::Resolved {
                        doi: "10.1/yyy".into(),
                        discrepancies: vec![Discrepancy {
                            field: "title".into(),
                            reference_value: "(title not found in reference)".into(),
                            crossref_value: "Neural Things".into(),
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
                        ordinal: 33,
                        raw_text: "Lee, C. (2018). Untitled work.".into(),
                        doi: None,
                    },
                    outcome: EntryOutcome::NoDoi {
                        suggested: Some(SuggestedDoi {
                            doi: "10.1000/xyz".into(),
                            title_match: 82,
                            source: Default::default(),
                        }),
                        from_cache: false,
                    },
                    llm_source: None,
                },
            ],
        };
        let text = render(&result);
        assert!(text.contains("Document:     thesis.pdf"));
        assert!(text.contains("[12] Smith, J. (2020). Neural things. Journal."));
        assert!(text.contains("10.1/yyy  title:"));
        assert!(text.contains("Neural Things"));
        assert!(text.contains("[33] Lee, C. (2018). Untitled work."));
        assert!(text.contains("no DOI; closest Crossref match 10.1000/xyz (title match 82%)"));
        assert!(text.contains("from cache:"));
    }

    #[test]
    fn renders_datacite_source_in_discrepancies_and_suggestions() {
        let result = CheckResult {
            filename: "x.pdf".into(),
            fingerprint: "fp".into(),
            run_at: "now".into(),
            bibliography_detected: true,
            entries: vec![
                CheckedEntry {
                    entry: ReferenceEntry {
                        ordinal: 1,
                        raw_text: "A dataset.".into(),
                        doi: Some("10.5281/zenodo.1".into()),
                    },
                    outcome: EntryOutcome::Resolved {
                        doi: "10.5281/zenodo.1".into(),
                        discrepancies: vec![Discrepancy {
                            field: "year".into(),
                            reference_value: "1999".into(),
                            crossref_value: "2020".into(),
                            dismissed: false,
                        }],
                        from_cache: false,
                        source: Source::DataCite,
                        via_search: false,
                    },
                    llm_source: None,
                },
                CheckedEntry {
                    entry: ReferenceEntry {
                        ordinal: 2,
                        raw_text: "No DOI here.".into(),
                        doi: None,
                    },
                    outcome: EntryOutcome::NoDoi {
                        suggested: Some(SuggestedDoi {
                            doi: "10.6084/m9.1".into(),
                            title_match: 88,
                            source: Source::DataCite,
                        }),
                        from_cache: false,
                    },
                    llm_source: None,
                },
            ],
        };
        let text = render(&result);
        assert!(text.contains("vs DataCite"), "{text}");
        assert!(
            text.contains("closest DataCite match 10.6084/m9.1"),
            "{text}"
        );
    }

    #[test]
    fn renders_retry_note_and_unresolved_wording() {
        let result = CheckResult {
            filename: "x.pdf".into(),
            fingerprint: "fp".into(),
            run_at: "now".into(),
            bibliography_detected: true,
            entries: vec![
                CheckedEntry {
                    entry: ReferenceEntry {
                        ordinal: 7,
                        raw_text: "Brown, B. (2021). Unreachable.".into(),
                        doi: Some("10.3/www".into()),
                    },
                    outcome: EntryOutcome::Unresolved {
                        doi: "10.3/www".into(),
                        network_error: true,
                    },
                    llm_source: None,
                },
                CheckedEntry {
                    entry: ReferenceEntry {
                        ordinal: 9,
                        raw_text: "Jones, A. (2019). Missing DOI.".into(),
                        doi: Some("10.2/zzz".into()),
                    },
                    outcome: EntryOutcome::Unresolved {
                        doi: "10.2/zzz".into(),
                        network_error: false,
                    },
                    llm_source: None,
                },
            ],
        };
        let text = render(&result);
        assert!(text.contains("could not be checked — retry needed"));
        assert!(text.contains("DOI not found on Crossref"));
        assert!(text.contains("[7] Brown, B. (2021). Unreachable."));
        assert!(text.contains(
            "Note: 1 entry could not be checked (network or capacity) and should be re-checked: [7]"
        ));
        // The genuine 404 (ordinal 9) must not be listed as needing a re-check.
        assert!(!text.contains("re-checked: [9]"));
        assert!(!text.contains("[7], [9]"));
        // Two-line layout: each detail sits on its own continuation line.
        assert!(text.contains("10.3/www  could not be checked — retry needed"));
        assert!(text.contains("10.2/zzz  DOI not found on Crossref"));
    }

    #[test]
    fn renders_via_search_matches() {
        let result = CheckResult {
            filename: "x.pdf".into(),
            fingerprint: "fp".into(),
            run_at: "now".into(),
            bibliography_detected: true,
            entries: vec![
                CheckedEntry {
                    entry: ReferenceEntry {
                        ordinal: 1,
                        raw_text: "Smith (2020). A clean match.".into(),
                        doi: None,
                    },
                    outcome: EntryOutcome::Resolved {
                        doi: "10.1/clean".into(),
                        discrepancies: vec![],
                        from_cache: false,
                        source: Source::Crossref,
                        via_search: true,
                    },
                    llm_source: None,
                },
                CheckedEntry {
                    entry: ReferenceEntry {
                        ordinal: 2,
                        raw_text: "Lee (1999). A mismatched match.".into(),
                        doi: None,
                    },
                    outcome: EntryOutcome::Resolved {
                        doi: "10.1/mism".into(),
                        discrepancies: vec![Discrepancy {
                            field: "year".into(),
                            reference_value: "1999".into(),
                            crossref_value: "2020".into(),
                            dismissed: false,
                        }],
                        from_cache: false,
                        source: Source::DataCite,
                        via_search: true,
                    },
                    llm_source: None,
                },
            ],
        };
        let text = render(&result);
        // Summary line carries the count (two via_search entries here).
        assert!(
            text.lines()
                .any(|l| l.split_whitespace().collect::<Vec<_>>().join(" ")
                    == "Matched via search: 2"),
            "{text}"
        );
        // Clean via-search entry listed under missing DOIs as a confirmed match.
        assert!(
            text.contains("no DOI; matched via Crossref search: 10.1/clean"),
            "{text}"
        );
        // Mismatched via-search entry annotated in the Discrepancies section.
        assert!(
            text.contains("no DOI; matched via DataCite search"),
            "{text}"
        );
        assert!(text.contains("10.1/mism  year:"), "{text}");
    }

    #[test]
    fn snippet_keeps_short_text() {
        assert_eq!(snippet("Smith 2020"), "Smith 2020");
    }

    #[test]
    fn snippet_collapses_whitespace() {
        assert_eq!(snippet("Smith,\n  J.   (2020)"), "Smith, J. (2020)");
    }

    #[test]
    fn snippet_truncates_long_text() {
        let long = "a".repeat(200);
        let s = snippet(&long);
        assert_eq!(s.chars().count(), 81); // 80 chars + ellipsis
        assert!(s.ends_with('…'));
    }
}
