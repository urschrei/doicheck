//! Orchestration: extracted text -> bibliography -> per-entry Crossref checks.

use crate::cache::{DoiCache, QueryKey, SearchCache};
use crate::compare::compare;
use crate::crossref::{CrossrefClient, LookupError};
use crate::model::{CheckResult, CheckedEntry, EntryOutcome, Progress, SuggestedDoi};
use crate::text::token_coverage;
use futures::stream::{self, StreamExt};
use std::collections::HashSet;

const SUGGEST_THRESHOLD: f64 = 0.8;

/// Cache-first resolution: returns an `EntryOutcome` for a single DOI.
/// Used by both `run` and `recheck_failures` to keep resolution consistent.
async fn resolve_doi_outcome(
    doi: &str,
    raw_text: &str,
    client: &CrossrefClient,
    cache: &(impl crate::cache::DoiCache + Sync),
) -> EntryOutcome {
    let key = crate::doi::Doi::new(doi);
    let (json, from_cache) = match cache.get(&key) {
        Some(j) => (Ok(j), true),
        None => {
            let fetched = client.resolve_json(doi).await;
            if let Ok(ref j) = fetched {
                cache.put(&key, j);
            }
            (fetched, false)
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
                from_cache,
            }
        }
        Err(LookupError::NotFound) => EntryOutcome::Unresolved {
            doi: doi.to_string(),
            network_error: false,
        },
        Err(LookupError::Network(_)) => EntryOutcome::Unresolved {
            doi: doi.to_string(),
            network_error: true,
        },
    }
}

/// Per-entry logic: resolve a DOI or search for one, returning an `EntryOutcome`.
async fn outcome_for_entry(
    entry: &crate::model::ReferenceEntry,
    client: &CrossrefClient,
    cache: &(impl DoiCache + SearchCache + Sync),
) -> EntryOutcome {
    match &entry.doi {
        Some(doi) => resolve_doi_outcome(doi, &entry.raw_text, client, cache).await,
        None => {
            // Search is keyed by the reference text. Reuse a cached suggestion
            // before hitting the network.
            let key = QueryKey::new(&entry.raw_text);
            if let Some(json) = cache.search_get(&key) {
                let suggested = serde_json::from_str::<SuggestedDoi>(&json).ok();
                return EntryOutcome::NoDoi {
                    suggested,
                    from_cache: true,
                };
            }
            let suggested = match client.search(&entry.raw_text).await {
                Ok(Some(hit)) if !hit.doi.is_empty() => {
                    // Seed the DOI cache with the matched work's record so a
                    // later direct resolve of this DOI is a cache hit. Done
                    // regardless of the suggestion threshold: the record is the
                    // valid Crossref entry for that DOI either way.
                    cache.put(&crate::doi::Doi::new(&hit.doi), &hit.record);
                    let cov = hit
                        .metadata
                        .title
                        .as_deref()
                        .map(|t| token_coverage(&entry.raw_text, t))
                        .unwrap_or(0.0);
                    if cov >= SUGGEST_THRESHOLD {
                        let suggested = SuggestedDoi {
                            doi: hit.doi,
                            // Clamp to the documented 0-100 range of `title_match`.
                            title_match: (cov * 100.0).round().clamp(0.0, 100.0) as u8,
                        };
                        if let Ok(json) = serde_json::to_string(&suggested) {
                            cache.search_put(&key, &json);
                        }
                        Some(suggested)
                    } else {
                        None
                    }
                }
                // No usable suggestion: an empty-DOI hit, no hit, or a search
                // failure (treated as "no suggestion available").
                Ok(Some(_))
                | Ok(None)
                | Err(LookupError::NotFound)
                | Err(LookupError::Network(_)) => None,
            };
            EntryOutcome::NoDoi {
                suggested,
                from_cache: false,
            }
        }
    }
}

/// Count an outcome's Crossref lookup towards the cache/fetch tallies: a
/// resolved DOI or a no-DOI bibliographic search, each served from cache or
/// fetched. Unresolved entries (failed lookups) contribute to neither.
/// Exhaustive so a new `EntryOutcome` variant forces a decision here.
fn tally(outcome: &EntryOutcome, cached: &mut usize, fetched: &mut usize) {
    match outcome {
        EntryOutcome::Resolved {
            from_cache: true, ..
        }
        | EntryOutcome::NoDoi {
            from_cache: true, ..
        } => *cached += 1,
        EntryOutcome::Resolved {
            from_cache: false, ..
        }
        | EntryOutcome::NoDoi {
            from_cache: false, ..
        } => *fetched += 1,
        EntryOutcome::Unresolved { .. } => {}
    }
}

/// Re-resolve only the entries that previously failed transiently (network /
/// capacity). Operates on a stored result, so it needs no document re-read.
/// Other entries (resolved, not-found, no-DOI) are left unchanged.
pub async fn recheck_failures(
    mut result: CheckResult,
    client: &CrossrefClient,
    cache: &(impl crate::cache::DoiCache + Sync),
    concurrency: usize,
    mut progress: impl FnMut(Progress),
) -> CheckResult {
    let total = result.entries.len();
    // The entries to retry: those that failed transiently. Capture each one's
    // DOI and text here so no later re-derivation (and no fabricated key) is
    // needed. The match is exhaustive, so a new outcome variant forces a choice.
    let jobs: Vec<(usize, String, String)> = result
        .entries
        .iter()
        .enumerate()
        .filter_map(|(i, ce)| match &ce.outcome {
            EntryOutcome::Unresolved {
                network_error: true,
                doi,
            } => Some((i, doi.clone(), ce.entry.raw_text.clone())),
            EntryOutcome::Unresolved {
                network_error: false,
                ..
            }
            | EntryOutcome::Resolved { .. }
            | EntryOutcome::NoDoi { .. } => None,
        })
        .collect();

    // Tally the cache/fetch source of already-resolved entries. Entries being
    // retried are Unresolved and contribute nothing to either count.
    let mut cached = 0usize;
    let mut fetched = 0usize;
    for ce in &result.entries {
        tally(&ce.outcome, &mut cached, &mut fetched);
    }
    let mut done = total - jobs.len();
    progress(Progress {
        done,
        total,
        cached,
        fetched,
    });

    let mut tasks = stream::iter(jobs.into_iter().map(|(i, doi, raw_text)| async move {
        (i, resolve_doi_outcome(&doi, &raw_text, client, cache).await)
    }))
    .buffer_unordered(concurrency.max(1));
    while let Some((i, outcome)) = tasks.next().await {
        tally(&outcome, &mut cached, &mut fetched);
        done += 1;
        progress(Progress {
            done,
            total,
            cached,
            fetched,
        });
        result.entries[i].outcome = outcome;
    }
    for ce in &mut result.entries {
        ce.llm_source = crate::integrity::llm_source(&ce.entry.raw_text);
    }
    result
}

/// Run the checks over already-extracted document text.
#[allow(clippy::too_many_arguments)]
pub async fn run(
    filename: String,
    fingerprint: String,
    run_at: String,
    text: &str,
    client: &CrossrefClient,
    cache: &(impl DoiCache + SearchCache + Sync),
    concurrency: usize,
    mut progress: impl FnMut(Progress),
) -> CheckResult {
    let bib = crate::biblio::detect(text);
    let detected = bib.detected;
    let entries = bib.entries;
    let total = entries.len();

    // Partition entries so each unique DOI is fetched once: first occurrence of
    // a DOI (and all no-DOI entries) in pass 1; later repeats in pass 2, served
    // from the cache the first occurrence populates.
    let mut seen: HashSet<String> = HashSet::new();
    let mut first_pass: Vec<usize> = Vec::new();
    let mut dup_pass: Vec<usize> = Vec::new();
    for (i, e) in entries.iter().enumerate() {
        match &e.doi {
            Some(doi) if !seen.insert(doi.clone()) => dup_pass.push(i),
            _ => first_pass.push(i),
        }
    }

    let mut outcomes: Vec<Option<EntryOutcome>> = (0..total).map(|_| None).collect();
    let mut done = 0usize;
    let mut cached = 0usize;
    let mut fetched = 0usize;
    let limit = concurrency.max(1);

    for indices in [first_pass, dup_pass] {
        let mut tasks = stream::iter(indices.into_iter().map(|i| {
            let entry = entries[i].clone();
            async move { (i, outcome_for_entry(&entry, client, cache).await) }
        }))
        .buffer_unordered(limit);
        while let Some((i, outcome)) = tasks.next().await {
            tally(&outcome, &mut cached, &mut fetched);
            done += 1;
            progress(Progress {
                done,
                total,
                cached,
                fetched,
            });
            outcomes[i] = Some(outcome);
        }
    }

    let checked: Vec<CheckedEntry> = entries
        .into_iter()
        .enumerate()
        .map(|(i, entry)| {
            let llm_source = crate::integrity::llm_source(&entry.raw_text);
            CheckedEntry {
                entry,
                // Safe: every index 0..total is placed in exactly one of the two
                // passes above and filled by the corresponding stream loop.
                outcome: outcomes[i].take().expect("every entry produced an outcome"),
                llm_source,
            }
        })
        .collect();

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
            5,
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
            5,
            |_| {},
        )
        .await;

        match &result.entries[0].outcome {
            EntryOutcome::NoDoi {
                suggested: Some(s), ..
            } => {
                assert_eq!(s.doi, "10.1000/xyz");
                assert!(s.title_match >= 80);
            }
            other => panic!("expected a suggestion, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn search_suggestion_is_cached_and_reused() {
        let server = MockServer::start().await;
        let body = serde_json::json!({
            "message": { "items": [{ "title": ["A Study of Widgets"], "DOI": "10.1000/xyz" }]}
        });
        // Respond to the search exactly once; a second search would 404.
        Mock::given(method("GET"))
            .and(query_param("rows", "1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        let client = CrossrefClient::with_base("", server.uri());
        let cache = MemoryCache::default();
        let text = "References\nSmith J. A Study of Widgets. Journal of Widgets.";

        let first = run(
            "a.pdf".into(),
            "fp".into(),
            "now".into(),
            text,
            &client,
            &cache,
            5,
            |_| {},
        )
        .await;
        assert!(matches!(
            &first.entries[0].outcome,
            EntryOutcome::NoDoi {
                suggested: Some(_),
                ..
            }
        ));

        // Second run: the search mock is exhausted, so a fresh search would fail.
        // The suggestion must come from the search cache instead.
        let second = run(
            "a.pdf".into(),
            "fp".into(),
            "now".into(),
            text,
            &client,
            &cache,
            5,
            |_| {},
        )
        .await;
        match &second.entries[0].outcome {
            EntryOutcome::NoDoi {
                suggested: Some(s),
                from_cache,
            } => {
                assert_eq!(s.doi, "10.1000/xyz");
                assert!(
                    *from_cache,
                    "the reused suggestion must be marked from_cache"
                );
            }
            other => panic!("expected cached suggestion, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn search_hit_seeds_the_doi_cache() {
        let server = MockServer::start().await;
        let body = serde_json::json!({
            "message": { "items": [{
                "title": ["A Study of Widgets"],
                "author": [{"family": "Smith"}],
                "issued": {"date-parts": [[2020]]},
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
        let _ = run(
            "a.pdf".into(),
            "fp".into(),
            "now".into(),
            text,
            &client,
            &cache,
            5,
            |_| {},
        )
        .await;

        // The matched work's record is now in the DOI cache under its DOI, so a
        // later direct resolve is a cache hit with usable metadata.
        let seeded = cache.get(&crate::doi::Doi::new("10.1000/xyz"));
        assert!(seeded.is_some(), "search hit should seed the DOI cache");
        let meta = crate::crossref::metadata_from_json(&seeded.unwrap());
        assert_eq!(meta.title.as_deref(), Some("A Study of Widgets"));
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
            5,
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
            &crate::doi::Doi::new("10.1000/abc"),
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
            5,
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
            5,
            |_| {},
        )
        .await;
        assert!(cache.get(&crate::doi::Doi::new("10.1000/abc")).is_some());
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
            5,
            |_| {},
        )
        .await;
        match &result.entries[0].outcome {
            EntryOutcome::Unresolved { network_error, .. } => assert!(*network_error),
            other => panic!("expected transient Unresolved, got {other:?}"),
        }
        assert!(cache.get(&crate::doi::Doi::new("10.1000/abc")).is_none());
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
                    llm_source: None,
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
                        from_cache: false,
                    },
                    llm_source: None,
                },
            ],
        };

        let updated = recheck_failures(result, &client, &cache, 5, |_| {}).await;
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

    #[tokio::test]
    async fn duplicate_doi_is_fetched_once() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path_regex(r"/works/.*"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "message": { "title": ["A Study of Widgets"], "DOI": "10.1000/dup" }
            })))
            .expect(1)
            .mount(&server)
            .await;
        let client = CrossrefClient::with_base("", server.uri());
        let cache = MemoryCache::default();
        let text = "References\n\
[1] Smith J (2020). A Study of Widgets. 10.1000/dup\n\
[2] Smith J (2020). A Study of Widgets, reprinted. 10.1000/dup";
        let result = run(
            "a.pdf".into(),
            "fp".into(),
            "now".into(),
            text,
            &client,
            &cache,
            5,
            |_| {},
        )
        .await;
        assert_eq!(result.entries.len(), 2);
        assert!(
            result
                .entries
                .iter()
                .all(|e| matches!(e.outcome, EntryOutcome::Resolved { .. }))
        );
        // `.expect(1)` on the mock asserts a single Crossref call when `server` drops.
    }

    #[tokio::test]
    async fn llm_source_flag_set_for_chatgpt_utm_parameter() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(query_param("rows", "1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "message": { "items": [] }
            })))
            .mount(&server)
            .await;
        let client = CrossrefClient::with_base("", server.uri());
        let cache = MemoryCache::default();

        // A reference whose raw text contains a ChatGPT tracking parameter.
        let text = "References\nSmith J (2024). A Study. \
            https://example.com/x.pdf?utm_source=chatgpt.com";
        let result = run(
            "a.pdf".into(),
            "fp".into(),
            "now".into(),
            text,
            &client,
            &cache,
            5,
            |_| {},
        )
        .await;

        assert_eq!(result.entries.len(), 1);
        assert_eq!(
            result.entries[0].llm_source.as_deref(),
            Some("utm_source=chatgpt.com")
        );
    }
}
