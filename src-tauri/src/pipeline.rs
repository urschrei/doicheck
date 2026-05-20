//! Orchestration: extracted text -> bibliography -> per-entry Crossref checks.

use crate::compare::compare;
use crate::crossref::{CrossrefClient, CrossrefError};
use crate::model::{CheckResult, CheckedEntry, EntryOutcome, Progress, SuggestedDoi};
use crate::text::token_coverage;

const SUGGEST_THRESHOLD: f64 = 0.8;

/// Run the checks over already-extracted document text.
pub async fn run(
    filename: String,
    fingerprint: String,
    run_at: String,
    text: &str,
    client: &CrossrefClient,
    mut progress: impl FnMut(Progress),
) -> CheckResult {
    let bib = crate::biblio::detect(text);
    let detected = bib.detected;
    let raw_entries = bib.entries;

    let total = raw_entries.len();
    let mut checked = Vec::with_capacity(total);
    for (i, entry) in raw_entries.into_iter().enumerate() {
        let outcome = match &entry.doi {
            Some(doi) => match client.resolve(doi).await {
                Ok(meta) => {
                    let discrepancies = if crate::text::is_comparable(&entry.raw_text) {
                        compare(&entry.raw_text, &meta)
                    } else {
                        Vec::new()
                    };
                    EntryOutcome::Resolved {
                        doi: doi.clone(),
                        discrepancies,
                    }
                }
                Err(CrossrefError::NotFound) => EntryOutcome::Unresolved {
                    doi: doi.clone(),
                    network_error: false,
                },
                Err(CrossrefError::Network(_)) => EntryOutcome::Unresolved {
                    doi: doi.clone(),
                    network_error: true,
                },
            },
            None => {
                let suggested = match client.search(&entry.raw_text).await {
                    Ok(Some(hit)) if !hit.doi.is_empty() => {
                        let cov = hit
                            .metadata
                            .title
                            .as_deref()
                            .map(|t| token_coverage(&entry.raw_text, t))
                            .unwrap_or(0.0);
                        if cov >= SUGGEST_THRESHOLD {
                            Some(SuggestedDoi {
                                doi: hit.doi,
                                title_match: (cov * 100.0).round() as u8,
                            })
                        } else {
                            None
                        }
                    }
                    _ => None,
                };
                EntryOutcome::NoDoi { suggested }
            }
        };
        checked.push(CheckedEntry { entry, outcome });
        progress(Progress { done: i + 1, total });
    }

    CheckResult {
        filename,
        fingerprint,
        run_at,
        bibliography_detected: detected,
        entries: checked,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path_regex, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn resolves_doi_entry_and_reports_progress() {
        let server = MockServer::start().await;
        let body = serde_json::json!({
            "message": {
                "title": ["A Study of Widgets"],
                "author": [{"family": "Smith"}],
                "issued": {"date-parts": [[2020]]},
                "DOI": "10.1000/abc"
            }
        });
        Mock::given(method("GET"))
            .and(path_regex(r"/works/.*"))
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .mount(&server)
            .await;
        let client = CrossrefClient::with_base("", server.uri());

        let text = "References\n[1] Smith J (2020). A Study of Widgets. 10.1000/abc";
        let mut updates = Vec::new();
        let result = run(
            "a.pdf".into(),
            "fp".into(),
            "now".into(),
            text,
            &client,
            |p| updates.push(p.done),
        )
        .await;

        assert!(result.bibliography_detected);
        assert_eq!(result.entries.len(), 1);
        assert!(matches!(
            result.entries[0].outcome,
            EntryOutcome::Resolved { .. }
        ));
        assert_eq!(updates, vec![1]);
    }

    #[tokio::test]
    async fn suggests_doi_for_entry_without_one() {
        let server = MockServer::start().await;
        let body = serde_json::json!({
            "message": { "items": [{
                "title": ["A Study of Widgets"],
                "DOI": "10.1000/xyz"
            }]}
        });
        Mock::given(method("GET"))
            .and(query_param("rows", "1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .mount(&server)
            .await;
        let client = CrossrefClient::with_base("", server.uri());

        let text = "References\nSmith J. A Study of Widgets. Journal of Widgets.";
        let result = run(
            "a.pdf".into(),
            "fp".into(),
            "now".into(),
            text,
            &client,
            |_| {},
        )
        .await;

        match &result.entries[0].outcome {
            EntryOutcome::NoDoi { suggested: Some(s) } => {
                assert_eq!(s.doi, "10.1000/xyz");
                assert!(s.title_match >= 80);
            }
            other => panic!("expected a suggestion, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn no_heading_uses_window_text_for_comparison() {
        let server = MockServer::start().await;
        let body = serde_json::json!({
            "message": {
                "title": ["A Study of Widgets"],
                "author": [{"family": "Smith"}],
                "issued": {"date-parts": [[2020]]},
                "DOI": "10.1000/abc"
            }
        });
        Mock::given(method("GET"))
            .and(path_regex(r"/works/.*"))
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .mount(&server)
            .await;
        let client = CrossrefClient::with_base("", server.uri());

        // No "References" heading: the fallback window must carry the entry text,
        // so the matching metadata yields NO discrepancies (not a false positive).
        let text = "Smith, J. (2020). A Study of Widgets. Journal. https://doi.org/10.1000/abc";
        let result = run(
            "a.pdf".into(),
            "fp".into(),
            "now".into(),
            text,
            &client,
            |_| {},
        )
        .await;

        assert!(!result.bibliography_detected);
        assert_eq!(result.entries.len(), 1);
        match &result.entries[0].outcome {
            EntryOutcome::Resolved { discrepancies, .. } => assert!(discrepancies.is_empty()),
            other => panic!("expected Resolved, got {other:?}"),
        }
    }
}
