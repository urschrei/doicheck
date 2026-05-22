//! Machine-readable exports of a CheckResult: full JSON and a flat CSV.

use crate::model::{CheckResult, EntryOutcome};
use std::fmt::Write;

/// Lossless JSON of the whole result.
pub fn to_json(result: &CheckResult) -> Result<String, serde_json::Error> {
    serde_json::to_string_pretty(result)
}

/// One row per entry: ordinal, reference_text, doi, status, unmatched fields, suggested doi, llm_source.
pub fn to_csv(result: &CheckResult) -> String {
    let mut out = String::from(
        "ordinal,reference_text,doi,status,unmatched_fields,suggested_doi,llm_source\n",
    );
    for ce in &result.entries {
        let (status, unmatched, suggested): (&str, String, String) = match &ce.outcome {
            EntryOutcome::Resolved { discrepancies, .. } => {
                let unmatched = discrepancies
                    .iter()
                    .filter(|d| !d.dismissed)
                    .map(|d| d.field.as_str())
                    .collect::<Vec<_>>()
                    .join("; ");
                let status = if unmatched.is_empty() {
                    "clean"
                } else {
                    "mismatch"
                };
                (status, unmatched, String::new())
            }
            EntryOutcome::Unresolved { network_error, .. } => {
                let status = if *network_error {
                    "retry_needed"
                } else {
                    "not_found"
                };
                (status, String::new(), String::new())
            }
            EntryOutcome::NoDoi { suggested } => (
                "no_doi",
                String::new(),
                suggested
                    .as_ref()
                    .map(|s| s.doi.clone())
                    .unwrap_or_default(),
            ),
        };
        let llm = ce.llm_source.as_deref().unwrap_or("");
        let _ = writeln!(
            out,
            "{},{},{},{},{},{},{}",
            ce.entry.ordinal,
            csv_field(&ce.entry.raw_text),
            csv_field(ce.entry.doi.as_deref().unwrap_or("")),
            status,
            csv_field(&unmatched),
            csv_field(&suggested),
            csv_field(llm),
        );
    }
    out
}

/// Render a CSV field. Quotes per RFC 4180 (comma, quote, CR, LF) and defends
/// against spreadsheet formula injection (CWE-1236): a field beginning with
/// `=`, `+`, `-`, `@`, tab, or CR is interpreted as a formula by Excel /
/// LibreOffice / Sheets, so it is prefixed with an apostrophe to force text.
fn csv_field(s: &str) -> String {
    let needs_formula_guard = s.starts_with(['=', '+', '-', '@', '\t', '\r']);
    if !needs_formula_guard && !s.contains([',', '"', '\n', '\r']) {
        return s.to_string();
    }
    let escaped = s.replace('"', "\"\"");
    let prefix = if needs_formula_guard { "'" } else { "" };
    format!("\"{prefix}{escaped}\"")
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
    fn csv_field_guards_against_formula_injection() {
        assert_eq!(csv_field("=1+1"), "\"'=1+1\"");
        assert_eq!(csv_field("+44"), "\"'+44\"");
        assert_eq!(csv_field("-5"), "\"'-5\"");
        assert_eq!(csv_field("@cmd"), "\"'@cmd\"");
        // A field starting with a safe character is untouched.
        assert_eq!(csv_field("Smith"), "Smith");
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
        let json = to_json(&result()).unwrap();
        let back: CheckResult = serde_json::from_str(&json).unwrap();
        assert_eq!(back, result());
    }
}
