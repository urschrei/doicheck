# Report Enrichment Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the text and CSV reports identify each flagged reference by a human-readable snippet, disambiguate the document status dot (red for "DOI not found on Crossref", yellow for metadata mismatch), and surface the transient "needs retry" state in the text and CSV exports.

**Architecture:** All output changes live in three Rust modules (`report.rs`, `export.rs`, `store.rs`) plus one Svelte tooltip string (`Sidebar.svelte`). The data model is unchanged — failure kinds are already separated in `EntryOutcome` (404 → `Unresolved{network_error:false}`, transient → `Unresolved{network_error:true}`). JSON output is unchanged. A new private `snippet` helper in `report.rs` produces the truncated reference text for the text report; CSV carries the full reference text.

**Tech Stack:** Rust (Tauri backend), SvelteKit frontend. Tests run with `cargo nextest`. Version control is jujutsu (`jj`).

---

## Conventions for every task

- Run Rust tests with: `cargo nextest run --manifest-path src-tauri/Cargo.toml <filter>`
- Before each commit, run `cargo fmt --manifest-path src-tauri/Cargo.toml` and `cargo clippy --manifest-path src-tauri/Cargo.toml` and resolve clippy warnings.
- Commit with jujutsu (it auto-tracks changes; there is no `jj add`):
  ```bash
  jj fix
  jj commit -m "[WIP: claude] <message>"
  ```
- UK spelling in comments and strings. No emoji in comments. Files end with a trailing newline; inserted blank lines contain no whitespace.

---

## File Structure

- `src-tauri/src/report.rs` — add private `snippet` helper; two-line entry layout; reworded discrepancy/unresolved details; retry note line; update existing test.
- `src-tauri/src/export.rs` — add `reference_text` CSV column; rename transient status `network_error` → `retry_needed`; update + add tests.
- `src-tauri/src/store.rs` — `list_documents` status precedence gains a `failed` state for genuine not-found; add a test.
- `src/lib/Sidebar.svelte` — change the `failed` tooltip text.

---

## Task 1: `snippet` helper in `report.rs`

**Files:**
- Modify: `src-tauri/src/report.rs` (add helper above `render`, add tests in the existing `#[cfg(test)] mod tests`)

- [ ] **Step 1: Write the failing tests**

Add these tests inside the existing `mod tests` block in `src-tauri/src/report.rs` (after the existing test):

```rust
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
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo nextest run --manifest-path src-tauri/Cargo.toml report::tests::snippet`
Expected: FAIL — `cannot find function snippet in this scope`.

- [ ] **Step 3: Write the helper**

Add this function in `src-tauri/src/report.rs` immediately above `pub fn render`:

```rust
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
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo nextest run --manifest-path src-tauri/Cargo.toml report::tests::snippet`
Expected: PASS (3 tests).

- [ ] **Step 5: Format, lint, commit**

```bash
cargo fmt --manifest-path src-tauri/Cargo.toml
cargo clippy --manifest-path src-tauri/Cargo.toml
jj fix
jj commit -m "[WIP: claude] Add snippet helper for reference identifiers"
```

---

## Task 2: Two-line text report layout, reworded details, retry note

**Files:**
- Modify: `src-tauri/src/report.rs` (the `render` function body and the existing `renders_summary_discrepancies_and_missing` test)

- [ ] **Step 1: Update the existing test to expect the new layout, and add a retry test**

In `src-tauri/src/report.rs`, find the `renders_summary_discrepancies_and_missing` test. Change the `raw_text` of the ordinal-12 entry from `"r".into()` to `"Smith, J. (2020). Neural things. Journal.".into()` and the ordinal-33 entry from `"r".into()` to `"Lee, C. (2018). Untitled work.".into()`.

Then replace its assertions with:

```rust
        let text = render(&result);
        assert!(text.contains("Document:     thesis.pdf"));
        assert!(text.contains("[12] Smith, J. (2020). Neural things. Journal."));
        assert!(text.contains("10.1/yyy  title:"));
        assert!(text.contains("Neural Things"));
        assert!(text.contains("[33] Lee, C. (2018). Untitled work."));
        assert!(text.contains("no DOI; closest Crossref match 10.1000/xyz (title match 82%)"));
        assert!(text.contains("from cache:"));
```

Add this new test after it (note `ReferenceEntry`, `Discrepancy`, `SuggestedDoi`, `CheckedEntry` are already imported in this `mod tests`):

```rust
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
        assert!(
            text.contains("Note: 1 entry could not be checked (network or capacity) and should be re-checked: [7]")
        );
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo nextest run --manifest-path src-tauri/Cargo.toml report::tests`
Expected: FAIL — `renders_summary_discrepancies_and_missing` (old `[12] 10.1/yyy` no longer present) and `renders_retry_note_and_unresolved_wording` (wording/note absent).

- [ ] **Step 3: Add the retry note to the Summary section**

In `render`, immediately after the `if c.llm_flagged > 0 { ... }` block and before the blank `let _ = writeln!(s);` that separates the Summary from the Discrepancies heading, insert:

```rust
    let retry_ords: Vec<String> = result
        .entries
        .iter()
        .filter_map(|e| match &e.outcome {
            EntryOutcome::Unresolved {
                network_error: true,
                ..
            } => Some(format!("[{}]", e.entry.ordinal)),
            _ => None,
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
```

- [ ] **Step 4: Convert the Discrepancies section to the two-line layout**

In `render`, replace the whole `for e in &result.entries { match &e.outcome { ... } }` loop in the Discrepancies section (the one that starts after `let mut any_disc = false;`) with:

```rust
    for e in &result.entries {
        match &e.outcome {
            EntryOutcome::Resolved {
                doi, discrepancies, ..
            } if discrepancies.iter().any(|d| !d.dismissed) => {
                any_disc = true;
                let _ = writeln!(s, "  [{}] {}", e.entry.ordinal, snippet(&e.entry.raw_text));
                for d in discrepancies.iter().filter(|d| !d.dismissed) {
                    let _ = writeln!(
                        s,
                        "       {}  {}: ref \"{}\" vs Crossref \"{}\"",
                        doi, d.field, d.reference_value, d.crossref_value
                    );
                }
                if let Some(marker) = &e.llm_source {
                    let _ = writeln!(
                        s,
                        "       ** POSSIBLE AI SOURCE - reference URL contains \"{}\" **",
                        marker
                    );
                }
            }
            EntryOutcome::Unresolved { doi, network_error } => {
                any_disc = true;
                let reason = if *network_error {
                    "could not be checked — retry needed"
                } else {
                    "DOI not found on Crossref"
                };
                let _ = writeln!(s, "  [{}] {}", e.entry.ordinal, snippet(&e.entry.raw_text));
                let _ = writeln!(s, "       {}  {}", doi, reason);
                if let Some(marker) = &e.llm_source {
                    let _ = writeln!(
                        s,
                        "       ** POSSIBLE AI SOURCE - reference URL contains \"{}\" **",
                        marker
                    );
                }
            }
            _ => {
                if let Some(marker) = &e.llm_source {
                    any_disc = true;
                    let _ = writeln!(s, "  [{}] {}", e.entry.ordinal, snippet(&e.entry.raw_text));
                    let _ = writeln!(
                        s,
                        "       ** POSSIBLE AI SOURCE - reference URL contains \"{}\" **",
                        marker
                    );
                }
            }
        }
    }
```

- [ ] **Step 5: Convert the "Possibly missing DOIs" section to the two-line layout**

In `render`, replace the `if let EntryOutcome::NoDoi { suggested: Some(sug) } = &e.outcome { ... }` body in the missing-DOIs loop with:

```rust
            any_missing = true;
            let _ = writeln!(s, "  [{}] {}", e.entry.ordinal, snippet(&e.entry.raw_text));
            let _ = writeln!(
                s,
                "       no DOI; closest Crossref match {} (title match {}%)",
                sug.doi, sug.title_match
            );
```

- [ ] **Step 6: Run the tests to verify they pass**

Run: `cargo nextest run --manifest-path src-tauri/Cargo.toml report::tests`
Expected: PASS (all report tests, including the two updated/added ones).

- [ ] **Step 7: Format, lint, commit**

```bash
cargo fmt --manifest-path src-tauri/Cargo.toml
cargo clippy --manifest-path src-tauri/Cargo.toml
jj fix
jj commit -m "[WIP: claude] Two-line text report with reference snippets and retry note"
```

---

## Task 3: CSV `reference_text` column and `retry_needed` status

**Files:**
- Modify: `src-tauri/src/export.rs` (the `to_csv` function and its tests)

- [ ] **Step 1: Update the CSV tests to expect the new column and status**

In `src-tauri/src/export.rs`, in the `csv_has_header_and_rows` test, replace the four assertions with:

```rust
        assert_eq!(
            lines[0],
            "ordinal,reference_text,doi,status,unmatched_fields,suggested_doi,llm_source"
        );
        assert_eq!(lines[1], "1,r,10.1000/a,clean,,,");
        assert_eq!(lines[2], "2,r,10.1000/b,mismatch,year,,");
        assert_eq!(lines[3], "3,r,,no_doi,,10.1000/c,");
```

Add this new test after `csv_has_header_and_rows`:

```rust
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
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo nextest run --manifest-path src-tauri/Cargo.toml export::tests`
Expected: FAIL — header mismatch in `csv_has_header_and_rows`; `retry_needed` absent in the new test.

- [ ] **Step 3: Add the `reference_text` column to the header and rows**

In `to_csv`, change the header line from:

```rust
    let mut out = String::from("ordinal,doi,status,unmatched_fields,suggested_doi,llm_source\n");
```

to:

```rust
    let mut out =
        String::from("ordinal,reference_text,doi,status,unmatched_fields,suggested_doi,llm_source\n");
```

Then change the row `push_str` from:

```rust
        out.push_str(&format!(
            "{},{},{},{},{},{}\n",
            e.entry.ordinal,
            csv_field(e.entry.doi.as_deref().unwrap_or("")),
            status,
            csv_field(&unmatched),
            csv_field(&suggested),
            csv_field(llm),
        ));
```

to:

```rust
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
```

- [ ] **Step 4: Rename the transient status value to `retry_needed`**

In `to_csv`, in the `EntryOutcome::Unresolved { network_error, .. }` arm, change `"network_error"` to `"retry_needed"`:

```rust
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
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo nextest run --manifest-path src-tauri/Cargo.toml export::tests`
Expected: PASS (including `json_round_trips`, which is unaffected).

- [ ] **Step 6: Format, lint, commit**

```bash
cargo fmt --manifest-path src-tauri/Cargo.toml
cargo clippy --manifest-path src-tauri/Cargo.toml
jj fix
jj commit -m "[WIP: claude] Add reference_text column and retry_needed status to CSV"
```

---

## Task 4: Document status `failed` for genuine not-found

**Files:**
- Modify: `src-tauri/src/store.rs` (the `list_documents` status match and the test module)

- [ ] **Step 1: Write the failing test**

Add this test inside the `#[cfg(test)] mod tests` block in `src-tauri/src/store.rs` (after `status_incomplete_when_network_failed`):

```rust
    #[test]
    fn status_failed_when_doi_not_found() {
        let mut store = Store::open_in_memory().unwrap();
        let mut r = sample();
        r.fingerprint = "sha256:nf".into();
        r.entries = vec![CheckedEntry {
            entry: ReferenceEntry {
                ordinal: 1,
                raw_text: "x".into(),
                doi: Some("10.1/a".into()),
            },
            outcome: EntryOutcome::Unresolved {
                doi: "10.1/a".into(),
                network_error: false,
            },
            llm_source: None,
        }];
        store.save_check(&r, "pdf", "T").unwrap();
        let docs = store.list_documents().unwrap();
        let d = docs.iter().find(|d| d.fingerprint == "sha256:nf").unwrap();
        assert_eq!(d.status, "failed");
    }
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo nextest run --manifest-path src-tauri/Cargo.toml store::tests::status_failed_when_doi_not_found`
Expected: FAIL — status is `has-issues`, not `failed`.

- [ ] **Step 3: Add the `failed` precedence to `list_documents`**

In `src-tauri/src/store.rs`, in `list_documents`, replace the status match:

```rust
            let status = match self.latest_result(&fingerprint)? {
                Some(result) => {
                    let c = result.counts();
                    if c.network_failed > 0 {
                        "incomplete"
                    } else if c.with_discrepancies > 0 || c.unresolved > 0 {
                        "has-issues"
                    } else {
                        "clean"
                    }
                }
                None => "clean",
            }
            .to_string();
```

with:

```rust
            let status = match self.latest_result(&fingerprint)? {
                Some(result) => {
                    let c = result.counts();
                    let not_found = c.unresolved.saturating_sub(c.network_failed);
                    if c.network_failed > 0 {
                        "incomplete"
                    } else if not_found > 0 {
                        "failed"
                    } else if c.with_discrepancies > 0 {
                        "has-issues"
                    } else {
                        "clean"
                    }
                }
                None => "clean",
            }
            .to_string();
```

- [ ] **Step 4: Run the store tests to verify they pass**

Run: `cargo nextest run --manifest-path src-tauri/Cargo.toml store::tests`
Expected: PASS — the new test passes; `status_incomplete_when_network_failed`, `save_then_retrieve_latest_report` (mismatch → `has-issues`), and `dismissal_clears_issue_and_status` still pass.

- [ ] **Step 5: Format, lint, commit**

```bash
cargo fmt --manifest-path src-tauri/Cargo.toml
cargo clippy --manifest-path src-tauri/Cargo.toml
jj fix
jj commit -m "[WIP: claude] Emit failed status for documents with not-found DOIs"
```

---

## Task 5: Sidebar `failed` tooltip text and frontend build

**Files:**
- Modify: `src/lib/Sidebar.svelte:7`

- [ ] **Step 1: Update the tooltip text**

In `src/lib/Sidebar.svelte`, change the `failed` line from:

```javascript
    if (status === "failed") return { glyph: "●", colour: "var(--sev-fail)", title: "Check failed" };
```

to:

```javascript
    if (status === "failed") return { glyph: "●", colour: "var(--sev-fail)", title: "DOI not found on Crossref" };
```

- [ ] **Step 2: Run eslint --fix on the file**

Run: `npx eslint --fix src/lib/Sidebar.svelte`
Expected: no errors.

- [ ] **Step 3: Build the frontend**

Run: `npm run build`
Expected: build completes with no errors.

- [ ] **Step 4: Commit**

```bash
jj fix
jj commit -m "[WIP: claude] Update sidebar failed-status tooltip to DOI not found"
```

---

## Final verification

- [ ] **Run the full Rust test suite**

Run: `cargo nextest run --manifest-path src-tauri/Cargo.toml`
Expected: all tests pass.

- [ ] **Confirm clippy is clean and the frontend builds**

Run:
```bash
cargo clippy --manifest-path src-tauri/Cargo.toml
npm run build
```
Expected: no clippy warnings; frontend build succeeds.
