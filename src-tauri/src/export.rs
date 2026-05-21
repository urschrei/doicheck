//! Machine-readable exports of a CheckResult: full JSON and a flat CSV.

use crate::model::{CheckResult, EntryOutcome};

/// Lossless JSON of the whole result.
pub fn to_json(result: &CheckResult) -> String {
    serde_json::to_string_pretty(result).unwrap_or_default()
}

/// One row per entry: ordinal, reference_text, doi, status, unmatched fields, suggested doi, llm_source.
pub fn to_csv(result: &CheckResult) -> String {
    let mut out = String::from(
        "ordinal,reference_text,doi,status,unmatched_fields,suggested_doi,llm_source\n",
    );
    for e in &result.entries {
        let (status, unmatched, suggested) = match &e.outcome {
            EntryOutcome::Resolved { discrepancies, .. } => {
                let active: Vec<_> = discrepancies.iter().filter(|d| !d.dismissed).collect();
                if active.is_empty() {
                    ("clean".to_string(), String::new(), String::new())
                } else {
                    (
                        "mismatch".to_string(),
                        active
                            .iter()
                            .map(|d| d.field.as_str())
                            .collect::<Vec<_>>()
                            .join("; "),
                        String::new(),
                    )
                }
            }
            EntryOutcome::Unresolved { network_error, .. } => (
                if *network_error {
                    "retry_needed"
                } else {
                    "not_found"
                }
                .to_string(),
                String::new(),
                String::new(),
            ),
            EntryOutcome::NoDoi { suggested } => (
                "no_doi".to_string(),
                String::new(),
                suggested
                    .as_ref()
                    .map(|s| s.doi.clone())
                    .unwrap_or_default(),
            ),
        };
        let llm = e.llm_source.as_deref().unwrap_or("");
        out.push_str(&format!(
            "{},{},{},{},{},{},{}\n",
            e.entry.ordinal,
            csv_field(&e.entry.raw_text),
            csv_field(e.entry.doi.as_deref().unwrap_or("")),
            status,
            csv_field(&unmatched),
            csv_field(&suggested),
            csv_field(llm),
        ));
    }
    out
}

/// Quote a CSV field if it contains a comma, quote, newline, or carriage return.
fn csv_field(s: &str) -> String {
    if s.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{CheckedEntry, Discrepancy, ReferenceEntry, SuggestedDoi};

    fn result() -> CheckResult {
        CheckResult {
            filename: "a.pdf".into(),
            fingerprint: "fp".into(),
            run_at: "now".into(),
            bibliography_detected: true,
            entries: vec![
                CheckedEntry {
                    entry: ReferenceEntry {
                        ordinal: 1,
                        raw_text: "r".into(),
                        doi: Some("10.1000/a".into()),
                    },
                    outcome: EntryOutcome::Resolved {
                        doi: "10.1000/a".into(),
                        discrepancies: vec![],
                        from_cache: false,
                    },
                    llm_source: None,
                },
                CheckedEntry {
                    entry: ReferenceEntry {
                        ordinal: 2,
                        raw_text: "r".into(),
                        doi: Some("10.1000/b".into()),
                    },
                    outcome: EntryOutcome::Resolved {
                        doi: "10.1000/b".into(),
                        discrepancies: vec![Discrepancy {
                            field: "year".into(),
                            reference_value: "x".into(),
                            crossref_value: "2020".into(),
                            dismissed: false,
                        }],
                        from_cache: false,
                    },
                    llm_source: None,
                },
                CheckedEntry {
                    entry: ReferenceEntry {
                        ordinal: 3,
                        raw_text: "r".into(),
                        doi: None,
                    },
                    outcome: EntryOutcome::NoDoi {
                        suggested: Some(SuggestedDoi {
                            doi: "10.1000/c".into(),
                            title_match: 90,
                        }),
                    },
                    llm_source: None,
                },
            ],
        }
    }

    #[test]
    fn csv_has_header_and_rows() {
        let csv = to_csv(&result());
        let lines: Vec<&str> = csv.lines().collect();
        assert_eq!(
            lines[0],
            "ordinal,reference_text,doi,status,unmatched_fields,suggested_doi,llm_source"
        );
        assert_eq!(lines[1], "1,r,10.1000/a,clean,,,");
        assert_eq!(lines[2], "2,r,10.1000/b,mismatch,year,,");
        assert_eq!(lines[3], "3,r,,no_doi,,10.1000/c,");
    }

    #[test]
    fn csv_distinguishes_retry_from_not_found() {
        let r = CheckResult {
            filename: "x.pdf".into(),
            fingerprint: "fp".into(),
            run_at: "now".into(),
            bibliography_detected: true,
            entries: vec![
                CheckedEntry {
                    entry: ReferenceEntry {
                        ordinal: 1,
                        raw_text: "transient".into(),
                        doi: Some("10.1/a".into()),
                    },
                    outcome: EntryOutcome::Unresolved {
                        doi: "10.1/a".into(),
                        network_error: true,
                    },
                    llm_source: None,
                },
                CheckedEntry {
                    entry: ReferenceEntry {
                        ordinal: 2,
                        raw_text: "missing".into(),
                        doi: Some("10.1/b".into()),
                    },
                    outcome: EntryOutcome::Unresolved {
                        doi: "10.1/b".into(),
                        network_error: false,
                    },
                    llm_source: None,
                },
            ],
        };
        let lines: Vec<String> = to_csv(&r).lines().map(|l| l.to_string()).collect();
        assert_eq!(lines[1], "1,transient,10.1/a,retry_needed,,,");
        assert_eq!(lines[2], "2,missing,10.1/b,not_found,,,");
    }

    #[test]
    fn csv_field_quotes_carriage_return() {
        assert_eq!(csv_field("a\rb"), "\"a\rb\"");
    }

    #[test]
    fn csv_quotes_reference_text_with_commas() {
        let r = CheckResult {
            filename: "x.pdf".into(),
            fingerprint: "fp".into(),
            run_at: "now".into(),
            bibliography_detected: true,
            entries: vec![CheckedEntry {
                entry: ReferenceEntry {
                    ordinal: 1,
                    raw_text: "Smith, J. (2020). Neural things.".into(),
                    doi: Some("10.1/a".into()),
                },
                outcome: EntryOutcome::Resolved {
                    doi: "10.1/a".into(),
                    discrepancies: vec![],
                    from_cache: false,
                },
                llm_source: None,
            }],
        };
        let lines: Vec<String> = to_csv(&r).lines().map(|l| l.to_string()).collect();
        assert_eq!(
            lines[1],
            "1,\"Smith, J. (2020). Neural things.\",10.1/a,clean,,,"
        );
    }

    #[test]
    fn json_round_trips() {
        let json = to_json(&result());
        let back: CheckResult = serde_json::from_str(&json).unwrap();
        assert_eq!(back, result());
    }
}
