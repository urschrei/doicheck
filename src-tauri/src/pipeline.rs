//! Orchestration: extracted text -> bibliography -> per-entry Crossref checks.

use crate::cache::{DoiCache, QueryKey, SearchCache};
use crate::compare::{Metadata, compare};
use crate::crossref::{CrossrefClient, LookupError, SearchHit};
use crate::datacite::DataCiteClient;
use crate::model::{
    CheckResult, CheckedEntry, EntryOutcome, Progress, Registration, Source, SuggestedDoi,
};
use crate::registration::RegistrationClient;
use crate::text::token_coverage;
use futures::stream::{self, StreamExt};
use std::collections::HashSet;
use std::future::Future;

const SUGGEST_THRESHOLD: f64 = 0.8;

/// Cache-first fetch from one agency: returns the JSON and whether it was a cache
/// hit. The `fetch` future is only awaited on a cache miss (futures are lazy), so
/// a hit makes no network call.
async fn fetch_cached(
    cache: &(impl DoiCache + Sync),
    source: Source,
    key: &crate::doi::Doi,
    fetch: impl Future<Output = Result<String, LookupError>>,
) -> Result<(String, bool), LookupError> {
    if let Some(json) = cache.get(source, key) {
        return Ok((json, true));
    }
    let json = fetch.await?;
    cache.put(source, key, &json);
    Ok((json, false))
}

/// Build a Resolved outcome, comparing the reference text against `meta` (unless
/// the text is too sparse to compare).
fn resolved_outcome(
    doi: &str,
    raw_text: &str,
    meta: &Metadata,
    from_cache: bool,
    source: Source,
    via_search: bool,
) -> EntryOutcome {
    let discrepancies = if crate::text::is_comparable(raw_text) {
        compare(raw_text, meta)
    } else {
        Vec::new()
    };
    EntryOutcome::Resolved {
        doi: doi.to_string(),
        discrepancies,
        from_cache,
        source,
        via_search,
    }
}

fn unresolved_outcome(doi: &str, network_error: bool) -> EntryOutcome {
    EntryOutcome::Unresolved {
        doi: doi.to_string(),
        network_error,
        registration: Registration::Unknown,
        suggested: None,
    }
}

/// Turn a bibliographic-search hit into a suggestion: seed `source`'s DOI cache
/// with the matched record (so a later direct resolve is a cache hit, regardless
/// of the threshold), and return a `SuggestedDoi` when the title coverage meets
/// the suggestion threshold.
fn suggestion_from_hit(
    hit: SearchHit,
    raw_text: &str,
    source: Source,
    cache: &impl DoiCache,
) -> Option<SuggestedDoi> {
    cache.put(source, &crate::doi::Doi::new(&hit.doi), &hit.record);
    let cov = hit
        .metadata
        .title
        .as_deref()
        .map(|t| token_coverage(raw_text, t))
        .unwrap_or(0.0);
    (cov >= SUGGEST_THRESHOLD).then(|| SuggestedDoi {
        doi: hit.doi,
        // Clamp to the documented 0-100 range of `title_match`.
        title_match: (cov * 100.0).round().clamp(0.0, 100.0) as u8,
        source,
    })
}

/// Read a matched record from the DOI cache and parse it to comparison
/// metadata, choosing the parser for the agency the record came from.
fn cached_metadata(cache: &impl DoiCache, source: Source, doi: &str) -> Option<Metadata> {
    let json = cache.get(source, &crate::doi::Doi::new(doi))?;
    Some(match source {
        Source::Crossref => crate::crossref::metadata_from_json(&json),
        Source::DataCite => crate::datacite::metadata_from_json(&json),
    })
}

/// Decide the outcome for a no-DOI entry from its best search suggestion. A
/// suggestion whose title is fully present in the reference (strict 100% token
/// coverage) is promoted to a `Resolved` via-search outcome, comparing the
/// matched record's metadata; otherwise it stays a `NoDoi` suggestion. The
/// matched record was seeded into the DOI cache by `suggestion_from_hit`; if it
/// is absent (e.g. expired) the entry degrades to a suggestion.
fn finalise_no_doi(
    suggested: Option<SuggestedDoi>,
    search_from_cache: bool,
    raw_text: &str,
    cache: &impl DoiCache,
) -> EntryOutcome {
    if let Some(sug) = &suggested
        && let Some(meta) = cached_metadata(cache, sug.source, &sug.doi)
        && let Some(title) = meta.title.as_deref()
        && token_coverage(raw_text, title) >= 1.0
    {
        return resolved_outcome(
            &sug.doi,
            raw_text,
            &meta,
            search_from_cache,
            sug.source,
            true,
        );
    }
    EntryOutcome::NoDoi {
        suggested,
        from_cache: search_from_cache,
    }
}

/// Cache-first resolution for a single DOI: Crossref first, then DataCite when
/// Crossref returns a definitive 404 (the DOI is registered with another agency).
/// A Crossref *network* error stays transient (retry later) and does not fall
/// through, since the DOI may well be a Crossref one we simply could not fetch.
/// Used by both `run` and `recheck_failures` to keep resolution consistent.
async fn resolve_doi_outcome(
    doi: &str,
    raw_text: &str,
    client: &CrossrefClient,
    datacite: &DataCiteClient,
    registration: &RegistrationClient,
    cache: &(impl DoiCache + SearchCache + Sync),
) -> EntryOutcome {
    let key = crate::doi::Doi::new(doi);
    match fetch_cached(cache, Source::Crossref, &key, client.resolve_json(doi)).await {
        Ok((body, from_cache)) => {
            let meta = crate::crossref::metadata_from_json(&body);
            return resolved_outcome(doi, raw_text, &meta, from_cache, Source::Crossref, false);
        }
        Err(LookupError::Network(_)) => return unresolved_outcome(doi, true),
        Err(LookupError::NotFound) => {}
    }
    match fetch_cached(cache, Source::DataCite, &key, datacite.resolve_json(doi)).await {
        Ok((body, from_cache)) => {
            let meta = crate::datacite::metadata_from_json(&body);
            resolved_outcome(doi, raw_text, &meta, from_cache, Source::DataCite, false)
        }
        Err(LookupError::Network(_)) => unresolved_outcome(doi, true),
        // Neither agency has the DOI. Diagnose: ask doi.org whether the DOI is
        // registered at all (telling a non-DOI miscast as one from a valid DOI
        // those two agencies do not index), and search by reference text for the
        // record it should point to. A registration-check failure leaves the
        // status `Unknown` rather than wrongly claiming the DOI is unregistered.
        Err(LookupError::NotFound) => {
            let reg = registration
                .check(doi)
                .await
                .unwrap_or(Registration::Unknown);
            let (suggested, _) = search_suggestion(raw_text, client, datacite, cache).await;
            EntryOutcome::Unresolved {
                doi: doi.to_string(),
                network_error: false,
                registration: reg,
                suggested,
            }
        }
    }
}

/// Cache-first bibliographic search for a reference's text across Crossref then
/// DataCite, returning the best suggestion (when title coverage meets the
/// threshold) and whether it was served from the search cache. Each agency's hit
/// seeds its own DOI cache via `suggestion_from_hit`. Shared by the no-DOI path
/// and the unresolved-DOI path, which both ask "what record does this reference
/// describe?".
async fn search_suggestion(
    raw_text: &str,
    client: &CrossrefClient,
    datacite: &DataCiteClient,
    cache: &(impl DoiCache + SearchCache + Sync),
) -> (Option<SuggestedDoi>, bool) {
    let key = QueryKey::new(raw_text);
    if let Some(json) = cache.search_get(&key) {
        return (serde_json::from_str::<SuggestedDoi>(&json).ok(), true);
    }
    // Crossref first; if it offers no suggestion (no good match, no hit, or a
    // search failure), fall back to DataCite.
    let crossref_suggestion = match client.search(raw_text).await {
        Ok(Some(hit)) if !hit.doi.is_empty() => {
            suggestion_from_hit(hit, raw_text, Source::Crossref, cache)
        }
        Ok(Some(_)) | Ok(None) | Err(LookupError::NotFound) | Err(LookupError::Network(_)) => None,
    };
    let suggested = match crossref_suggestion {
        Some(s) => Some(s),
        None => match datacite.search(raw_text).await {
            Ok(Some(hit)) if !hit.doi.is_empty() => {
                suggestion_from_hit(hit, raw_text, Source::DataCite, cache)
            }
            Ok(Some(_)) | Ok(None) | Err(LookupError::NotFound) | Err(LookupError::Network(_)) => {
                None
            }
        },
    };
    // Cache only a positive suggestion (>=80% match); a near-miss is left
    // uncached so a later run can try again.
    if let Some(s) = &suggested
        && let Ok(json) = serde_json::to_string(s)
    {
        cache.search_put(&key, &json);
    }
    (suggested, false)
}

/// Per-entry logic: resolve a DOI or search for one, returning an `EntryOutcome`.
async fn outcome_for_entry(
    entry: &crate::model::ReferenceEntry,
    client: &CrossrefClient,
    datacite: &DataCiteClient,
    registration: &RegistrationClient,
    cache: &(impl DoiCache + SearchCache + Sync),
) -> EntryOutcome {
    match &entry.doi {
        Some(doi) => {
            resolve_doi_outcome(doi, &entry.raw_text, client, datacite, registration, cache).await
        }
        None => {
            let (suggested, from_cache) =
                search_suggestion(&entry.raw_text, client, datacite, cache).await;
            finalise_no_doi(suggested, from_cache, &entry.raw_text, cache)
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
    datacite: &DataCiteClient,
    registration: &RegistrationClient,
    cache: &(impl DoiCache + SearchCache + Sync),
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
                ..
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
        (
            i,
            resolve_doi_outcome(&doi, &raw_text, client, datacite, registration, cache).await,
        )
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
    datacite: &DataCiteClient,
    registration: &RegistrationClient,
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
            async move {
                (
                    i,
                    outcome_for_entry(&entry, client, datacite, registration, cache).await,
                )
            }
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
        let datacite = DataCiteClient::with_base("", server.uri());
        let registration = RegistrationClient::with_base("", server.uri());
        let cache = MemoryCache::default();

        let text = "References\n[1] Smith J (2020). A Study of Widgets. 10.1000/abc";
        let mut updates = Vec::new();
        let result = run(
            "a.pdf".into(),
            "fp".into(),
            "now".into(),
            text,
            &client,
            &datacite,
            &registration,
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
    async fn falls_back_to_datacite_when_crossref_404s() {
        // Crossref does not have the DOI; DataCite does (e.g. a Zenodo dataset).
        let crossref = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&crossref)
            .await;
        let datacite_server = MockServer::start().await;
        let body = serde_json::json!({
            "data": {"attributes": {
                "doi": "10.5281/zenodo.99",
                "titles": [{"title": "A Dataset"}],
                "creators": [{"name": "Lee, K", "familyName": "Lee", "nameType": "Personal"}],
                "publicationYear": 2021
            }}
        });
        Mock::given(method("GET"))
            .and(path_regex(r"/dois/.*"))
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .mount(&datacite_server)
            .await;
        let client = CrossrefClient::with_base("", crossref.uri());
        let datacite = DataCiteClient::with_base("", datacite_server.uri());
        let registration = RegistrationClient::with_base("", datacite_server.uri());
        let cache = MemoryCache::default();

        let text = "References\nLee, K. (2021). A Dataset. https://doi.org/10.5281/zenodo.99";
        let result = run(
            "a.pdf".into(),
            "fp".into(),
            "now".into(),
            text,
            &client,
            &datacite,
            &registration,
            &cache,
            5,
            |_| {},
        )
        .await;
        match &result.entries[0].outcome {
            EntryOutcome::Resolved { source, doi, .. } => {
                assert_eq!(*source, Source::DataCite);
                assert_eq!(doi, "10.5281/zenodo.99");
            }
            other => panic!("expected DataCite-resolved, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn unresolved_when_neither_agency_has_the_doi() {
        // Both agencies 404: the DOI is genuinely unresolved (not a network error).
        let crossref = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&crossref)
            .await;
        let datacite_server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&datacite_server)
            .await;
        let client = CrossrefClient::with_base("", crossref.uri());
        let datacite = DataCiteClient::with_base("", datacite_server.uri());
        let registration = RegistrationClient::with_base("", datacite_server.uri());
        let cache = MemoryCache::default();

        let text = "References\nGhost, A. (2099). Nonexistent. https://doi.org/10.9999/nope";
        let result = run(
            "a.pdf".into(),
            "fp".into(),
            "now".into(),
            text,
            &client,
            &datacite,
            &registration,
            &cache,
            5,
            |_| {},
        )
        .await;
        assert!(matches!(
            result.entries[0].outcome,
            EntryOutcome::Unresolved {
                network_error: false,
                ..
            }
        ));
    }

    /// A server that 404s both resolve endpoints, answers the doi.org
    /// registration check with `ra`, and returns `search` for a bibliographic
    /// search. One server stands in for Crossref, DataCite, and doi.org, since
    /// each uses a distinct path.
    async fn unresolved_server(ra: serde_json::Value, search: serde_json::Value) -> MockServer {
        let srv = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path_regex(r"/works/.+"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&srv)
            .await;
        Mock::given(method("GET"))
            .and(path_regex(r"/dois/.+"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&srv)
            .await;
        Mock::given(method("GET"))
            .and(path_regex(r"/doiRA/.+"))
            .respond_with(ResponseTemplate::new(200).set_body_json(ra))
            .mount(&srv)
            .await;
        Mock::given(method("GET"))
            .and(query_param("rows", "1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(search))
            .mount(&srv)
            .await;
        srv
    }

    async fn unresolved_outcome_for(srv: &MockServer, text: &str) -> EntryOutcome {
        let client = CrossrefClient::with_base("", srv.uri());
        let datacite = DataCiteClient::with_base("", srv.uri());
        let registration = RegistrationClient::with_base("", srv.uri());
        let cache = MemoryCache::default();
        let result = run(
            "a.pdf".into(),
            "fp".into(),
            "now".into(),
            text,
            &client,
            &datacite,
            &registration,
            &cache,
            5,
            |_| {},
        )
        .await;
        result.entries.into_iter().next().unwrap().outcome
    }

    #[tokio::test]
    async fn unregistered_doi_with_no_match_is_flagged() {
        let srv = unresolved_server(
            serde_json::json!([{ "status": "DOI does not exist" }]),
            serde_json::json!({ "message": { "items": [] } }),
        )
        .await;
        let text = "References\nGhost A. Nonexistent thing. https://doi.org/10.9999/nope";
        match unresolved_outcome_for(&srv, text).await {
            EntryOutcome::Unresolved {
                network_error,
                registration,
                suggested,
                ..
            } => {
                assert!(!network_error);
                assert_eq!(registration, Registration::Unregistered);
                assert!(suggested.is_none());
            }
            other => panic!("expected unresolved, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn unregistered_doi_with_a_title_match_offers_the_match() {
        let srv = unresolved_server(
            serde_json::json!([{ "status": "DOI does not exist" }]),
            serde_json::json!({ "message": { "items": [
                { "title": ["A Study of Widgets"], "DOI": "10.1000/xyz" }
            ]}}),
        )
        .await;
        // The cited DOI is broken but the reference's title matches a real record,
        // so the correct DOI is offered rather than the entry being substituted.
        let text = "References\nSmith J. A Study of Widgets. Journal of Widgets. https://doi.org/10.9999/nope";
        match unresolved_outcome_for(&srv, text).await {
            EntryOutcome::Unresolved {
                registration,
                suggested: Some(s),
                ..
            } => {
                assert_eq!(registration, Registration::Unregistered);
                assert_eq!(s.doi, "10.1000/xyz");
            }
            other => panic!("expected unresolved with a suggestion, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn doi_registered_with_another_agency_is_named() {
        let srv = unresolved_server(
            serde_json::json!([{ "RA": "mEDRA" }]),
            serde_json::json!({ "message": { "items": [] } }),
        )
        .await;
        let text = "References\nRossi M. Un articolo. https://doi.org/10.9999/medra";
        match unresolved_outcome_for(&srv, text).await {
            EntryOutcome::Unresolved {
                registration,
                suggested,
                ..
            } => {
                assert_eq!(registration, Registration::Agency("mEDRA".to_string()));
                assert!(suggested.is_none());
            }
            other => panic!("expected unresolved, got {other:?}"),
        }
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
        let datacite = DataCiteClient::with_base("", server.uri());
        let registration = RegistrationClient::with_base("", server.uri());
        let cache = MemoryCache::default();

        let text = "References\nSmith J. A Study of Widgets. Journal of Widgets.";
        let result = run(
            "a.pdf".into(),
            "fp".into(),
            "now".into(),
            text,
            &client,
            &datacite,
            &registration,
            &cache,
            5,
            |_| {},
        )
        .await;

        // The title "A Study of Widgets" is fully present in the reference text,
        // so the entry is promoted from a NoDoi suggestion to a via_search Resolved.
        match &result.entries[0].outcome {
            EntryOutcome::Resolved {
                doi, via_search, ..
            } => {
                assert_eq!(doi, "10.1000/xyz");
                assert!(*via_search);
            }
            other => panic!("expected via_search Resolved, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn suggests_datacite_doi_when_crossref_has_none() {
        // Crossref search finds nothing; DataCite has a matching record.
        let crossref = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({"message":{"items":[]}})),
            )
            .mount(&crossref)
            .await;
        let datacite_server = MockServer::start().await;
        let body = serde_json::json!({
            "data": [{"attributes": {
                "doi": "10.5281/zenodo.7",
                "titles": [{"title": "A Study of Widgets"}],
                "creators": [{"name": "Smith, J", "familyName": "Smith", "nameType": "Personal"}],
                "publicationYear": 2020
            }}]
        });
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .mount(&datacite_server)
            .await;
        let client = CrossrefClient::with_base("", crossref.uri());
        let datacite = DataCiteClient::with_base("", datacite_server.uri());
        let registration = RegistrationClient::with_base("", datacite_server.uri());
        let cache = MemoryCache::default();

        let text = "References\nSmith J. A Study of Widgets. Journal of Widgets.";
        let result = run(
            "a.pdf".into(),
            "fp".into(),
            "now".into(),
            text,
            &client,
            &datacite,
            &registration,
            &cache,
            5,
            |_| {},
        )
        .await;
        // The title "A Study of Widgets" is fully present in the reference text,
        // so the entry is promoted from a NoDoi suggestion to a via_search Resolved.
        match &result.entries[0].outcome {
            EntryOutcome::Resolved {
                doi,
                source,
                via_search,
                ..
            } => {
                assert_eq!(*source, Source::DataCite);
                assert_eq!(doi, "10.5281/zenodo.7");
                assert!(*via_search);
            }
            other => panic!("expected via_search DataCite Resolved, got {other:?}"),
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
        let datacite = DataCiteClient::with_base("", server.uri());
        let registration = RegistrationClient::with_base("", server.uri());
        let cache = MemoryCache::default();
        let text = "References\nSmith J. A Study of Widgets. Journal of Widgets.";

        let first = run(
            "a.pdf".into(),
            "fp".into(),
            "now".into(),
            text,
            &client,
            &datacite,
            &registration,
            &cache,
            5,
            |_| {},
        )
        .await;
        // The title "A Study of Widgets" is fully present in the reference text,
        // so the first run produces a via_search Resolved (not a NoDoi suggestion).
        assert!(matches!(
            &first.entries[0].outcome,
            EntryOutcome::Resolved {
                via_search: true,
                ..
            }
        ));

        // Second run: the search mock is exhausted, so a fresh search would fail.
        // The SuggestedDoi is replayed from the search cache; the DOI cache is
        // still seeded, so the via_search outcome is reproduced from cache.
        let second = run(
            "a.pdf".into(),
            "fp".into(),
            "now".into(),
            text,
            &client,
            &datacite,
            &registration,
            &cache,
            5,
            |_| {},
        )
        .await;
        match &second.entries[0].outcome {
            EntryOutcome::Resolved {
                doi,
                via_search,
                from_cache,
                ..
            } => {
                assert_eq!(doi, "10.1000/xyz");
                assert!(*via_search);
                assert!(
                    *from_cache,
                    "the reused via_search match must be marked from_cache"
                );
            }
            other => panic!("expected cached via_search Resolved, got {other:?}"),
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
        let datacite = DataCiteClient::with_base("", server.uri());
        let registration = RegistrationClient::with_base("", server.uri());
        let cache = MemoryCache::default();
        let text = "References\nSmith J. A Study of Widgets. Journal of Widgets.";
        let _ = run(
            "a.pdf".into(),
            "fp".into(),
            "now".into(),
            text,
            &client,
            &datacite,
            &registration,
            &cache,
            5,
            |_| {},
        )
        .await;

        // The matched work's record is now in the DOI cache under its DOI, so a
        // later direct resolve is a cache hit with usable metadata.
        let seeded = cache.get(Source::Crossref, &crate::doi::Doi::new("10.1000/xyz"));
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
        let datacite = DataCiteClient::with_base("", server.uri());
        let registration = RegistrationClient::with_base("", server.uri());
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
            &datacite,
            &registration,
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
            Source::Crossref,
            &crate::doi::Doi::new("10.1000/abc"),
            &serde_json::json!({"message":{"title":["Cached"],"DOI":"10.1000/abc"}}).to_string(),
        );
        let client = CrossrefClient::with_base("", server.uri());
        let datacite = DataCiteClient::with_base("", server.uri());
        let registration = RegistrationClient::with_base("", server.uri());
        let text = "References\n[1] Smith J (2020). A Study of Widgets. 10.1000/abc";
        let result = run(
            "a.pdf".into(),
            "fp".into(),
            "now".into(),
            text,
            &client,
            &datacite,
            &registration,
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
        let datacite = DataCiteClient::with_base("", server.uri());
        let registration = RegistrationClient::with_base("", server.uri());
        let cache = MemoryCache::default();
        let text = "References\n[1] Smith J (2020). A Study of Widgets. 10.1000/abc";
        let _ = run(
            "a.pdf".into(),
            "fp".into(),
            "now".into(),
            text,
            &client,
            &datacite,
            &registration,
            &cache,
            5,
            |_| {},
        )
        .await;
        assert!(
            cache
                .get(Source::Crossref, &crate::doi::Doi::new("10.1000/abc"))
                .is_some()
        );
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
        let datacite = DataCiteClient::with_base("", server.uri());
        let registration = RegistrationClient::with_base("", server.uri());
        let cache = MemoryCache::default();
        let text = "References\n[1] Smith J (2020). A Study of Widgets. 10.1000/abc";
        let result = run(
            "a.pdf".into(),
            "fp".into(),
            "now".into(),
            text,
            &client,
            &datacite,
            &registration,
            &cache,
            5,
            |_| {},
        )
        .await;
        match &result.entries[0].outcome {
            EntryOutcome::Unresolved { network_error, .. } => assert!(*network_error),
            other => panic!("expected transient Unresolved, got {other:?}"),
        }
        assert!(
            cache
                .get(Source::Crossref, &crate::doi::Doi::new("10.1000/abc"))
                .is_none()
        );
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
        let datacite = DataCiteClient::with_base("", server.uri());
        let registration = RegistrationClient::with_base("", server.uri());
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
                        registration: Registration::Unknown,
                        suggested: None,
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
                        source: Default::default(),
                        via_search: false,
                    },
                    llm_source: None,
                },
            ],
        };

        let updated =
            recheck_failures(result, &client, &datacite, &registration, &cache, 5, |_| {}).await;
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
        let datacite = DataCiteClient::with_base("", server.uri());
        let registration = RegistrationClient::with_base("", server.uri());
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
            &datacite,
            &registration,
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
    async fn full_search_match_becomes_clean_via_search() {
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
        let datacite = DataCiteClient::with_base("", server.uri());
        let registration = RegistrationClient::with_base("", server.uri());
        let cache = MemoryCache::default();
        let text = "References\nSmith J (2020). A Study of Widgets. Journal of Widgets.";
        let result = run(
            "a.pdf".into(),
            "fp".into(),
            "now".into(),
            text,
            &client,
            &datacite,
            &registration,
            &cache,
            5,
            |_| {},
        )
        .await;
        match &result.entries[0].outcome {
            EntryOutcome::Resolved {
                via_search,
                discrepancies,
                source,
                doi,
                ..
            } => {
                assert!(*via_search, "full-title match should be via_search");
                assert!(discrepancies.is_empty(), "metadata should match cleanly");
                assert_eq!(*source, Source::Crossref);
                assert_eq!(doi, "10.1000/xyz");
            }
            other => panic!("expected via_search Resolved, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn full_search_match_with_wrong_year_is_via_search_mismatch() {
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
        let datacite = DataCiteClient::with_base("", server.uri());
        let registration = RegistrationClient::with_base("", server.uri());
        let cache = MemoryCache::default();
        // Title fully present, but the reference cites the wrong year.
        let text = "References\nSmith J (1999). A Study of Widgets. Journal of Widgets.";
        let result = run(
            "a.pdf".into(),
            "fp".into(),
            "now".into(),
            text,
            &client,
            &datacite,
            &registration,
            &cache,
            5,
            |_| {},
        )
        .await;
        match &result.entries[0].outcome {
            EntryOutcome::Resolved {
                via_search,
                discrepancies,
                ..
            } => {
                assert!(*via_search);
                assert!(
                    discrepancies.iter().any(|d| d.field == "year"),
                    "expected a year discrepancy, got {discrepancies:?}"
                );
            }
            other => panic!("expected via_search Resolved, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn via_search_match_is_reproduced_from_cache() {
        let server = MockServer::start().await;
        let body = serde_json::json!({
            "message": { "items": [{
                "title": ["A Study of Widgets"],
                "author": [{"family": "Smith"}],
                "issued": {"date-parts": [[2020]]},
                "DOI": "10.1000/xyz"
            }]}
        });
        // Respond to the search exactly once; a second search would 404.
        Mock::given(method("GET"))
            .and(query_param("rows", "1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        let client = CrossrefClient::with_base("", server.uri());
        let datacite = DataCiteClient::with_base("", server.uri());
        let registration = RegistrationClient::with_base("", server.uri());
        let cache = MemoryCache::default();
        let text = "References\nSmith J (2020). A Study of Widgets. Journal of Widgets.";

        let first = run(
            "a.pdf".into(),
            "fp".into(),
            "now".into(),
            text,
            &client,
            &datacite,
            &registration,
            &cache,
            5,
            |_| {},
        )
        .await;
        assert!(matches!(
            first.entries[0].outcome,
            EntryOutcome::Resolved {
                via_search: true,
                ..
            }
        ));

        // Second run: the search mock is exhausted; the via_search outcome must
        // be rebuilt from the search cache and the seeded DOI-cache record.
        let second = run(
            "a.pdf".into(),
            "fp".into(),
            "now".into(),
            text,
            &client,
            &datacite,
            &registration,
            &cache,
            5,
            |_| {},
        )
        .await;
        match &second.entries[0].outcome {
            EntryOutcome::Resolved {
                via_search,
                from_cache,
                ..
            } => {
                assert!(*via_search);
                assert!(*from_cache, "reused via_search match must be from_cache");
            }
            other => panic!("expected cached via_search Resolved, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn partial_search_match_stays_a_suggestion() {
        let server = MockServer::start().await;
        // Title has five tokens; the reference omits "today" -> 80% coverage.
        let body = serde_json::json!({
            "message": { "items": [{
                "title": ["A Study of Widgets Today"],
                "DOI": "10.1000/xyz"
            }]}
        });
        Mock::given(method("GET"))
            .and(query_param("rows", "1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .mount(&server)
            .await;
        let client = CrossrefClient::with_base("", server.uri());
        let datacite = DataCiteClient::with_base("", server.uri());
        let registration = RegistrationClient::with_base("", server.uri());
        let cache = MemoryCache::default();
        let text = "References\nSmith J. A Study of Widgets. Journal of Widgets.";
        let result = run(
            "a.pdf".into(),
            "fp".into(),
            "now".into(),
            text,
            &client,
            &datacite,
            &registration,
            &cache,
            5,
            |_| {},
        )
        .await;
        match &result.entries[0].outcome {
            EntryOutcome::NoDoi {
                suggested: Some(s), ..
            } => {
                // 4 of the 5 title tokens present -> exactly 80%, below the
                // strict 100% promotion threshold, so it stays a suggestion.
                assert_eq!(s.title_match, 80);
            }
            other => panic!("expected NoDoi suggestion, got {other:?}"),
        }
    }

    #[test]
    fn full_match_degrades_to_suggestion_when_record_absent() {
        // A cached suggestion claims a full-title match, but the matched record
        // is no longer in the DOI cache (e.g. expired independently). Promotion
        // must degrade gracefully to a NoDoi suggestion rather than panic.
        let cache = MemoryCache::default();
        let suggested = Some(SuggestedDoi {
            doi: "10.1000/gone".into(),
            title_match: 100,
            source: Source::Crossref,
        });
        let outcome = finalise_no_doi(
            suggested,
            true,
            "Smith J (2020). A Study of Widgets. Journal of Widgets.",
            &cache,
        );
        match outcome {
            EntryOutcome::NoDoi {
                suggested: Some(s),
                from_cache,
            } => {
                assert_eq!(s.doi, "10.1000/gone");
                assert!(from_cache, "the cached search flag must be preserved");
            }
            other => panic!("expected NoDoi degrade, got {other:?}"),
        }
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
        let datacite = DataCiteClient::with_base("", server.uri());
        let registration = RegistrationClient::with_base("", server.uri());
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
            &datacite,
            &registration,
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
