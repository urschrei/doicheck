# Confirmed bibliographic-search match Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Promote a no-DOI reference whose search match covers the title fully (strict 100%) to a verified clean/mismatch entry, comparing author/year/container and annotating it as matched via bibliography search.

**Architecture:** Reuse `EntryOutcome::Resolved` with a new `via_search: bool` flag. A full-coverage search hit resolves the matched (already cache-seeded) record and runs the existing `compare()`. Counts, the text report, CSV, document status, and the entry card branch on the flag; everything else (discrepancy detection, false-positive dismissal, the cache tally) is reused unchanged.

**Tech Stack:** Rust (Tauri backend) tested with `cargo nextest`; Svelte 5 frontend verified with `npm run build`. Version control is jujutsu (`jj`).

**Spec:** `docs/superpowers/specs/2026-05-22-confirmed-search-match-design.md`

**Conventions for every commit (from CLAUDE.md):**
- Run `cargo fmt --manifest-path src-tauri/Cargo.toml` and `cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets` and fix warnings.
- Then `jj fix`, then `jj commit -m "[WIP: claude] <message>"`.
- Run Rust tests with `cargo nextest run --manifest-path src-tauri/Cargo.toml <substring-filter>`.
- UK spelling; new files end with a trailing newline; no whitespace on blank lines.

---

### Task 1: Add the `via_search` flag and count to the model

Adds the field and count, routes `counts()`, and updates every `Resolved` literal so the crate still compiles. Behaviour is otherwise unchanged (everything constructs `via_search: false`).

**Files:**
- Modify: `src-tauri/src/model.rs` (Resolved variant ~62-69, `Counts` ~92-108, `counts()` ~153-168, test literals ~212 and ~226)
- Modify: `src-tauri/src/pipeline.rs` (`resolved_outcome` ~34-52 and its two callers ~102/~110; recheck test literal ~875)
- Modify: `src-tauri/src/report.rs` (test literals ~214, ~269)
- Modify: `src-tauri/src/export.rs` (test literals ~97, ~111, ~224)
- Modify: `src-tauri/src/store.rs` (test literal ~545)

- [ ] **Step 1: Write the failing counts test**

Add to the `tests` module in `src-tauri/src/model.rs`:

```rust
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
```

- [ ] **Step 2: Run the test to verify it fails to compile**

Run: `cargo nextest run --manifest-path src-tauri/Cargo.toml counts_via_search_match`
Expected: build error — no field `via_search` on `EntryOutcome::Resolved`, no field `matched_via_search` on `Counts`.

- [ ] **Step 3: Add the `via_search` field and `matched_via_search` count**

In `src-tauri/src/model.rs`, add the field to the `Resolved` variant:

```rust
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
```

Add the count field to `Counts` (after `missing_doi_flagged`):

```rust
    pub missing_doi_flagged: usize,
    /// No-DOI references confirmed by a full-title bibliographic search.
    pub matched_via_search: usize,
    pub network_failed: usize,
```

- [ ] **Step 4: Route `counts()` by `via_search`**

In `src-tauri/src/model.rs`, replace the `EntryOutcome::Resolved` arm of `counts()` with:

```rust
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
```

- [ ] **Step 5: Give `resolved_outcome` a `via_search` parameter**

In `src-tauri/src/pipeline.rs`, change `resolved_outcome`:

```rust
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
```

Update its two callers in `resolve_doi_outcome` to pass `false` (cited DOIs):

```rust
            return resolved_outcome(doi, raw_text, &meta, from_cache, Source::Crossref, false);
```
```rust
            resolved_outcome(doi, raw_text, &meta, from_cache, Source::DataCite, false)
```

- [ ] **Step 6: Add `via_search: false` to every remaining `Resolved` literal**

These are struct constructions (not `..` patterns) and must list the new field. Add `via_search: false,` after the `source: ...,` line in each:

- `src-tauri/src/pipeline.rs` ~875 (recheck_failures test, the `Resolved` entry)
- `src-tauri/src/model.rs` ~212 and ~226 (counts_classify_each_outcome test)
- `src-tauri/src/report.rs` ~214 and ~269 (render tests)
- `src-tauri/src/export.rs` ~97, ~111, ~224 (csv tests)
- `src-tauri/src/store.rs` ~545 (list_documents test)

Example (the shape to match in each case):

```rust
                    outcome: EntryOutcome::Resolved {
                        doi: "10.1/a".into(),
                        discrepancies: vec![],
                        from_cache: true,
                        source: Default::default(),
                        via_search: false,
                    },
```

- [ ] **Step 7: Run the new test and the full suite**

Run: `cargo nextest run --manifest-path src-tauri/Cargo.toml counts_via_search_match`
Expected: PASS

Run: `cargo nextest run --manifest-path src-tauri/Cargo.toml`
Expected: PASS (all existing tests still pass)

- [ ] **Step 8: Format, lint, commit**

```bash
cargo fmt --manifest-path src-tauri/Cargo.toml
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets
jj fix
jj commit -m "[WIP: claude] model: add via_search flag and matched_via_search count"
```

---

### Task 2: Promote full-coverage search matches in the pipeline

**Files:**
- Modify: `src-tauri/src/pipeline.rs` (add `cached_metadata` and `finalise_no_doi` after `suggestion_from_hit` ~84; rewire the no-DOI branch of `outcome_for_entry` ~126-172)
- Test: `src-tauri/src/pipeline.rs` (tests module)

- [ ] **Step 1: Write the failing tests**

Add to the `tests` module in `src-tauri/src/pipeline.rs`:

```rust
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
        let cache = MemoryCache::default();
        let text = "References\nSmith J (2020). A Study of Widgets. Journal of Widgets.";
        let result = run(
            "a.pdf".into(), "fp".into(), "now".into(), text,
            &client, &datacite, &cache, 5, |_| {},
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
        let cache = MemoryCache::default();
        // Title fully present, but the reference cites the wrong year.
        let text = "References\nSmith J (1999). A Study of Widgets. Journal of Widgets.";
        let result = run(
            "a.pdf".into(), "fp".into(), "now".into(), text,
            &client, &datacite, &cache, 5, |_| {},
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
        let cache = MemoryCache::default();
        let text = "References\nSmith J (2020). A Study of Widgets. Journal of Widgets.";

        let first = run(
            "a.pdf".into(), "fp".into(), "now".into(), text,
            &client, &datacite, &cache, 5, |_| {},
        )
        .await;
        assert!(matches!(
            first.entries[0].outcome,
            EntryOutcome::Resolved { via_search: true, .. }
        ));

        // Second run: the search mock is exhausted; the via_search outcome must
        // be rebuilt from the search cache and the seeded DOI-cache record.
        let second = run(
            "a.pdf".into(), "fp".into(), "now".into(), text,
            &client, &datacite, &cache, 5, |_| {},
        )
        .await;
        match &second.entries[0].outcome {
            EntryOutcome::Resolved { via_search, from_cache, .. } => {
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
        let cache = MemoryCache::default();
        let text = "References\nSmith J. A Study of Widgets. Journal of Widgets.";
        let result = run(
            "a.pdf".into(), "fp".into(), "now".into(), text,
            &client, &datacite, &cache, 5, |_| {},
        )
        .await;
        match &result.entries[0].outcome {
            EntryOutcome::NoDoi { suggested: Some(s), .. } => {
                assert!(s.title_match < 100, "partial match must stay below 100");
            }
            other => panic!("expected NoDoi suggestion, got {other:?}"),
        }
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo nextest run --manifest-path src-tauri/Cargo.toml search_match`
Expected: the new full/partial tests FAIL — a full-coverage match is still returned as `NoDoi`, so the `Resolved` assertions panic.

- [ ] **Step 3: Add the helpers**

In `src-tauri/src/pipeline.rs`, after `suggestion_from_hit` (around line 84), add:

```rust
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
```

- [ ] **Step 4: Rewire the no-DOI branch to use `finalise_no_doi`**

In `outcome_for_entry`, replace the two outcome returns in the `None` branch. The cached-suggestion early return becomes:

```rust
            if let Some(json) = cache.search_get(&key) {
                let suggested = serde_json::from_str::<SuggestedDoi>(&json).ok();
                return finalise_no_doi(suggested, true, &entry.raw_text, cache);
            }
```

and the final `EntryOutcome::NoDoi { suggested, from_cache: false }` at the end of the branch becomes:

```rust
            finalise_no_doi(suggested, false, &entry.raw_text, cache)
```

Leave the search, the `suggestion_from_hit` calls, and the `cache.search_put` block between them unchanged.

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo nextest run --manifest-path src-tauri/Cargo.toml search_match`
Expected: PASS

Run: `cargo nextest run --manifest-path src-tauri/Cargo.toml`
Expected: PASS (the existing `suggests_doi_for_entry_without_one` and `search_*` tests still pass — their titles are not fully present in the references, so they remain suggestions)

- [ ] **Step 6: Format, lint, commit**

```bash
cargo fmt --manifest-path src-tauri/Cargo.toml
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets
jj fix
jj commit -m "[WIP: claude] pipeline: promote full-title search matches to via_search Resolved"
```

---

### Task 3: Reflect via-search matches in the text report

**Files:**
- Modify: `src-tauri/src/report.rs` (summary ~68-72; Discrepancies `Resolved` arm ~111-132; "Possibly missing DOIs" match ~164-186)
- Test: `src-tauri/src/report.rs` (tests module)

- [ ] **Step 1: Write the failing test**

Add to the `tests` module in `src-tauri/src/report.rs`:

```rust
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
        // Summary line.
        assert!(text.contains("Matched via search:"), "{text}");
        // Clean via-search entry listed under missing DOIs as a confirmed match.
        assert!(
            text.contains("no DOI; matched via Crossref search: 10.1/clean"),
            "{text}"
        );
        // Mismatched via-search entry annotated in the Discrepancies section.
        assert!(text.contains("no DOI; matched via DataCite search"), "{text}");
        assert!(text.contains("10.1/mism  year:"), "{text}");
    }
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo nextest run --manifest-path src-tauri/Cargo.toml renders_via_search_matches`
Expected: FAIL — none of the expected strings are present.

- [ ] **Step 3: Add the summary line**

In `render`, after the `No-DOI entries flagged` writeln block, add:

```rust
    let _ = writeln!(
        s,
        "  Matched via search:          {}",
        c.matched_via_search
    );
```

- [ ] **Step 4: Annotate via-search mismatches in the Discrepancies section**

In the `EntryOutcome::Resolved { .. } if discrepancies...` arm, add `via_search,` to the destructure and emit the annotation after the entry header line:

```rust
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
```

- [ ] **Step 5: List clean via-search matches under "Possibly missing DOIs"**

In the "Possibly missing DOIs" loop, add a new arm before the catch-all (so clean via-search entries are listed; mismatched ones already appear in Discrepancies and are excluded by the guard):

```rust
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
```

(The existing `NoDoi { suggested: Some(sug), .. }` arm stays as the first arm; the trailing catch-all above replaces the current one.)

- [ ] **Step 6: Run the test and the suite**

Run: `cargo nextest run --manifest-path src-tauri/Cargo.toml renders_via_search_matches`
Expected: PASS

Run: `cargo nextest run --manifest-path src-tauri/Cargo.toml report`
Expected: PASS (existing report tests unaffected)

- [ ] **Step 7: Format, lint, commit**

```bash
cargo fmt --manifest-path src-tauri/Cargo.toml
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets
jj fix
jj commit -m "[WIP: claude] report: show via_search matches in summary, discrepancies, and missing-DOI list"
```

---

### Task 4: CSV export status for via-search matches

**Files:**
- Modify: `src-tauri/src/export.rs` (`to_csv` `Resolved` arm ~18-31)
- Test: `src-tauri/src/export.rs` (tests module)

- [ ] **Step 1: Write the failing test**

Add to the `tests` module in `src-tauri/src/export.rs`:

```rust
    #[test]
    fn csv_marks_via_search_matches() {
        let result = CheckResult {
            filename: "x.pdf".into(),
            fingerprint: "fp".into(),
            run_at: "now".into(),
            bibliography_detected: true,
            entries: vec![
                CheckedEntry {
                    entry: ReferenceEntry {
                        ordinal: 1,
                        raw_text: "Clean via search".into(),
                        doi: None,
                    },
                    outcome: EntryOutcome::Resolved {
                        doi: "10.1/clean".into(),
                        discrepancies: vec![],
                        from_cache: false,
                        source: Default::default(),
                        via_search: true,
                    },
                    llm_source: None,
                },
                CheckedEntry {
                    entry: ReferenceEntry {
                        ordinal: 2,
                        raw_text: "Mismatch via search".into(),
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
                        source: Default::default(),
                        via_search: true,
                    },
                    llm_source: None,
                },
            ],
        };
        let csv = to_csv(&result);
        // doi column empty (no cited DOI), matched DOI in suggested_doi column.
        assert!(csv.contains("1,Clean via search,,matched_via_search,,10.1/clean,"));
        // mismatched fields recorded for the amber case.
        assert!(csv.contains("2,Mismatch via search,,matched_via_search,year,10.1/mism,"));
    }
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo nextest run --manifest-path src-tauri/Cargo.toml csv_marks_via_search`
Expected: FAIL — via-search rows currently emit status `clean`/`mismatch` with an empty `suggested_doi`.

- [ ] **Step 3: Branch the `Resolved` arm on `via_search`**

In `to_csv`, replace the `EntryOutcome::Resolved` arm:

```rust
            EntryOutcome::Resolved {
                doi,
                discrepancies,
                via_search,
                ..
            } => {
                let unmatched = discrepancies
                    .iter()
                    .filter(|d| !d.dismissed)
                    .map(|d| d.field.as_str())
                    .collect::<Vec<_>>()
                    .join("; ");
                if *via_search {
                    ("matched_via_search", unmatched, doi.clone())
                } else {
                    let status = if unmatched.is_empty() {
                        "clean"
                    } else {
                        "mismatch"
                    };
                    (status, unmatched, String::new())
                }
            }
```

- [ ] **Step 4: Run the test and the suite**

Run: `cargo nextest run --manifest-path src-tauri/Cargo.toml csv_marks_via_search`
Expected: PASS

Run: `cargo nextest run --manifest-path src-tauri/Cargo.toml export`
Expected: PASS

- [ ] **Step 5: Format, lint, commit**

```bash
cargo fmt --manifest-path src-tauri/Cargo.toml
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets
jj fix
jj commit -m "[WIP: claude] export: csv status matched_via_search with matched DOI in suggested column"
```

---

### Task 5: Document-status tests for via-search matches

`store.rs::list_documents` derives status from `counts()`, so no logic change is needed: a clean via-search entry adds nothing to `with_discrepancies`/`unresolved` (stays `Clean`), and a via-search mismatch raises `with_discrepancies` (becomes `HasIssues`). Lock this in with tests.

**Files:**
- Test: `src-tauri/src/store.rs` (tests module)

- [ ] **Step 1: Write the tests**

Add to the `tests` module in `src-tauri/src/store.rs`. That module imports `CheckedEntry, Discrepancy, EntryOutcome, ReferenceEntry` and brings `CheckResult`, `Store`, `DocumentStatus`, and `DocumentSummary` in via `use super::*`. `Source` is not imported, so use `Default::default()` (which is `Source::Crossref`) for the source. First add this helper, mirroring the existing `save_then_retrieve_latest_report` test's use of `save_check`:

```rust
    fn status_for_single_entry(entry: CheckedEntry) -> DocumentSummary {
        let mut store = Store::open_in_memory().unwrap();
        let result = CheckResult {
            filename: "x.pdf".into(),
            fingerprint: "sha256:via".into(),
            run_at: "now".into(),
            bibliography_detected: true,
            entries: vec![entry],
        };
        store.save_check(&result, "pdf", "REPORT").unwrap();
        store.list_documents().unwrap().pop().unwrap()
    }
```

Then the two cases:

```rust
    #[test]
    fn status_clean_for_clean_via_search_match() {
        let entry = CheckedEntry {
            entry: ReferenceEntry {
                ordinal: 1,
                raw_text: "Clean via search".into(),
                doi: None,
            },
            outcome: EntryOutcome::Resolved {
                doi: "10.1/clean".into(),
                discrepancies: vec![],
                from_cache: false,
                source: Default::default(),
                via_search: true,
            },
            llm_source: None,
        };
        let d = status_for_single_entry(entry);
        assert_eq!(d.status, DocumentStatus::Clean);
    }

    #[test]
    fn status_has_issues_for_via_search_mismatch() {
        let entry = CheckedEntry {
            entry: ReferenceEntry {
                ordinal: 1,
                raw_text: "Mismatch via search".into(),
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
                source: Default::default(),
                via_search: true,
            },
            llm_source: None,
        };
        let d = status_for_single_entry(entry);
        assert_eq!(d.status, DocumentStatus::HasIssues);
    }
```

- [ ] **Step 2: Run the tests**

Run: `cargo nextest run --manifest-path src-tauri/Cargo.toml via_search`
Expected: PASS

Run: `cargo nextest run --manifest-path src-tauri/Cargo.toml store`
Expected: PASS

- [ ] **Step 3: Format, lint, commit**

```bash
cargo fmt --manifest-path src-tauri/Cargo.toml
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets
jj fix
jj commit -m "[WIP: claude] store: tests for document status of via_search matches"
```

---

### Task 6: Annotate via-search matches in the entry card

**Files:**
- Modify: `src/lib/EntryCard.svelte`

- [ ] **Step 1: Add a `viaSearch` derived flag**

In the `<script>` block of `src/lib/EntryCard.svelte`, after the `resolvedSource` line, add:

```js
  // Whether this entry had no cited DOI and was matched by bibliographic search.
  const viaSearch = $derived(entry.outcome.Resolved?.via_search ?? false);
```

- [ ] **Step 2: Render the annotation line**

In the markup, after the `{#if entry.entry.raw_text}` blockquote block and before the `{#if active.length}` discrepancy list, add:

```svelte
  {#if viaSearch}
    <p class="suggest">No DOI: matched via bibliography search on {resolvedSource}.</p>
  {/if}
```

(The `suggest` class already exists and styles the existing suggestion line.)

- [ ] **Step 3: Build the frontend**

Run: `npm run build`
Expected: build completes (`✓ built`, "Wrote site to build").

- [ ] **Step 4: Commit**

```bash
jj fix
jj commit -m "[WIP: claude] ui: annotate via_search matches on the entry card"
```

---

### Task 7: Changelog and final verification

**Files:**
- Modify: `CHANGELOG.md` (under the existing `## [Unreleased]` section; create it above `## [0.5.0]` if absent)

- [ ] **Step 1: Add the changelog entry**

Add this bullet under `## [Unreleased]`:

```markdown
- Treat a reference that has no DOI but matches a Crossref or DataCite
  bibliographic search at full title coverage as a verified entry: it is
  compared on author, year, and container like a cited DOI (clean when all
  match, a flagged mismatch otherwise) and annotated "No DOI: matched via
  bibliography search on [source]". Such entries are reported among the
  missing-DOI references and counted separately from cited-DOI lookups.
```

- [ ] **Step 2: Full backend test run**

Run: `cargo nextest run --manifest-path src-tauri/Cargo.toml`
Expected: PASS (whole suite)

- [ ] **Step 3: Frontend build**

Run: `npm run build`
Expected: build completes.

- [ ] **Step 4: Format, lint, commit**

```bash
cargo fmt --manifest-path src-tauri/Cargo.toml
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets
jj fix
jj commit -m "[WIP: claude] changelog: confirmed bibliographic-search match"
```

---

## Notes for the implementer

- `tally()` in `pipeline.rs` needs **no** change: its `Resolved { from_cache, .. }` patterns already cover the new field, so a via-search match is counted once in the cache/fetch tally.
- `apply_dismissals` in `model.rs` needs **no** change: it matches all `Resolved` entries, so a via-search mismatch supports false-positive dismissal keyed on its matched DOI.
- The strict trigger is `token_coverage(raw_text, title) >= 1.0` (every title token present). `token_coverage` returns 0.0 for an empty title, so a record without a title never promotes.
- `recheck_failures` only retries transient `Unresolved` entries, so via-search matches are never re-checked.
