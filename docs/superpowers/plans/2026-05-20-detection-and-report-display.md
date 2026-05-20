# Detection Hardening & Report Display Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix the false-positive discrepancies caused by failed bibliography detection, then expose structured results to the UI and rebuild the discrepancy display and save/export experience.

**Architecture:** Backend gains robust heading detection, author–date/numbered entry segmentation with de-wrapping, a DOI-window fallback, a comparability guard, and HTML-unescaping of Crossref values; the structured `CheckResult` is persisted as JSON and returned to the UI, which renders per-entry cards (problems first) with severity, actions, and improved save/export.

**Tech Stack:** Rust (regex, serde_json, rusqlite, reqwest, wiremock), Svelte 5 / SvelteKit, Tauri v2 (`opener`, `dialog` plugins).

---

## Conventions for every task

- **Version control: jujutsu (`jj`), not git.** Run `jj st` before each task. Commit steps run `jj fix` then `jj commit -m "..."`.
- **Rust checks before each commit (from `src-tauri/`):** `cargo fmt`, `cargo clippy --all-targets -- -D warnings`, `cargo nextest r`.
- **Frontend checks (from repo root):** `npm run build` must succeed.
- UK spelling; no emoji in code/comments; trailing newline on new files; no whitespace on blank lines.
- Repo root: `/Users/sth/dev/doicheck`. Backend: `src-tauri/`. Library crate: `doicheck_lib`.

## File structure

- `src-tauri/src/doi.rs` — add `extract_with_context`.
- `src-tauri/src/biblio.rs` — relax heading, rewrite segmentation, add no-heading fallback in `detect`.
- `src-tauri/src/text.rs` — add `is_comparable`.
- `src-tauri/src/crossref.rs` — unescape Crossref string values.
- `src-tauri/src/pipeline.rs` — use `biblio::detect` for all paths, apply `is_comparable` guard.
- `src-tauri/src/store.rs` — `result_json` column + migration + `latest_result`.
- `src-tauri/src/export.rs` — new: CSV/JSON rendering of a `CheckResult`.
- `src-tauri/src/commands.rs`, `src-tauri/src/lib.rs` — new command surface.
- `src/lib/api.js`, `src/lib/result.js` (new), `src/lib/ReportPane.svelte` (rewrite), `src/lib/EntryCard.svelte` (new), `src/lib/Settings.svelte`, `src/routes/+page.svelte`.

---

## Task 1: DOI extraction with surrounding context (`doi.rs`)

**Files:** Modify `src-tauri/src/doi.rs`

- [ ] **Step 1: Add the failing test**

Append to the `tests` module in `src-tauri/src/doi.rs`:

```rust
    #[test]
    fn extract_with_context_windows_each_doi() {
        let text = "Smith, J. (2020). A Study of Widgets. Journal. https://doi.org/10.1000/abc \
                    and later Jones (2019). Other Work. 10.2000/def";
        let v = extract_with_context(text);
        assert_eq!(v.len(), 2);
        assert_eq!(v[0].0, "10.1000/abc");
        assert!(v[0].1.contains("A Study of Widgets"));
        assert_eq!(v[1].0, "10.2000/def");
        assert!(v[1].1.contains("Other Work"));
        // The second window starts after the first DOI, so it must not
        // contain the first entry's title.
        assert!(!v[1].1.contains("Widgets"));
    }
```

- [ ] **Step 2: Run it to confirm it fails**

Run: `cd src-tauri && cargo nextest r doi::tests::extract_with_context_windows_each_doi`
Expected: FAIL (function `extract_with_context` not found).

- [ ] **Step 3: Implement `extract_with_context`**

Add to `src-tauri/src/doi.rs` (after `first_in`):

```rust
/// For each DOI in the text, return the DOI plus the text immediately preceding
/// it (back to the previous DOI, capped at a window), de-wrapped. Used as a
/// fallback when no bibliography heading is detected, so comparison still has
/// real reference text to work with. De-duplicates by DOI.
pub fn extract_with_context(text: &str) -> Vec<(String, String)> {
    const WINDOW: usize = 600;
    let mut out: Vec<(String, String)> = Vec::new();
    let mut seen: Vec<String> = Vec::new();
    let mut prev_end = 0usize;
    for m in DOI_RE.find_iter(text) {
        let doi = normalise(m.as_str());
        if seen.contains(&doi) {
            prev_end = m.end();
            continue;
        }
        let lower_bound = m.start().saturating_sub(WINDOW).max(prev_end);
        let start = snap_char_boundary(text, lower_bound);
        let context = text[start..m.end()]
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        out.push((doi.clone(), context));
        seen.push(doi);
        prev_end = m.end();
    }
    out
}

fn snap_char_boundary(s: &str, mut i: usize) -> usize {
    while !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}
```

- [ ] **Step 4: Run it to confirm it passes**

Run: `cargo nextest r doi::`
Expected: PASS (all doi tests).

- [ ] **Step 5: Commit**

```bash
cargo fmt && cargo clippy --all-targets -- -D warnings
jj fix && jj commit -m "Add DOI extraction with surrounding context"
```

---

## Task 2: Relax bibliography heading detection (`biblio.rs`)

**Files:** Modify `src-tauri/src/biblio.rs`

- [ ] **Step 1: Add failing tests**

Append to the `tests` module in `src-tauri/src/biblio.rs`:

```rust
    #[test]
    fn heading_allows_section_number_but_not_toc() {
        // Real heading with a section number on its own line.
        let text = "body\n 6. References  \nAdams, D. (2012). Title. 10.4324/9780203857007";
        assert!(section_after_heading(text).is_some());
        // A table-of-contents line (dotted leaders + page number) must NOT match.
        let toc = "6. References .......................................... 13\nmore body";
        assert!(section_after_heading(toc).is_none());
    }

    #[test]
    fn heading_still_matches_plain_keywords() {
        assert!(section_after_heading("x\nReferences\n[1] A 10.1000/a").is_some());
        assert!(section_after_heading("x\nBibliography\nA 10.1000/a").is_some());
    }
```

- [ ] **Step 2: Run to confirm failure**

Run: `cargo nextest r biblio::tests::heading_allows_section_number_but_not_toc`
Expected: FAIL (the `6. References` line is not matched by the current regex).

- [ ] **Step 3: Relax the heading regex**

In `src-tauri/src/biblio.rs`, replace the `HEADING_RE` definition:

```rust
static HEADING_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?im)^\s*(references|bibliography|works cited|literature cited)\s*$").unwrap()
});
```

with:

```rust
static HEADING_RE: LazyLock<Regex> = LazyLock::new(|| {
    // Optional leading section number or Roman numeral, then the keyword as
    // effectively the whole line. Trailing dotted leaders/page numbers (a
    // table-of-contents entry) prevent a match.
    Regex::new(
        r"(?im)^\s*(?:\d+\.?\s+|[ivxlcdm]+\.?\s+)?(references|bibliography|works cited|literature cited)\s*$",
    )
    .unwrap()
});
```

- [ ] **Step 4: Run to confirm pass**

Run: `cargo nextest r biblio::`
Expected: the new heading tests PASS; the existing `finds_section_after_last_heading` and `undetected_when_no_heading` still PASS, and `splits_numbered_entries_and_finds_dois` still PASS.

- [ ] **Step 5: Commit**

```bash
cargo fmt && cargo clippy --all-targets -- -D warnings
jj fix && jj commit -m "Relax bibliography heading detection to allow section numbers"
```

---

## Task 3: Author–date / numbered entry segmentation with de-wrapping (`biblio.rs`)

**Files:** Modify `src-tauri/src/biblio.rs`

- [ ] **Step 1: Add a failing test using a realistic PDF-derived fixture**

Append to the `tests` module in `src-tauri/src/biblio.rs`:

```rust
    // Models how pdf-extract renders an author-date reference list: a numbered
    // heading, hanging-indent wrapping, and blank lines within and between
    // entries. The third entry has no DOI (a handle.net link).
    const SAMPLE: &str = "Some preamble paragraph.\n \n 6. References  \n \n\
Adams, D., & Watkins, C. (2012). Urban Planning and the Development Process. Routledge. \n \n\
https://doi.org/10.4324/9780203857007 \n \n\
Arnstein, S. R. (1969). A Ladder of Citizen Participation. Journal of the American Institute of \n \n\
Planners, 35(4), 216-224. https://doi.org/10.1080/01944366908977225 \n \n\
Malfer, B. (2025). Smart Cities in the European Union. Handle.Net. \n \n\
https://hdl.handle.net/20.500.12608/83965 \n";

    #[test]
    fn segments_wrapped_author_date_entries() {
        let bib = detect(SAMPLE);
        assert!(bib.detected);
        assert_eq!(bib.entries.len(), 3);
        assert_eq!(
            bib.entries[0].doi.as_deref(),
            Some("10.4324/9780203857007")
        );
        // The second entry's wrapped continuation line must be joined in.
        assert!(bib.entries[1].raw_text.contains("Planners"));
        assert_eq!(
            bib.entries[1].doi.as_deref(),
            Some("10.1080/01944366908977225")
        );
        // The handle.net entry has no DOI.
        assert_eq!(bib.entries[2].doi, None);
        assert!(bib.entries[2].raw_text.contains("Malfer"));
    }
```

- [ ] **Step 2: Run to confirm failure**

Run: `cargo nextest r biblio::tests::segments_wrapped_author_date_entries`
Expected: FAIL (current blank-line splitter shreds the wrapped entries).

- [ ] **Step 3: Rewrite segmentation**

In `src-tauri/src/biblio.rs`, replace the `split_entries` and `split_on_markers` functions (lines defining them) with this entry-start detection plus de-wrap, keeping `collapse_ws`:

```rust
static YEAR_PAREN_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\(\d{4}[a-z]?\)").unwrap());

/// A line begins a new entry if it carries a numbered marker, or it looks like
/// an author-date opening: starts with an uppercase letter and has a
/// parenthesised year near the start.
fn is_entry_start(line: &str) -> bool {
    if NUMBER_MARKER_RE.is_match(line) {
        return true;
    }
    let trimmed = line.trim_start();
    let begins_upper = trimmed.chars().next().is_some_and(|c| c.is_uppercase());
    begins_upper && YEAR_PAREN_RE.find(trimmed).is_some_and(|m| m.start() <= 80)
}

/// Split a bibliography section into entries by detecting entry starts and
/// joining wrapped continuation lines. Falls back to blank-line paragraphs if
/// no entry starts are found.
pub fn split_entries(section: &str) -> Vec<String> {
    let mut entries: Vec<String> = Vec::new();
    let mut current: Option<String> = None;
    for line in section.lines() {
        if is_entry_start(line) {
            if let Some(buf) = current.take() {
                let cleaned = collapse_ws(&buf);
                if !cleaned.is_empty() {
                    entries.push(cleaned);
                }
            }
            current = Some(line.to_string());
        } else if let Some(buf) = current.as_mut() {
            buf.push(' ');
            buf.push_str(line);
        }
    }
    if let Some(buf) = current {
        let cleaned = collapse_ws(&buf);
        if !cleaned.is_empty() {
            entries.push(cleaned);
        }
    }

    if entries.is_empty() {
        // No detectable entry starts: fall back to blank-line paragraphs.
        return section
            .split("\n\n")
            .map(collapse_ws)
            .filter(|s| !s.is_empty())
            .collect();
    }
    entries
}
```

- [ ] **Step 4: Run to confirm pass**

Run: `cargo nextest r biblio::`
Expected: all biblio tests PASS, including the new fixture test and the existing `splits_numbered_entries_and_finds_dois` (numbered markers still detected as entry starts).

- [ ] **Step 5: Commit**

```bash
cargo fmt && cargo clippy --all-targets -- -D warnings
jj fix && jj commit -m "Segment author-date and numbered reference lists with de-wrapping"
```

---

## Task 4: No-heading fallback in `detect` (`biblio.rs`)

**Files:** Modify `src-tauri/src/biblio.rs`

- [ ] **Step 1: Update the existing no-heading test and add a fallback test**

In `src-tauri/src/biblio.rs`, replace the existing `undetected_when_no_heading` test with:

```rust
    #[test]
    fn no_heading_falls_back_to_doi_windows() {
        let bib = detect("Just a body with 10.1000/xyz inline and no heading line.");
        assert!(!bib.detected);
        assert_eq!(bib.entries.len(), 1);
        assert_eq!(bib.entries[0].doi.as_deref(), Some("10.1000/xyz"));
        // The window carries surrounding text, not just the bare DOI.
        assert!(bib.entries[0].raw_text.contains("body"));
    }
```

- [ ] **Step 2: Run to confirm failure**

Run: `cargo nextest r biblio::tests::no_heading_falls_back_to_doi_windows`
Expected: FAIL (current `detect` returns empty entries when no heading).

- [ ] **Step 3: Implement the fallback in `detect`**

In `src-tauri/src/biblio.rs`, replace the `detect` function with:

```rust
/// Detect and segment the bibliography from full document text. If no heading is
/// found, fall back to DOI-anchored windows so comparison still has real text.
pub fn detect(text: &str) -> Bibliography {
    if let Some(section) = section_after_heading(text) {
        let entries = split_entries(section)
            .into_iter()
            .enumerate()
            .map(|(i, raw_text)| ReferenceEntry {
                ordinal: i + 1,
                doi: crate::doi::first_in(&raw_text),
                raw_text,
            })
            .collect();
        Bibliography {
            detected: true,
            entries,
        }
    } else {
        let entries = crate::doi::extract_with_context(text)
            .into_iter()
            .enumerate()
            .map(|(i, (doi, raw_text))| ReferenceEntry {
                ordinal: i + 1,
                raw_text,
                doi: Some(doi),
            })
            .collect();
        Bibliography {
            detected: false,
            entries,
        }
    }
}
```

- [ ] **Step 4: Run to confirm pass**

Run: `cargo nextest r biblio::`
Expected: all biblio tests PASS.

- [ ] **Step 5: Commit**

```bash
cargo fmt && cargo clippy --all-targets -- -D warnings
jj fix && jj commit -m "Fall back to DOI-anchored windows when no heading is found"
```

---

## Task 5: Comparability guard (`text.rs`)

**Files:** Modify `src-tauri/src/text.rs`

- [ ] **Step 1: Add a failing test**

Append to the `tests` module in `src-tauri/src/text.rs`:

```rust
    #[test]
    fn is_comparable_requires_real_text() {
        assert!(!is_comparable("10.1000/abc"));
        assert!(!is_comparable("https://doi.org/10.1000/abc"));
        assert!(is_comparable(
            "Smith, J. (2020). A Study of Widgets. Journal of Widgets."
        ));
    }
```

- [ ] **Step 2: Run to confirm failure**

Run: `cargo nextest r text::tests::is_comparable_requires_real_text`
Expected: FAIL (function not found).

- [ ] **Step 3: Implement `is_comparable`**

Append to `src-tauri/src/text.rs` (before the `tests` module):

```rust
/// Whether a reference string has enough non-identifier text to compare against
/// Crossref metadata. Strips URL/DOI tokens, then requires a minimum count of
/// alphanumeric characters. Prevents false discrepancies for entries whose only
/// content is a DOI (e.g. a sparse fallback window).
pub fn is_comparable(reference: &str) -> bool {
    let without_ids: String = reference
        .split_whitespace()
        .filter(|t| {
            let l = t.to_ascii_lowercase();
            !l.starts_with("http") && !l.starts_with("10.") && !l.contains("doi.org")
        })
        .collect::<Vec<_>>()
        .join(" ");
    without_ids.chars().filter(|c| c.is_alphanumeric()).count() >= 15
}
```

- [ ] **Step 4: Run to confirm pass**

Run: `cargo nextest r text::`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
cargo fmt && cargo clippy --all-targets -- -D warnings
jj fix && jj commit -m "Add is_comparable guard for reference text"
```

---

## Task 6: Unescape HTML entities in Crossref values (`crossref.rs`)

**Files:** Modify `src-tauri/src/crossref.rs`

- [ ] **Step 1: Add a failing test**

Append a `tests` module at the end of `src-tauri/src/crossref.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn to_metadata_unescapes_html_entities() {
        let work = Work {
            title: vec!["Science, Technology, &amp; Human Values".to_string()],
            author: vec![Author {
                family: "O&apos;Neil".to_string(),
            }],
            container_title: vec!["A &lt;Journal&gt;".to_string()],
            issued: None,
            doi: "10.1000/x".to_string(),
        };
        let m = work.to_metadata();
        assert_eq!(m.title.as_deref(), Some("Science, Technology, & Human Values"));
        assert_eq!(m.first_author_surname.as_deref(), Some("O'Neil"));
        assert_eq!(m.container_title.as_deref(), Some("A <Journal>"));
    }
}
```

- [ ] **Step 2: Run to confirm failure**

Run: `cargo nextest r crossref::tests::to_metadata_unescapes_html_entities`
Expected: FAIL (values still contain `&amp;` etc.).

- [ ] **Step 3: Implement unescaping in `to_metadata`**

In `src-tauri/src/crossref.rs`, add a helper and apply it in `to_metadata`. Add near the top (after the imports):

```rust
/// Resolve XML/HTML entity references that Crossref sometimes returns in string
/// fields (e.g. `&amp;`, `&#233;`). Falls back to the raw string on error.
fn unescape(s: &str) -> String {
    quick_xml::escape::unescape(s)
        .map(|c| c.into_owned())
        .unwrap_or_else(|_| s.to_string())
}
```

Then replace the body of `to_metadata` with:

```rust
    fn to_metadata(&self) -> Metadata {
        Metadata {
            title: self.title.first().map(|t| unescape(t)),
            first_author_surname: self
                .author
                .first()
                .map(|a| unescape(&a.family))
                .filter(|f| !f.is_empty()),
            year: self
                .issued
                .as_ref()
                .and_then(|i| i.date_parts.first())
                .and_then(|p| p.first())
                .copied(),
            container_title: self.container_title.first().map(|c| unescape(c)),
        }
    }
```

Note: `quick_xml` is already a dependency (used by `extract::docx`).

- [ ] **Step 4: Run to confirm pass**

Run: `cargo nextest r crossref::`
Expected: PASS (the new unit test and the existing integration tests in `tests/crossref_client.rs`).

- [ ] **Step 5: Commit**

```bash
cargo fmt && cargo clippy --all-targets -- -D warnings
jj fix && jj commit -m "Unescape HTML entities in Crossref metadata values"
```

---

## Task 7: Apply real reference text and the comparability guard in the pipeline (`pipeline.rs`)

**Files:** Modify `src-tauri/src/pipeline.rs`

The pipeline no longer builds its own fallback (that now lives in `biblio::detect`), and it only compares when the reference text is comparable.

- [ ] **Step 1: Add a failing test**

Append to the `tests` module in `src-tauri/src/pipeline.rs`:

```rust
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
        let result = run("a.pdf".into(), "fp".into(), "now".into(), text, &client, |_| {}).await;

        assert!(!result.bibliography_detected);
        assert_eq!(result.entries.len(), 1);
        match &result.entries[0].outcome {
            EntryOutcome::Resolved { discrepancies, .. } => assert!(discrepancies.is_empty()),
            other => panic!("expected Resolved, got {other:?}"),
        }
    }
```

- [ ] **Step 2: Run to confirm failure**

Run: `cargo nextest r pipeline::tests::no_heading_uses_window_text_for_comparison`
Expected: FAIL (current fallback sets `raw_text` to the bare DOI, producing discrepancies).

- [ ] **Step 3: Rewrite the pipeline body**

In `src-tauri/src/pipeline.rs`, replace the section from `let bib = crate::biblio::detect(text);` down to the start of the `for` loop (the `let (detected, raw_entries) = ...` block) with:

```rust
    let bib = crate::biblio::detect(text);
    let detected = bib.detected;
    let raw_entries = bib.entries;
```

Then, inside the `for` loop, replace the `Ok(meta) => EntryOutcome::Resolved { ... }` arm with one that applies the comparability guard:

```rust
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
```

Remove the now-unused imports if clippy flags them (`ReferenceEntry` and `SuggestedDoi` may still be used by the search arm and other tests — only remove what clippy reports as unused).

- [ ] **Step 4: Run to confirm pass**

Run: `cargo nextest r pipeline::`
Expected: all pipeline tests PASS (the two existing tests and the new one).

- [ ] **Step 5: Commit**

```bash
cargo fmt && cargo clippy --all-targets -- -D warnings
jj fix && jj commit -m "Compare against real reference text with a comparability guard"
```

---

## Task 8: Persist the structured result (`store.rs`)

**Files:** Modify `src-tauri/src/store.rs`

- [ ] **Step 1: Add failing tests**

Append to the `tests` module in `src-tauri/src/store.rs`:

```rust
    #[test]
    fn save_then_retrieve_structured_result() {
        let mut store = Store::open_in_memory().unwrap();
        let r = sample();
        store.save_check(&r, "pdf", "REPORT TEXT").unwrap();
        let got = store.latest_result("sha256:aaa").unwrap();
        assert_eq!(got, Some(r));
        assert_eq!(store.latest_result("sha256:none").unwrap(), None);
    }

    #[test]
    fn migrate_is_idempotent_on_a_persisted_db() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("d.sqlite3");
        {
            let mut s = Store::open(&path).unwrap();
            s.save_check(&sample(), "pdf", "T").unwrap();
        }
        // Reopen: migrate must run again without error and data persists.
        let s = Store::open(&path).unwrap();
        assert!(s.latest_result("sha256:aaa").unwrap().is_some());
    }
```

- [ ] **Step 2: Run to confirm failure**

Run: `cargo nextest r store::tests::save_then_retrieve_structured_result`
Expected: FAIL (`latest_result` not found).

- [ ] **Step 3: Add the column, migration, storage, and accessor**

In `src-tauri/src/store.rs`:

(a) Add `result_json` to the `checks` table in the `execute_batch` schema — change the `report_text TEXT NOT NULL` line in the `CREATE TABLE IF NOT EXISTS checks` block to:

```sql
                report_text TEXT NOT NULL,
                result_json TEXT NOT NULL DEFAULT ''
```

(b) At the end of `migrate`, before `Ok(())`, add a guarded `ALTER TABLE` so databases created by the previous version gain the column:

```rust
        let has_result_json: bool = self.conn.query_row(
            "SELECT COUNT(*) FROM pragma_table_info('checks') WHERE name = 'result_json'",
            [],
            |r| r.get::<_, i64>(0),
        )? > 0;
        if !has_result_json {
            self.conn
                .execute("ALTER TABLE checks ADD COLUMN result_json TEXT NOT NULL DEFAULT ''", [])?;
        }
```

(c) In `save_check`, store the serialised result. Change the `checks` INSERT to include `result_json`:

Replace the INSERT statement and its params:

```rust
        tx.execute(
            "INSERT INTO checks(document_id, run_at, total, checkable, resolved,
                 unresolved, with_discrepancies, missing_doi_flagged, report_text, result_json)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
            params![
                document_id,
                result.run_at,
                counts.total as i64,
                counts.checkable as i64,
                counts.resolved as i64,
                counts.unresolved as i64,
                counts.with_discrepancies as i64,
                counts.missing_doi_flagged as i64,
                report_text,
                serde_json::to_string(result).unwrap_or_default()
            ],
        )?;
```

(d) Add the accessor after `latest_report`:

```rust
    /// The most recent structured result for a document, by fingerprint.
    pub fn latest_result(
        &self,
        fingerprint: &str,
    ) -> Result<Option<crate::model::CheckResult>, StoreError> {
        let mut stmt = self.conn.prepare(
            "SELECT c.result_json FROM checks c
             JOIN documents d ON d.id = c.document_id
             WHERE d.fingerprint = ?1
             ORDER BY c.id DESC LIMIT 1",
        )?;
        let mut rows = stmt.query(params![fingerprint])?;
        match rows.next()? {
            Some(row) => {
                let json: String = row.get(0)?;
                Ok(serde_json::from_str(&json).ok())
            }
            None => Ok(None),
        }
    }
```

Note: `serde_json` is already a dependency; `CheckResult` already derives `Serialize`/`Deserialize`/`PartialEq`.

- [ ] **Step 4: Run to confirm pass**

Run: `cargo nextest r store::`
Expected: all store tests PASS.

- [ ] **Step 5: Commit**

```bash
cargo fmt && cargo clippy --all-targets -- -D warnings
jj fix && jj commit -m "Persist and retrieve the structured CheckResult"
```

---

## Task 9: CSV/JSON export rendering (`export.rs`)

**Files:** Create `src-tauri/src/export.rs`; modify `src-tauri/src/lib.rs`

- [ ] **Step 1: Create the module with a failing test**

Create `src-tauri/src/export.rs`:

```rust
//! Machine-readable exports of a CheckResult: full JSON and a flat CSV.

use crate::model::{CheckResult, EntryOutcome};

/// Lossless JSON of the whole result.
pub fn to_json(result: &CheckResult) -> String {
    serde_json::to_string_pretty(result).unwrap_or_default()
}

/// One row per entry: ordinal, doi, status, unmatched fields, suggested doi.
pub fn to_csv(result: &CheckResult) -> String {
    let mut out = String::from("ordinal,doi,status,unmatched_fields,suggested_doi\n");
    for e in &result.entries {
        let (status, unmatched, suggested) = match &e.outcome {
            EntryOutcome::Resolved { discrepancies, .. } if discrepancies.is_empty() => {
                ("clean".to_string(), String::new(), String::new())
            }
            EntryOutcome::Resolved { discrepancies, .. } => (
                "mismatch".to_string(),
                discrepancies
                    .iter()
                    .map(|d| d.field.as_str())
                    .collect::<Vec<_>>()
                    .join("; "),
                String::new(),
            ),
            EntryOutcome::Unresolved { network_error, .. } => (
                if *network_error { "network_error" } else { "not_found" }.to_string(),
                String::new(),
                String::new(),
            ),
            EntryOutcome::NoDoi { suggested } => (
                "no_doi".to_string(),
                String::new(),
                suggested.as_ref().map(|s| s.doi.clone()).unwrap_or_default(),
            ),
        };
        out.push_str(&format!(
            "{},{},{},{},{}\n",
            e.entry.ordinal,
            csv_field(e.entry.doi.as_deref().unwrap_or("")),
            status,
            csv_field(&unmatched),
            csv_field(&suggested),
        ));
    }
    out
}

/// Quote a CSV field if it contains a comma, quote, or newline.
fn csv_field(s: &str) -> String {
    if s.contains([',', '"', '\n']) {
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
                    entry: ReferenceEntry { ordinal: 1, raw_text: "r".into(), doi: Some("10.1000/a".into()) },
                    outcome: EntryOutcome::Resolved { doi: "10.1000/a".into(), discrepancies: vec![] },
                },
                CheckedEntry {
                    entry: ReferenceEntry { ordinal: 2, raw_text: "r".into(), doi: Some("10.1000/b".into()) },
                    outcome: EntryOutcome::Resolved {
                        doi: "10.1000/b".into(),
                        discrepancies: vec![Discrepancy { field: "year".into(), reference_value: "x".into(), crossref_value: "2020".into() }],
                    },
                },
                CheckedEntry {
                    entry: ReferenceEntry { ordinal: 3, raw_text: "r".into(), doi: None },
                    outcome: EntryOutcome::NoDoi { suggested: Some(SuggestedDoi { doi: "10.1000/c".into(), title_match: 90 }) },
                },
            ],
        }
    }

    #[test]
    fn csv_has_header_and_rows() {
        let csv = to_csv(&result());
        let lines: Vec<&str> = csv.lines().collect();
        assert_eq!(lines[0], "ordinal,doi,status,unmatched_fields,suggested_doi");
        assert_eq!(lines[1], "1,10.1000/a,clean,,");
        assert_eq!(lines[2], "2,10.1000/b,mismatch,year,");
        assert_eq!(lines[3], "3,,no_doi,,10.1000/c");
    }

    #[test]
    fn json_round_trips() {
        let json = to_json(&result());
        let back: CheckResult = serde_json::from_str(&json).unwrap();
        assert_eq!(back, result());
    }
}
```

Add to `src-tauri/src/lib.rs` (with the other `pub mod` lines):

```rust
pub mod export;
```

- [ ] **Step 2: Run the tests**

Run: `cargo nextest r export::`
Expected: both PASS.

- [ ] **Step 3: Commit**

```bash
cargo fmt && cargo clippy --all-targets -- -D warnings
jj fix && jj commit -m "Add CSV and JSON export rendering"
```

---

## Task 10: New command surface (`commands.rs`, `lib.rs`)

**Files:** Modify `src-tauri/src/commands.rs`, `src-tauri/src/lib.rs`

- [ ] **Step 1: Rewrite the command bodies**

In `src-tauri/src/commands.rs`:

(a) Change `open_document` to return the structured result:

```rust
/// Look up an already-seen document by file path; return the latest structured
/// result if present.
#[tauri::command]
pub fn open_document(
    state: State<'_, AppState>,
    path: String,
) -> Result<Option<crate::model::CheckResult>, String> {
    let ingested = crate::ingest::ingest(&PathBuf::from(&path)).map_err(map_err)?;
    let store = state.store.lock().map_err(|e| e.to_string())?;
    store.latest_result(&ingested.fingerprint).map_err(map_err)
}
```

(b) Replace `report_by_fingerprint` with `latest_check`:

```rust
/// The most recent structured result for a document, by fingerprint (sidebar).
#[tauri::command]
pub fn latest_check(
    state: State<'_, AppState>,
    fingerprint: String,
) -> Result<Option<crate::model::CheckResult>, String> {
    let store = state.store.lock().map_err(|e| e.to_string())?;
    store.latest_result(&fingerprint).map_err(map_err)
}
```

(c) Change `check_document` to return the structured result. Change its signature return type to `Result<crate::model::CheckResult, String>` and replace the final block:

```rust
    let report_text = crate::report::render(&result);
    let kind = match ingested.kind {
        crate::model::FileKind::Pdf => "pdf",
        crate::model::FileKind::Docx => "docx",
    };
    {
        let mut store = state.store.lock().map_err(|e| e.to_string())?;
        store
            .save_check(&result, kind, &report_text)
            .map_err(map_err)?;
    }
    Ok(result)
}
```

(d) Replace `export_report` with a format-aware version:

```rust
/// Write a stored report to `path` in the given format ("txt", "json", "csv").
#[tauri::command]
pub fn export_report(
    state: State<'_, AppState>,
    path: String,
    fingerprint: String,
    format: String,
) -> Result<(), String> {
    let store = state.store.lock().map_err(|e| e.to_string())?;
    let content = match format.as_str() {
        "txt" => store
            .latest_report(&fingerprint)
            .map_err(map_err)?
            .ok_or_else(|| "no report stored for this document".to_string())?,
        "json" => {
            let r = store
                .latest_result(&fingerprint)
                .map_err(map_err)?
                .ok_or_else(|| "no result stored for this document".to_string())?;
            crate::export::to_json(&r)
        }
        "csv" => {
            let r = store
                .latest_result(&fingerprint)
                .map_err(map_err)?
                .ok_or_else(|| "no result stored for this document".to_string())?;
            crate::export::to_csv(&r)
        }
        other => return Err(format!("unknown export format: {other}")),
    };
    std::fs::write(&path, content).map_err(map_err)
}
```

(e) Add reports-folder settings commands (after `set_email`):

```rust
#[tauri::command]
pub fn get_reports_dir(state: State<'_, AppState>) -> Result<Option<String>, String> {
    let store = state.store.lock().map_err(|e| e.to_string())?;
    store.get_setting("reports_dir").map_err(map_err)
}

#[tauri::command]
pub fn set_reports_dir(state: State<'_, AppState>, dir: String) -> Result<(), String> {
    let store = state.store.lock().map_err(|e| e.to_string())?;
    store.set_setting("reports_dir", &dir).map_err(map_err)
}
```

- [ ] **Step 2: Update the handler registration in `lib.rs`**

In `src-tauri/src/lib.rs`, replace the `invoke_handler(tauri::generate_handler![...])` list with:

```rust
        .invoke_handler(tauri::generate_handler![
            commands::list_documents,
            commands::get_email,
            commands::set_email,
            commands::get_reports_dir,
            commands::set_reports_dir,
            commands::open_document,
            commands::latest_check,
            commands::check_document,
            commands::export_report,
        ])
```

- [ ] **Step 3: Verify it builds and tests pass**

Run: `cd src-tauri && cargo build && cargo nextest r`
Expected: build succeeds; all existing tests still pass (commands have no unit tests).

- [ ] **Step 4: Commit**

```bash
cargo fmt && cargo clippy --all-targets -- -D warnings
jj fix && jj commit -m "Return structured results and add format-aware export commands"
```

---

## Task 11: Frontend API wrapper + result helpers

**Files:** Modify `src/lib/api.js`; create `src/lib/result.js`; install the opener JS binding

- [ ] **Step 1: Install the opener JS binding**

```bash
cd /Users/sth/dev/doicheck && npm install @tauri-apps/plugin-opener
```

- [ ] **Step 2: Rewrite `src/lib/api.js`**

```js
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

export const listDocuments = () => invoke("list_documents");
export const getEmail = () => invoke("get_email");
export const setEmail = (email) => invoke("set_email", { email });
export const getReportsDir = () => invoke("get_reports_dir");
export const setReportsDir = (dir) => invoke("set_reports_dir", { dir });
export const openDocument = (path) => invoke("open_document", { path });
export const latestCheck = (fingerprint) => invoke("latest_check", { fingerprint });
export const checkDocument = (path) => invoke("check_document", { path });
export const exportReport = (path, fingerprint, format) =>
  invoke("export_report", { path, fingerprint, format });
export const onProgress = (handler) => listen("progress", (e) => handler(e.payload));
```

- [ ] **Step 3: Create `src/lib/result.js`**

```js
// Helpers for interpreting a serialised CheckResult. EntryOutcome is an
// externally tagged enum, so each entry.outcome is an object with one of the
// keys "Resolved" | "Unresolved" | "NoDoi".

export function classify(entry) {
  const o = entry.outcome;
  if (o.Resolved) return o.Resolved.discrepancies.length ? "mismatch" : "clean";
  if (o.Unresolved) return o.Unresolved.network_error ? "network" : "unresolved";
  if (o.NoDoi) return o.NoDoi.suggested ? "no_doi_suggested" : "no_doi";
  return "clean";
}

export const SEVERITY = {
  unresolved: { label: "DOI not found on Crossref", colour: "#b00020", order: 0 },
  network: { label: "Check failed (network)", colour: "#b00020", order: 1 },
  mismatch: { label: "Metadata mismatch", colour: "#9a6700", order: 2 },
  no_doi_suggested: { label: "No DOI — suggestion available", colour: "#0a52c2", order: 3 },
  no_doi: { label: "No DOI found", colour: "#0a52c2", order: 4 },
  clean: { label: "Matched", colour: "#1a7f37", order: 5 },
};

// The DOI of an entry regardless of outcome (for links/copy).
export function entryDoi(entry) {
  const o = entry.outcome;
  if (o.Resolved) return o.Resolved.doi;
  if (o.Unresolved) return o.Unresolved.doi;
  return entry.entry.doi || "";
}

export function discrepancies(entry) {
  return entry.outcome.Resolved ? entry.outcome.Resolved.discrepancies : [];
}

export function suggestion(entry) {
  return entry.outcome.NoDoi ? entry.outcome.NoDoi.suggested : null;
}
```

- [ ] **Step 4: Verify the build**

Run: `cd /Users/sth/dev/doicheck && npm run build`
Expected: success (these are plain modules; consumers come next).

- [ ] **Step 5: Commit**

```bash
jj fix && jj commit -m "Add frontend API wrappers and CheckResult helpers"
```

---

## Task 12: Discrepancy display — EntryCard + ReportPane rewrite

**Files:** Create `src/lib/EntryCard.svelte`; rewrite `src/lib/ReportPane.svelte`; update `src/routes/+page.svelte`

- [ ] **Step 1: Create `src/lib/EntryCard.svelte`**

```svelte
<script>
  import { openUrl } from "@tauri-apps/plugin-opener";
  import { classify, SEVERITY, entryDoi, discrepancies, suggestion } from "$lib/result.js";

  let { entry } = $props();

  const kind = $derived(classify(entry));
  const sev = $derived(SEVERITY[kind]);
  const doi = $derived(entryDoi(entry));
  const discs = $derived(discrepancies(entry));
  const sugg = $derived(suggestion(entry));

  async function copy(text) {
    try {
      await navigator.clipboard.writeText(text);
    } catch (e) {
      console.error("copy failed", e);
    }
  }
</script>

<div class="card" style="border-left-color:{sev.colour}">
  <div class="head">
    <span class="badge" style="color:{sev.colour}">&#9679;</span>
    <span class="ord">[{entry.entry.ordinal}]</span>
    <span class="label" style="color:{sev.colour}">{sev.label}</span>
  </div>

  {#if entry.entry.raw_text}
    <p class="ref">{entry.entry.raw_text}</p>
  {/if}

  {#if discs.length}
    <ul class="fields">
      {#each discs as d (d.field)}
        <li><b>{d.field}:</b> Crossref says &ldquo;{d.crossref_value}&rdquo; &mdash; not found in your reference</li>
      {/each}
    </ul>
  {/if}

  {#if sugg}
    <p class="suggest">Closest Crossref match: <code>{sugg.doi}</code> ({sugg.title_match}%)
      <button onclick={() => copy(sugg.doi)}>copy</button></p>
  {/if}

  {#if doi}
    <div class="actions">
      <button onclick={() => openUrl(`https://doi.org/${doi}`)}>open DOI</button>
      <button onclick={() => copy(doi)}>copy DOI</button>
    </div>
  {/if}
</div>

<style>
  .card { border: 1px solid #eee; border-left-width: 3px; border-radius: 6px; padding: 8px 10px; margin-bottom: 8px; }
  .head { display: flex; align-items: center; gap: 6px; }
  .ord { font-weight: 600; }
  .label { font-size: 12px; }
  .ref { color: #444; margin: 4px 0; }
  .fields { margin: 4px 0; padding-left: 18px; }
  .fields li { margin: 2px 0; }
  .suggest code { font-family: ui-monospace, Menlo, monospace; }
  .actions { display: flex; gap: 6px; margin-top: 4px; }
  button { font: inherit; font-size: 12px; padding: 2px 8px; }
</style>
```

- [ ] **Step 2: Rewrite `src/lib/ReportPane.svelte`**

```svelte
<script>
  import { save } from "@tauri-apps/plugin-dialog";
  import { exportReport, getReportsDir, setReportsDir } from "$lib/api.js";
  import { classify, SEVERITY } from "$lib/result.js";
  import EntryCard from "$lib/EntryCard.svelte";

  let { result = null, busy = false, progress = null, currentPath = "", onopen, onrecheck } = $props();

  let filter = $state("all");
  let query = $state("");
  let showClean = $state(false);

  const classified = $derived(
    (result?.entries ?? []).map((e) => ({ entry: e, kind: classify(e) })),
  );
  const counts = $derived.by(() => {
    const c = { clean: 0, mismatch: 0, unresolved: 0, network: 0, no_doi: 0, no_doi_suggested: 0 };
    for (const x of classified) c[x.kind]++;
    return c;
  });
  const issues = $derived(
    classified
      .filter((x) => x.kind !== "clean")
      .filter((x) => filter === "all" || matchesFilter(x.kind, filter))
      .filter((x) => !query || JSON.stringify(x.entry).toLowerCase().includes(query.toLowerCase()))
      .sort((a, b) => SEVERITY[a.kind].order - SEVERITY[b.kind].order),
  );
  const cleanOnes = $derived(classified.filter((x) => x.kind === "clean"));

  function matchesFilter(kind, f) {
    if (f === "unresolved") return kind === "unresolved" || kind === "network";
    if (f === "no_doi") return kind === "no_doi" || kind === "no_doi_suggested";
    return kind === f;
  }

  async function pickAndCheck() {
    const { open } = await import("@tauri-apps/plugin-dialog");
    const path = await open({ multiple: false, filters: [{ name: "Documents", extensions: ["pdf", "docx"] }] });
    if (path) onopen?.(path);
  }

  function smartName(ext) {
    const stem = (result?.filename ?? "report").replace(/\.[^.]+$/, "");
    const date = new Date().toISOString().slice(0, 10);
    return `${stem}-doi-report-${date}.${ext}`;
  }

  async function doExport(format, ext) {
    if (!result) return;
    const dir = await getReportsDir();
    const path = await save({
      defaultPath: dir ? `${dir}/${smartName(ext)}` : smartName(ext),
      filters: [{ name: ext.toUpperCase(), extensions: [ext] }],
    });
    if (!path) return;
    await exportReport(path, result.fingerprint, format);
    const slash = Math.max(path.lastIndexOf("/"), path.lastIndexOf("\\"));
    if (slash > 0) await setReportsDir(path.slice(0, slash));
  }
</script>

<div class="toolbar">
  <button onclick={pickAndCheck} disabled={busy}>Open</button>
  <button onclick={() => onrecheck?.()} disabled={busy || !currentPath}>Re-check</button>
  <span class="spacer"></span>
  <button onclick={() => doExport("txt", "txt")} disabled={!result}>Save report</button>
  <button onclick={() => doExport("json", "json")} disabled={!result}>Export JSON</button>
  <button onclick={() => doExport("csv", "csv")} disabled={!result}>Export CSV</button>
</div>

{#if busy}
  <p class="progress">{progress ? `Checking ${progress.done} of ${progress.total}...` : "Working..."}</p>
{/if}

{#if result}
  <div class="summary">
    <button class:active={filter === "all"} onclick={() => (filter = "all")}>All issues {issues.length}</button>
    <button class:active={filter === "unresolved"} onclick={() => (filter = "unresolved")}>Unresolved {counts.unresolved + counts.network}</button>
    <button class:active={filter === "mismatch"} onclick={() => (filter = "mismatch")}>Mismatch {counts.mismatch}</button>
    <button class:active={filter === "no_doi"} onclick={() => (filter = "no_doi")}>No DOI {counts.no_doi + counts.no_doi_suggested}</button>
    <input placeholder="Search..." bind:value={query} />
  </div>
  {#if !result.bibliography_detected}
    <p class="note">No bibliography heading detected; results came from a whole-document scan.</p>
  {/if}

  {#each issues as x (x.entry.entry.ordinal)}
    <EntryCard entry={x.entry} />
  {/each}
  {#if issues.length === 0}
    <p class="note">No issues to show.</p>
  {/if}

  {#if cleanOnes.length}
    <button class="clean-toggle" onclick={() => (showClean = !showClean)}>
      {showClean ? "▾" : "▸"} {cleanOnes.length} entries matched cleanly
    </button>
    {#if showClean}
      {#each cleanOnes as x (x.entry.entry.ordinal)}
        <EntryCard entry={x.entry} />
      {/each}
    {/if}
  {/if}
{:else if !busy}
  <div class="empty">Open a PDF or .docx, or drop one on the window.</div>
{/if}

<style>
  .toolbar { display: flex; gap: 8px; align-items: center; margin-bottom: 12px; }
  .spacer { flex: 1; }
  button { font: inherit; padding: 4px 12px; }
  .summary { display: flex; gap: 6px; align-items: center; margin-bottom: 10px; flex-wrap: wrap; }
  .summary button { font-size: 12px; padding: 2px 10px; border-radius: 12px; border: 1px solid #ccc; background: #fff; }
  .summary button.active { border-color: #0a52c2; color: #0a52c2; }
  .summary input { margin-left: auto; padding: 3px 8px; font: inherit; }
  .clean-toggle { background: #f4faf5; color: #1a7f37; border: 1px solid #d6ecd9; border-radius: 6px; width: 100%; text-align: left; padding: 6px 10px; }
  .empty { color: #888; border: 2px dashed #ccc; border-radius: 8px; padding: 32px; text-align: center; }
  .note { color: #888; }
  .progress { color: #555; }
</style>
```

- [ ] **Step 3: Update `src/routes/+page.svelte` to carry the structured result**

Replace the `<script>` state and the three handlers (`runCheck`, `openPath`, `selectDocument`) and the `ReportPane` usage so the page holds a `result` object instead of a `report` string. Change the state declaration `let report = $state("");` to `let result = $state(null);`, and replace the three functions with:

```js
  async function runCheck(path) {
    error = "";
    busy = true;
    progress = null;
    currentPath = path;
    try {
      result = await api.checkDocument(path);
      await refresh();
    } catch (e) {
      error = String(e);
    } finally {
      busy = false;
      progress = null;
    }
  }

  async function openPath(path) {
    error = "";
    currentPath = path;
    try {
      const stored = await api.openDocument(path);
      if (stored) {
        result = stored;
      } else {
        await runCheck(path);
      }
    } catch (e) {
      error = String(e);
    }
  }

  async function selectDocument(fingerprint) {
    selectedFingerprint = fingerprint;
    const stored = await api.latestCheck(fingerprint);
    if (stored) result = stored;
  }
```

Then change the `<ReportPane ... />` usage from `{report}` to `{result}`:

```svelte
    <ReportPane
      {result}
      {busy}
      {progress}
      {currentPath}
      onopen={openPath}
      onrecheck={() => currentPath && runCheck(currentPath)}
    />
```

(The `onMount` drag-drop/progress wiring is unchanged.)

- [ ] **Step 4: Verify the build**

Run: `cd /Users/sth/dev/doicheck && npm run build`
Expected: success.

- [ ] **Step 5: Commit**

```bash
jj fix && jj commit -m "Rebuild discrepancy display with per-entry cards and filters"
```

---

## Task 13: Settings — Crossref email plus reports folder

**Files:** Modify `src/lib/Settings.svelte`

- [ ] **Step 1: Update `src/lib/Settings.svelte`**

```svelte
<script>
  import { onMount } from "svelte";
  import { getEmail, setEmail, getReportsDir, setReportsDir } from "$lib/api.js";
  import { open } from "@tauri-apps/plugin-dialog";

  let { onclose } = $props();
  let email = $state("");
  let reportsDir = $state("");

  onMount(async () => {
    email = await getEmail();
    reportsDir = (await getReportsDir()) ?? "";
  });

  async function pickDir() {
    const dir = await open({ directory: true });
    if (dir) reportsDir = dir;
  }

  async function saveAndClose() {
    await setEmail(email);
    await setReportsDir(reportsDir);
    onclose?.();
  }
</script>

<div class="backdrop" role="presentation" onclick={() => onclose?.()}></div>
<div class="sheet">
  <h3>Settings</h3>
  <label>Crossref contact email
    <input bind:value={email} type="email" placeholder="you@example.com" />
  </label>
  <p class="hint">Used for the Crossref polite pool. Leave blank to stay anonymous.</p>
  <label>Default reports folder
    <span class="row">
      <input bind:value={reportsDir} placeholder="(ask each time)" />
      <button onclick={pickDir}>Choose...</button>
    </span>
  </label>
  <p class="hint">Save dialogs will default to this folder.</p>
  <div class="actions">
    <button onclick={() => onclose?.()}>Cancel</button>
    <button class="primary" onclick={saveAndClose}>Save</button>
  </div>
</div>

<style>
  .backdrop { position: fixed; inset: 0; background: rgba(0,0,0,0.2); }
  .sheet { position: fixed; top: 16%; left: 50%; transform: translateX(-50%); background: #fff; border-radius: 10px; padding: 20px; width: 420px; box-shadow: 0 10px 40px rgba(0,0,0,0.2); }
  label { display: block; font-size: 12px; color: #555; margin-top: 8px; }
  input { width: 100%; box-sizing: border-box; margin-top: 4px; padding: 6px; font: inherit; }
  .row { display: flex; gap: 6px; }
  .row input { flex: 1; }
  .hint { color: #888; font-size: 11px; }
  .actions { display: flex; justify-content: flex-end; gap: 8px; margin-top: 12px; }
  .primary { background: #0a84ff; color: #fff; border: 0; border-radius: 6px; padding: 5px 14px; }
  button { font: inherit; }
</style>
```

- [ ] **Step 2: Verify the build**

Run: `cd /Users/sth/dev/doicheck && npm run build`
Expected: success.

- [ ] **Step 3: Commit**

```bash
jj fix && jj commit -m "Add reports-folder setting to Settings"
```

---

## Task 14: Full verification

**Files:** none (verification only)

- [ ] **Step 1: Backend suite + lint**

Run: `cd src-tauri && cargo nextest r && cargo clippy --all-targets -- -D warnings`
Expected: all tests pass; clippy clean.

- [ ] **Step 2: Frontend build**

Run: `cd /Users/sth/dev/doicheck && npm run build`
Expected: success.

- [ ] **Step 3: Release compile (end-to-end)**

Run: `cd src-tauri && cargo build --release`
Expected: success.

- [ ] **Step 4: Interactive check (manual, by the user)**

Run `npm run tauri dev`, drop the Barcelona term-paper PDF, and confirm: the bibliography is now detected (not "n/a"); the great majority of entries are clean; the unresolved DOI and the two no-DOI entries are flagged; opening DOIs and copy work; Save report / Export JSON / Export CSV write files; the reports folder is remembered.

- [ ] **Step 5: Commit (if any incidental fixes were needed)**

```bash
cargo fmt && cargo clippy --all-targets -- -D warnings
jj fix && jj commit -m "Final verification fixes for detection and display"
```

---

## Self-review notes (addressed)

- **Spec coverage:** heading relaxation (Task 2), author–date/numbered segmentation + de-wrap (Task 3), DOI-window fallback (Tasks 1, 4), comparability guard (Tasks 5, 7), HTML unescape (Task 6), structured persistence + retrieval (Task 8), JSON/CSV export (Task 9), new command surface incl. reports_dir (Task 10), API + result helpers (Task 11), per-entry card display with severity/filters/actions (Task 12), smart filename + remembered folder + JSON/CSV save UI (Task 12 toolbar + Task 13), committed fixture regression test (Task 3), structured-result round-trip and migration idempotency (Task 8).
- **Type/name consistency:** `extract_with_context`, `is_comparable`, `latest_result`, `latest_check`, `export_report(path, fingerprint, format)`, `result_json`, and the externally-tagged `EntryOutcome` keys (`Resolved`/`Unresolved`/`NoDoi`) are used consistently across backend and frontend.
- **Known follow-ups (not in scope):** field-level parsing of the reference (deferred per the spec); Crossref retry/backoff/429 (carried over from the first iteration's follow-ups).
