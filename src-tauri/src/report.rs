//! Rendering a CheckResult to the canonical plain-text report.

use crate::model::{CheckResult, EntryOutcome};
use std::fmt::Write;

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
    let _ = writeln!(s, "  Resolved on Crossref:        {}", c.resolved);
    let _ = writeln!(
        s,
        "    from cache: {}, fetched: {}",
        c.from_cache,
        c.resolved.saturating_sub(c.from_cache)
    );
    let _ = writeln!(s, "  Not resolved:                {}", c.unresolved);
    let _ = writeln!(s, "  Entries with discrepancies:  {}", c.with_discrepancies);
    let _ = writeln!(s, "  Dismissed (false positives): {}", c.dismissed);
    let _ = writeln!(
        s,
        "  No-DOI entries flagged:      {}",
        c.missing_doi_flagged
    );
    if c.llm_flagged > 0 {
        let _ = writeln!(s, "  Possible AI sources flagged: {}", c.llm_flagged);
    }
    let _ = writeln!(s);

    let _ = writeln!(s, "Discrepancies");
    let mut any_disc = false;
    for e in &result.entries {
        match &e.outcome {
            EntryOutcome::Resolved {
                doi, discrepancies, ..
            } if discrepancies.iter().any(|d| !d.dismissed) => {
                any_disc = true;
                for d in discrepancies.iter().filter(|d| !d.dismissed) {
                    let _ = writeln!(
                        s,
                        "  [{}] {}  {}: ref {} vs Crossref \"{}\"",
                        e.entry.ordinal, doi, d.field, d.reference_value, d.crossref_value
                    );
                }
                if let Some(marker) = &e.llm_source {
                    let _ = writeln!(
                        s,
                        "    ** POSSIBLE AI SOURCE - reference URL contains \"{}\" **",
                        marker
                    );
                }
            }
            EntryOutcome::Unresolved { doi, network_error } => {
                any_disc = true;
                let reason = if *network_error {
                    "check failed (network)"
                } else {
                    "not found on Crossref"
                };
                let _ = writeln!(s, "  [{}] {}  {}", e.entry.ordinal, doi, reason);
                if let Some(marker) = &e.llm_source {
                    let _ = writeln!(
                        s,
                        "    ** POSSIBLE AI SOURCE - reference URL contains \"{}\" **",
                        marker
                    );
                }
            }
            _ => {
                if let Some(marker) = &e.llm_source {
                    any_disc = true;
                    let _ = writeln!(
                        s,
                        "  [{}]   ** POSSIBLE AI SOURCE - reference URL contains \"{}\" **",
                        e.entry.ordinal, marker
                    );
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
        if let EntryOutcome::NoDoi {
            suggested: Some(sug),
        } = &e.outcome
        {
            any_missing = true;
            let _ = writeln!(
                s,
                "  [{}] no DOI; closest Crossref match {} (title match {}%)",
                e.entry.ordinal, sug.doi, sug.title_match
            );
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
    use crate::model::{CheckedEntry, Discrepancy, ReferenceEntry, SuggestedDoi};

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
                        raw_text: "r".into(),
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
                    },
                    llm_source: None,
                },
                CheckedEntry {
                    entry: ReferenceEntry {
                        ordinal: 33,
                        raw_text: "r".into(),
                        doi: None,
                    },
                    outcome: EntryOutcome::NoDoi {
                        suggested: Some(SuggestedDoi {
                            doi: "10.1000/xyz".into(),
                            title_match: 82,
                        }),
                    },
                    llm_source: None,
                },
            ],
        };
        let text = render(&result);
        assert!(text.contains("Document:     thesis.pdf"));
        assert!(text.contains("[12] 10.1/yyy  title:"));
        assert!(text.contains("Neural Things"));
        assert!(text.contains("[33] no DOI; closest Crossref match 10.1000/xyz (title match 82%)"));
        assert!(text.contains("from cache:"));
    }
}
