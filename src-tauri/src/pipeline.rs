//! Orchestration: extracted text -> bibliography -> per-entry Crossref checks.

use crate::cache::DoiCache;
use crate::compare::compare;
use crate::crossref::{CrossrefClient, CrossrefError};
use crate::model::{CheckResult, CheckedEntry, EntryOutcome, Progress, SuggestedDoi};
use crate::text::token_coverage;

const SUGGEST_THRESHOLD: f64 = 0.8;

/// Cache-first resolution: returns an `EntryOutcome` for a single DOI.
/// Used by both `run` and `recheck_failures` to keep resolution consistent.
async fn resolve_doi_outcome(
    doi: &str,
    raw_text: &str,
    client: &CrossrefClient,
    cache: &(impl crate::cache::DoiCache + Sync),
) -> EntryOutcome {
    let json = match cache.get(doi) {
        Some(j) => Ok(j),
        None => {
            let fetched = client.resolve_json(doi).await;
            if let Ok(ref j) = fetched {
                cache.put(doi, j);
            }
            fetched
        }
    };
    match json {
        Ok(body) => {
            let meta = crate::crossref::metadata_from_json(&body);
            let discrepancies = if crate::text::is_comparable(raw_text) {
                compare(raw_text, &meta)
            } else {
                Vec::new()
            };
            EntryOutcome::Resolved {
                doi: doi.to_string(),
                discrepancies,
            }
        }
        Err(CrossrefError::NotFound) => EntryOutcome::Unresolved {
            doi: doi.to_string(),
            network_error: false,
        },
        Err(CrossrefError::Network(_)) => EntryOutcome::Unresolved {
            doi: doi.to_string(),
            network_error: true,
        },
    }
}

/// Re-resolve only the entries that previously failed transiently (network /
/// capacity). Operates on a stored result, so it needs no document re-read.
/// Other entries (resolved, not-found, no-DOI) are left unchanged.
pub async fn recheck_failures(
    mut result: CheckResult,
    client: &CrossrefClient,
    cache: &(impl crate::cache::DoiCache + Sync),
    mut progress: impl FnMut(Progress),
) -> CheckResult {
    let total = result.entries.len();
    for (i, ce) in result.entries.iter_mut().enumerate() {
        let retry_doi = match &ce.outcome {
            EntryOutcome::Unresolved {
                doi,
                network_error: true,
            } => Some(doi.clone()),
            _ => None,
        };
        if let Some(doi) = retry_doi {
            ce.outcome = resolve_doi_outcome(&doi, &ce.entry.raw_text, client, cache).await;
        }
        progress(Progress { done: i + 1, total });
    }
    result
}

/// Run the checks over already-extracted document text.
pub async fn run(
    filename: String,
    fingerprint: String,
    run_at: String,
    text: &str,
    client: &CrossrefClient,
    cache: &(impl DoiCache + Sync),
    mut progress: impl FnMut(Progress),
) -> CheckResult {
    let bib = crate::biblio::detect(text);
    let detected = bib.detected;
    let raw_entries = bib.entries;

    let total = raw_entries.len();
    let mut checked = Vec::with_capacity(total);
    for (i, entry) in raw_entries.into_iter().enumerate() {
        let outcome = match &entry.doi {
            Some(doi) => resolve_doi_outcome(doi, &entry.raw_text, client, cache).await,
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
    use crate::cache::MemoryCache;
    use crate::model::{CheckResult, CheckedEntry, ReferenceEntry};
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
        let cache = MemoryCache::default();

        let text = "References\n[1] Smith J (2020). A Study of Widgets. 10.1000/abc";
        let mut updates = Vec::new();
        let result = run(
            "a.pdf".into(),
            "fp".into(),
            "now".into(),
            text,
            &client,
            &cache,
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
        let cache = MemoryCache::default();

        let text = "References\nSmith J. A Study of Widgets. Journal of Widgets.";
        let result = run(
            "a.pdf".into(),
            "fp".into(),
            "now".into(),
            text,
            &client,
            &cache,
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
        let cache = MemoryCache::default();

        // No "References" heading: the fallback window must carry the entry text,
        // so the matching metadata yields NO discrepancies (not a false positive).
        let text = "Smith, J. (2020). A Study of Widgets. Journal. https://doi.org/10.1000/abc";
        let result = run(
            "a.pdf".into(),
            "fp".into(),
            "now".into(),
            text,
            &client,
            &cache,
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

    #[tokio::test]
    async fn cache_hit_skips_network() {
        // No mocks mounted: any real request would 404 -> Unresolved. A cache
        // hit must yield Resolved without touching the network.
        let server = MockServer::start().await;
        let cache = MemoryCache::default();
        cache.put(
            "10.1000/abc",
            &serde_json::json!({"message":{"title":["Cached"],"DOI":"10.1000/abc"}}).to_string(),
        );
        let client = CrossrefClient::with_base("", server.uri());
        let text = "References\n[1] Smith J (2020). A Study of Widgets. 10.1000/abc";
        let result = run(
            "a.pdf".into(),
            "fp".into(),
            "now".into(),
            text,
            &client,
            &cache,
            |_| {},
        )
        .await;
        assert!(matches!(
            result.entries[0].outcome,
            EntryOutcome::Resolved { .. }
        ));
    }

    #[tokio::test]
    async fn successful_resolve_populates_cache() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path_regex(r"/works/.*"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "message": { "title": ["A Study of Widgets"], "DOI": "10.1000/abc" }
            })))
            .mount(&server)
            .await;
        let client = CrossrefClient::with_base("", server.uri());
        let cache = MemoryCache::default();
        let text = "References\n[1] Smith J (2020). A Study of Widgets. 10.1000/abc";
        let _ = run(
            "a.pdf".into(),
            "fp".into(),
            "now".into(),
            text,
            &client,
            &cache,
            |_| {},
        )
        .await;
        assert!(cache.get("10.1000/abc").is_some());
    }

    #[tokio::test]
    async fn transient_failure_is_not_cached() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path_regex(r"/works/.*"))
            .respond_with(ResponseTemplate::new(503))
            .mount(&server)
            .await;
        let client = CrossrefClient::with_base("", server.uri());
        let cache = MemoryCache::default();
        let text = "References\n[1] Smith J (2020). A Study of Widgets. 10.1000/abc";
        let result = run(
            "a.pdf".into(),
            "fp".into(),
            "now".into(),
            text,
            &client,
            &cache,
            |_| {},
        )
        .await;
        match &result.entries[0].outcome {
            EntryOutcome::Unresolved { network_error, .. } => assert!(*network_error),
            other => panic!("expected transient Unresolved, got {other:?}"),
        }
        assert!(cache.get("10.1000/abc").is_none());
    }

    #[tokio::test]
    async fn recheck_failures_only_retries_transient() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path_regex(r"/works/.*"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "message": { "title": ["A Study of Widgets"], "DOI": "10.1000/fail" }
            })))
            .mount(&server)
            .await;
        let client = CrossrefClient::with_base("", server.uri());
        let cache = MemoryCache::default();

        let result = CheckResult {
            filename: "a.pdf".into(),
            fingerprint: "fp".into(),
            run_at: "now".into(),
            bibliography_detected: true,
            entries: vec![
                CheckedEntry {
                    entry: ReferenceEntry {
                        ordinal: 1,
                        raw_text: "Smith (2020). A Study of Widgets.".into(),
                        doi: Some("10.1000/fail".into()),
                    },
                    outcome: EntryOutcome::Unresolved {
                        doi: "10.1000/fail".into(),
                        network_error: true,
                    },
                },
                CheckedEntry {
                    entry: ReferenceEntry {
                        ordinal: 2,
                        raw_text: "x".into(),
                        doi: Some("10.1000/ok".into()),
                    },
                    outcome: EntryOutcome::Resolved {
                        doi: "10.1000/ok".into(),
                        discrepancies: vec![],
                    },
                },
            ],
        };

        let updated = recheck_failures(result, &client, &cache, |_| {}).await;
        // The transient failure is now resolved.
        assert!(matches!(
            updated.entries[0].outcome,
            EntryOutcome::Resolved { .. }
        ));
        // The previously-resolved entry is untouched (no network call for it).
        assert!(matches!(
            updated.entries[1].outcome,
            EntryOutcome::Resolved { .. }
        ));
    }
}
