# Bibliography boundary parsing Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix two reference-list parsing failures — trailing matter (a `DECLARATION`) absorbed into the last reference, and adjacent references conflated under an unrecognised `Resources` heading — plus the running-head contamination behind them.

**Architecture:** All changes are in `src-tauri/src/biblio.rs` and `src-tauri/src/doi.rs`. `biblio::detect` first strips repeating running heads/page numbers, then either segments the section after a (now-broader) heading that ends at a (now-broader) set of terminators, or falls back to DOI-context windows that are trimmed to a single reference.

**Tech Stack:** Rust 2024, `regex` crate (no lookbehind), `cargo nextest`. Spec: `docs/superpowers/specs/2026-05-21-bibliography-boundary-parsing-design.md`.

---

## Conventions for every task

- Work in `/Users/sth/dev/doicheck/src-tauri`.
- Run tests with `cargo nextest run <filter>`.
- Before each commit: `cargo fmt`, then `cargo clippy --all-targets` (fix warnings), then `jj fix`, then `jj commit -m "[WIP: claude] <message>"`.
- Tests live in the existing `#[cfg(test)] mod tests` blocks of the file under change.

---

## Task 1: End-of-references terminators

Stop the reference section before a trailing `Declaration` / `Statement of …` /
`Acknowledgements` / author-biography section, while keeping the loose appendix
matcher and not truncating a citation that merely starts with such a word.

**Files:**
- Modify: `src-tauri/src/biblio.rs:19-20` (`END_HEADING_RE`)
- Test: `src-tauri/src/biblio.rs` (tests module)

- [ ] **Step 1: Write the failing tests**

Add to the `tests` module in `src-tauri/src/biblio.rs`:

```rust
// A DECLARATION section after the last reference must not be absorbed into it.
#[test]
fn section_stops_at_declaration() {
    let text = "References\n\
Yi, H. and Lim, J. (2020) 'Health equity'. Journal of Travel Medicine. https://doi.org/10.1093/jtm/taaa159\n\
DECLARATION\n\
AI tools were used to proofread for grammar.\n\
Name: A Student\n";
    let bib = detect(text);
    assert!(bib.detected);
    let last = bib.entries.last().unwrap();
    assert!(last.raw_text.contains("Yi"));
    assert!(!last.raw_text.contains("DECLARATION"));
    assert!(!last.raw_text.to_lowercase().contains("proofread"));
    assert_eq!(last.doi.as_deref(), Some("10.1093/jtm/taaa159"));
}

// A reference whose title begins "Declaration of ..." is NOT a heading and must
// not truncate the list.
#[test]
fn reference_titled_declaration_not_truncated() {
    let text = "References\n\
Declaration of Helsinki (2013) 'Ethical principles'. https://doi.org/10.1001/jama.2013.281053\n\
Zull, A. (2020) 'Last ref'. https://doi.org/10.1/last\n";
    let bib = detect(text);
    assert!(bib.detected);
    assert_eq!(bib.entries.len(), 2);
    assert!(bib.entries.iter().any(|e| e.doi.as_deref() == Some("10.1/last")));
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo nextest run section_stops_at_declaration reference_titled_declaration_not_truncated`
Expected: `section_stops_at_declaration` FAILS (the entry still contains "DECLARATION"/"proofread"). `reference_titled_declaration_not_truncated` should PASS already (sanity).

- [ ] **Step 3: Extend `END_HEADING_RE`**

Replace `src-tauri/src/biblio.rs:19-20`:

```rust
static END_HEADING_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?im)^\s*(?:appendix|appendices)\b").unwrap());
```

with:

```rust
// Headings that mark the end of the bibliography (the start of a later section),
// so trailing matter is not treated as references. The appendix matcher stays
// loose (a title may follow on the same line); the remaining terminators must be
// the whole line, so a citation that merely begins with the word (e.g.
// "Declaration of Helsinki (2013) ...") is not mistaken for a heading.
static END_HEADING_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?im)^\s*(?:appendix|appendices)\b|^\s*(?:declarations?|statement\s+of\s+\w+(?:\s+\w+)*|acknowledge?ments?|about\s+the\s+authors?|author\s+biograph\w+|biograph(?:y|ies)|biographical\s+notes?)\s*[:.]?\s*$",
    )
    .unwrap()
});
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo nextest run section_stops_at_declaration reference_titled_declaration_not_truncated section_stops_at_appendix`
Expected: all PASS (including the pre-existing `section_stops_at_appendix`).

- [ ] **Step 5: Commit**

```bash
cargo fmt && cargo clippy --all-targets && jj fix
jj commit -m "[WIP: claude] End references section at declaration/statement/biography headings"
```

---

## Task 2: Recognise `Resources`/`Sources` headings

**Files:**
- Modify: `src-tauri/src/biblio.rs:7-15` (`HEADING_RE`)
- Test: `src-tauri/src/biblio.rs` (tests module)

- [ ] **Step 1: Write the failing tests**

Add to the `tests` module:

```rust
// "Resources"/"Sources" are heading synonyms used by some student papers.
#[test]
fn detects_resources_and_sources_headings() {
    let r = detect("Body.\n\nResources\nSmith, J. (2020) A study. https://doi.org/10.1/x\n");
    assert!(r.detected);
    let s = detect("Body.\n\nSources\nSmith, J. (2020) A study. https://doi.org/10.1/x\n");
    assert!(s.detected);
}

// Under a recognised heading, adjacent author-date references split apart even
// when the first carries no DOI.
#[test]
fn resources_heading_splits_adjacent_references() {
    let text = "Resources\n\
Atkinson, R. & Easthope, H. (2008) 'Creative Class'. https://www.jstor.org/stable/23289786\n\
Black, J. (2026) 'Towards Net-Zero'. https://doi.org/10.7916/qbtt-xa42\n";
    let bib = detect(text);
    assert!(bib.detected);
    assert_eq!(bib.entries.len(), 2);
    assert!(bib.entries[0].raw_text.contains("Atkinson"));
    assert!(bib.entries[1].raw_text.contains("Black"));
    assert_eq!(bib.entries[1].doi.as_deref(), Some("10.7916/qbtt-xa42"));
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo nextest run detects_resources_and_sources_headings resources_heading_splits_adjacent_references`
Expected: FAIL (`detected` is false because "Resources"/"Sources" are not matched, so the fallback path runs and entry counts/text differ).

- [ ] **Step 3: Add the synonyms to `HEADING_RE`**

In `src-tauri/src/biblio.rs:11-14`, change the keyword alternation. Replace:

```rust
        r"(?im)^\s*(?:\d+\.?\s+|[ivxlcdm]+\.?\s+)?(references|reference list|bibliography|works cited|literature cited)\s*[:.]?\s*$",
```

with:

```rust
        r"(?im)^\s*(?:\d+\.?\s+|[ivxlcdm]+\.?\s+)?(references|reference list|bibliography|works cited|literature cited|resources|sources)\s*[:.]?\s*$",
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo nextest run detects_resources_and_sources_headings resources_heading_splits_adjacent_references heading_allows_section_number_but_not_toc`
Expected: all PASS (including the pre-existing TOC-rejection test).

- [ ] **Step 5: Commit**

```bash
cargo fmt && cargo clippy --all-targets && jj fix
jj commit -m "[WIP: claude] Recognise Resources/Sources as bibliography headings"
```

---

## Task 3: Strip running heads and page numbers

Remove repeating running-page-headers (e.g. `Anderson 9`) and bare page numbers
before segmentation, applied to the whole document in `detect`. This also fixes
the corrupted-DOI symptom: a bare page number after a DOI is otherwise glued onto
it by `continues_url` (`10.1093/jtm/taaa159` + `14` -> `…taaa15914`).

**Files:**
- Modify: `src-tauri/src/biblio.rs` (add `running_head_parts` + `strip_running_heads`; call in `detect`)
- Test: `src-tauri/src/biblio.rs` (tests module)

- [ ] **Step 1: Write the failing tests**

Add to the `tests` module:

```rust
// Repeating headers/page numbers are removed; one-offs and 4-digit numbers stay.
#[test]
fn strip_running_heads_keeps_one_offs() {
    let text = "Anderson 1\nAnderson 2\nAnderson 3\nSmart Cities Plan 2016\nReport 7\n1\n2\n3\nBody text here.\n";
    let cleaned = strip_running_heads(text);
    assert!(!cleaned.contains("Anderson"));
    assert!(cleaned.contains("Smart Cities Plan 2016"));
    assert!(cleaned.contains("Report 7"));
    assert!(cleaned.contains("Body text here."));
    assert!(!cleaned.lines().any(|l| l.trim() == "1"));
}

// A recurring running header between references is not glued onto an entry.
#[test]
fn running_header_not_glued_to_reference() {
    let text = "References\n\
Albino, V. (2015) 'Smart Cities'. https://doi.org/10.1/albino\n\
Anderson 9\n\
Atkinson, R. (2008) 'Creative Class'. https://www.jstor.org/stable/23289786\n\
Anderson 10\n\
Black, J. (2026) 'Net-Zero'. https://doi.org/10.1/black\n\
Anderson 11\n";
    let bib = detect(text);
    assert!(bib.detected);
    assert!(bib.entries.iter().all(|e| !e.raw_text.contains("Anderson")));
}

// A bare page number after a DOI must not corrupt the DOI.
#[test]
fn page_number_does_not_corrupt_doi() {
    let text = "References\n\
Aaa, B. (2019) 'First'. https://doi.org/10.1/aaa\n\
12\n\
Yi, H. (2020) 'Health equity'. Journal, 28(2), taaa159. https://doi.org/10.1093/jtm/taaa159\n\
13\n\
Zzz, C. (2021) 'Third'. https://doi.org/10.1/zzz\n\
14\n";
    let bib = detect(text);
    assert!(bib.detected);
    let yi = bib.entries.iter().find(|e| e.raw_text.contains("Yi")).unwrap();
    assert_eq!(yi.doi.as_deref(), Some("10.1093/jtm/taaa159"));
    assert!(!yi.raw_text.contains("taaa15914"));
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo nextest run strip_running_heads_keeps_one_offs running_header_not_glued_to_reference page_number_does_not_corrupt_doi`
Expected: `strip_running_heads_keeps_one_offs` FAILS to compile (function does not exist yet); the other two FAIL on assertions.

- [ ] **Step 3: Add the helpers**

Add these two functions to `src-tauri/src/biblio.rs` (e.g. just above `detect`):

```rust
/// Split a line into a (lowercased prefix, page-number) candidate for a running
/// header. The prefix may be empty (a bare page number). Returns `None` when the
/// line is not header-shaped: the trailing token must be a 1-3 digit integer (a
/// page number, not a year), and the prefix must be short (a header, not a
/// reference line).
fn running_head_parts(line: &str) -> Option<(String, &str)> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return None;
    }
    let (prefix, num) = match trimmed.rsplit_once(char::is_whitespace) {
        Some((p, n)) => (p.trim(), n),
        None => ("", trimmed),
    };
    if num.len() > 3 || num.is_empty() || !num.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    if prefix.chars().count() > 30 {
        return None;
    }
    Some((prefix.to_lowercase(), num))
}

/// Remove running page-headers and bare page numbers. A header-shaped line is
/// dropped only when its prefix recurs on at least three lines with differing
/// numbers, which marks a repeating header/footer rather than reference content.
fn strip_running_heads(text: &str) -> String {
    use std::collections::{HashMap, HashSet};
    let mut groups: HashMap<String, HashSet<&str>> = HashMap::new();
    for line in text.lines() {
        if let Some((prefix, num)) = running_head_parts(line) {
            groups.entry(prefix).or_default().insert(num);
        }
    }
    let strip: HashSet<String> = groups
        .into_iter()
        .filter(|(_, nums)| nums.len() >= 3)
        .map(|(prefix, _)| prefix)
        .collect();
    let mut out = String::with_capacity(text.len());
    for line in text.lines() {
        let drop = running_head_parts(line).is_some_and(|(prefix, _)| strip.contains(&prefix));
        if !drop {
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}
```

- [ ] **Step 4: Call it from `detect`**

In `src-tauri/src/biblio.rs`, change the start of `detect` (currently
`pub fn detect(text: &str) -> Bibliography {` then
`if let Some(section) = section_after_heading(text) {`) so it cleans first and
operates on the owned cleaned text. Replace the function body's two references to
`text` in the detection branches with `&cleaned`:

```rust
pub fn detect(text: &str) -> Bibliography {
    let cleaned = strip_running_heads(text);
    if let Some(section) = section_after_heading(&cleaned) {
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
        let entries = crate::doi::extract_with_context(&cleaned)
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

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo nextest run strip_running_heads_keeps_one_offs running_header_not_glued_to_reference page_number_does_not_corrupt_doi`
Expected: all PASS.

- [ ] **Step 6: Run the whole biblio module to check for regressions**

Run: `cargo nextest run biblio`
Expected: all PASS (existing tests are unaffected — none contain three-plus repeating header-shaped lines).

- [ ] **Step 7: Commit**

```bash
cargo fmt && cargo clippy --all-targets && jj fix
jj commit -m "[WIP: claude] Strip repeating running heads and page numbers before segmentation"
```

---

## Task 4: Trim no-heading fallback windows to a single reference

When no heading is detected, each DOI's context window can swallow a preceding
DOI-less reference. Trim each window to begin at the last reference-start within
it.

**Files:**
- Modify: `src-tauri/src/biblio.rs:54` (`is_entry_start` visibility)
- Modify: `src-tauri/src/doi.rs:45-67` (`extract_with_context`) and add `trim_to_last_entry_start`
- Test: `src-tauri/src/doi.rs` (tests module)

- [ ] **Step 1: Write the failing test**

Add to the `tests` module in `src-tauri/src/doi.rs`:

```rust
// A window must not include a prior reference that happened to lack a DOI.
#[test]
fn context_excludes_prior_doiless_reference() {
    let text = "Atkinson, R. & Easthope, H. (2008) 'Creative Class'. https://www.jstor.org/stable/23289786\n\
Black, J. (2026) 'Towards Net-Zero'. https://doi.org/10.7916/qbtt-xa42\n";
    let v = extract_with_context(text);
    assert_eq!(v.len(), 1);
    assert_eq!(v[0].0, "10.7916/qbtt-xa42");
    assert!(v[0].1.contains("Black"));
    assert!(!v[0].1.contains("Atkinson"));
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo nextest run context_excludes_prior_doiless_reference`
Expected: FAIL (the context still contains "Atkinson").

- [ ] **Step 3: Make `is_entry_start` crate-visible**

In `src-tauri/src/biblio.rs:54`, change:

```rust
fn is_entry_start(line: &str) -> bool {
```

to:

```rust
pub(crate) fn is_entry_start(line: &str) -> bool {
```

- [ ] **Step 4: Trim the window in `extract_with_context`**

In `src-tauri/src/doi.rs`, replace the window-building lines inside the loop
(currently lines 56-61):

```rust
        let lower_bound = m.start().saturating_sub(WINDOW).max(prev_end);
        let start = snap_char_boundary(text, lower_bound);
        let context = text[start..m.end()]
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
```

with:

```rust
        let lower_bound = m.start().saturating_sub(WINDOW).max(prev_end);
        let start = snap_char_boundary(text, lower_bound);
        let window = trim_to_last_entry_start(&text[start..m.end()]);
        let context = window.split_whitespace().collect::<Vec<_>>().join(" ");
```

Add this helper to `src-tauri/src/doi.rs` (e.g. just below `snap_char_boundary`):

```rust
/// Trim a context window to start at the last line that begins a reference entry,
/// so the window holds only the reference that owns the DOI. If no entry start is
/// found, return the whole window unchanged.
fn trim_to_last_entry_start(window: &str) -> &str {
    let mut best = 0usize;
    let mut found = false;
    let mut offset = 0usize;
    for line in window.split_inclusive('\n') {
        if crate::biblio::is_entry_start(line) {
            best = offset;
            found = true;
        }
        offset += line.len();
    }
    if found { &window[best..] } else { window }
}
```

- [ ] **Step 5: Run the test to verify it passes**

Run: `cargo nextest run context_excludes_prior_doiless_reference`
Expected: PASS.

- [ ] **Step 6: Run the whole doi + biblio modules for regressions**

Run: `cargo nextest run doi biblio`
Expected: all PASS — in particular the pre-existing `extract_with_context_windows_each_doi` (its single-line input has no `\n`, so trimming returns the whole window) and `no_heading_falls_back_to_doi_windows`.

- [ ] **Step 7: Commit**

```bash
cargo fmt && cargo clippy --all-targets && jj fix
jj commit -m "[WIP: claude] Trim no-heading fallback windows to a single reference"
```

---

## Task 5: Full-suite verification and out-of-tree check on the real PDFs

- [ ] **Step 1: Run the full test suite**

Run: `cargo nextest run`
Expected: all PASS.

- [ ] **Step 2: Lint/format clean**

Run: `cargo fmt --check && cargo clippy --all-targets`
Expected: no diffs, no warnings.

- [ ] **Step 3: Verify against the two real PDFs out-of-tree**

Recreate the throwaway harness used during diagnosis and confirm the fix, then
remove it so the tree stays clean:

```bash
cat > examples/dump_bib.rs <<'EOF'
use doicheck_lib::{biblio, extract};
fn main() {
    let path = std::env::args().nth(1).expect("usage: dump_bib <pdf>");
    let bytes = std::fs::read(&path).expect("read file");
    let text = extract::pdf::extract(&bytes).expect("extract");
    let bib = biblio::detect(&text);
    println!("detected={} entries={}", bib.detected, bib.entries.len());
    for e in &bib.entries {
        println!("[{:>3}] doi={:?}\n      {}", e.ordinal, e.doi.as_deref(), e.raw_text);
    }
}
EOF
export PDFIUM_LIB_DIR="$PWD/pdfium"
cargo run -q --example dump_bib -- "/Users/sth/Documents/TCD Teaching 2026/DP8017, Smart and Sustainable Eco-Cities/Term Papers/Term Paper_chlong_attempt_2026-04-24-21-33-32_Final Term Paper (Singapore).pdf" | grep -iE "declaration|taaa159"
cargo run -q --example dump_bib -- "/Users/sth/Documents/TCD Teaching 2026/DP8017, Smart and Sustainable Eco-Cities/Term Papers/Term Paper_ianderso_attempt_2026-04-24-20-39-34_Term Paper, SEC.pdf" | grep -iE "detected=|Atkinson|Black, J"
rm examples/dump_bib.rs
```

Expected:
- chlong: the last Yi entry shows DOI `10.1093/jtm/taaa159` (not `…taaa15914`) and no `DECLARATION` text.
- ianderso: `detected=true`; Atkinson (2008) and Black (2026) are separate entries; no `Anderson N` text glued on.

- [ ] **Step 4: Confirm the tree is clean**

Run: `jj st`
Expected: only the committed source changes (no `examples/dump_bib.rs`).

---

## Self-review notes

- **Spec coverage:** Goal 1 → Task 1; Goal 2 → Task 2; Goal 3 → Task 3; Goal 4 →
  Task 4. Non-goal "no real PDFs as fixtures" honoured (synthetic tests; harness
  removed in Task 5).
- **Type consistency:** `is_entry_start` is referenced as `crate::biblio::is_entry_start`
  after being made `pub(crate)` (Task 4 Steps 3-4). `strip_running_heads`/
  `running_head_parts` are private, used by `detect` and the biblio tests in the
  same module. `trim_to_last_entry_start` is private to `doi.rs`.
- **Ordering:** Task 3 wires `strip_running_heads` into `detect`; the Task 1/2
  tests do not contain repeating header-shaped lines, so they are unaffected when
  Task 3 lands.
