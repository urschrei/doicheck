# Detection Hardening & Report Display — Design

Date: 2026-05-20
Status: Approved
Builds on: 2026-05-20-doi-checker-design.md

## Purpose

A second iteration that fixes a correctness problem found in real use and
improves how results are presented. On a real term-paper PDF the tool reported
19 of 20 entries as having discrepancies; all were false positives caused by the
no-bibliography fallback comparing Crossref metadata against a bare DOI string
rather than against reference text. This work hardens bibliography detection and
segmentation, makes comparison run against real reference text, exposes the
structured result to the UI, redesigns the discrepancy display, and improves the
save/export experience.

## Diagnosis (what prompted this)

Extracting the failing PDF with the app's own extractor showed:

- The references heading exists but came out as `" 6. References  "` (section
  numbered). The heading regex requires the line to be only the keyword, so the
  `6. ` prefix stopped it matching, and the whole-document DOI fallback ran.
- In the fallback (`pipeline.rs`), each synthesised entry's `raw_text` was set to
  the bare DOI, so every metadata field compared as "not found in reference".
- References are author–date with hanging-indent wrapping; PDF extraction puts
  blank lines both between and within entries, so blank-line splitting shreds
  entries.
- Some entries have no DOI (a handle.net link, a news URL); these are invisible
  in the DOI-only fallback.
- Crossref sometimes returns HTML-escaped values (e.g.
  `Science, Technology, &amp; Human Values`).

## Decisions

Settled during brainstorming:

- Segmentation: entry-start detection (numbered or author–date) with de-wrapping
  when a heading is found; DOI/URL-anchored windows as a last-resort fallback
  when no heading is detectable.
- Comparison: keep token-presence (full-text) comparison; do NOT parse the
  reference into fields. The display shows the full reference text plus the
  Crossref values whose tokens are absent from it. (Revisit field-level parsing
  later if needed.)
- Display: Layout A — per-entry cards, problems first, clean entries collapsed,
  severity colours, inline actions.
- Severity model: red = DOI does not resolve; amber = metadata not found in the
  reference; blue = entry has no DOI but a Crossref match is suggested; green =
  clean.
- Save: smart pre-filled filename defaulting to the last-used folder.
- Export formats: plain text plus machine-readable JSON and CSV.

## 1. Bibliography detection and segmentation (`biblio.rs`)

### Heading detection

Relax `HEADING_RE` to allow an optional leading section number or Roman numeral
before the keyword, while still rejecting table-of-contents lines (which carry
trailing dotted leaders and a page number). The keyword must still be effectively
the whole line. Continue selecting the last matching heading.

- Matches: `References`, `6. References`, `VI. Bibliography`, `Works Cited`.
- Does not match: `6. References .................. 13` (TOC), or in-body mentions.

### Entry segmentation

Replace blank-line splitting with entry-start detection:

- An entry begins at a line matching a numbered marker (`[n]`, `n.`, `n)`) OR an
  author–date opening, approximated by a capitalised author run followed by a
  parenthesised year, e.g. `^\p{Lu}[\p{L}'.,&\- ]+\(\d{4}[a-z]?\)`.
- All other lines (including blank and wrapped continuation lines) append to the
  current entry. Whitespace is collapsed per entry.
- This enumerates every entry, including those without a DOI.

### No-heading fallback

If no heading is detectable, build entries from DOI/URL-anchored windows: for
each DOI occurrence, take the preceding text back to the previous DOI/URL
boundary (bounded to a maximum window) as the entry's `raw_text`. Set
`bibliography_detected = false`.

## 2. Trustworthy comparison (`pipeline.rs`, `compare.rs`, `crossref.rs`)

- Entries carry their real reference text (segmented entry, or window) instead of
  a bare DOI. This removes the false positives at the source.
- Suppress meaningless comparisons: before comparing, check the entry text has
  real content (a helper such as `text::is_comparable(reference)` requiring a
  minimum count of alphanumeric, non-DOI characters). If not, record the DOI as
  resolved with an empty discrepancy list.
- HTML-unescape Crossref string values (title, author family, container) when
  building `Metadata`, so stored and displayed values are clean.
- No-DOI entries continue to use the Crossref title search to suggest a DOI; with
  correct segmentation these now appear in results.

## 3. Structured result to the UI (`store.rs`, `commands.rs`)

The display renders structured data, not the plain-text blob.

- Add a `result_json TEXT` column to `checks` (migration). `save_check` stores the
  serialised `CheckResult` alongside `report_text`.
- Command surface:
  - `check_document(path) -> CheckResult` (was `-> String`); still emits
    `progress` events and persists the check.
  - `open_document(path) -> Option<CheckResult>` (was `-> Option<String>`):
    fingerprint the file, return the latest structured result if seen.
  - `latest_check(fingerprint) -> Option<CheckResult>` (replaces
    `report_by_fingerprint`): sidebar selection.
  - `export_report(path, fingerprint, format)` where `format` is `txt`, `json`,
    or `csv`: writes from the stored check (txt from `report_text`, json from
    `result_json`, csv derived from the structured result).
  - Settings: keep `get_email`/`set_email`; add `get_reports_dir`/
    `set_reports_dir` (remembered save folder).
- The plain-text report continues to be rendered (`report.rs`) and stored.

## 4. Discrepancy display — Layout A (frontend)

`ReportPane` is split into focused components and renders a `CheckResult`.

- **Summary bar:** count chips (clean / unresolved / mismatch / no-DOI) acting as
  filters, plus a text search box.
- **Entry cards, problems first:** each flagged card shows its severity colour,
  ordinal, DOI, the reference text, the Crossref fields whose tokens are not found
  in the reference, and actions: open DOI (via the existing `opener` plugin) and
  copy DOI / copy suggested DOI (web Clipboard API, falling back to the clipboard
  plugin if unavailable).
- **Clean entries** collapse behind a single "N matched cleanly" expander.
- Components: `ReportPane` (summary + filter + list), `EntryCard`, `FieldList`,
  each small and focused.

## 5. Save and export UX (`ReportPane`, `commands.rs`, settings)

- **Save report (.txt):** dialog pre-filled with `<document-stem>-doi-report-
  YYYY-MM-DD.txt`, defaulting to the last-used folder (`reports_dir` in settings,
  updated after each save).
- **Export data:** JSON (full `CheckResult`, lossless) and CSV (one row per
  entry: ordinal, DOI, status, list of unmatched fields, suggested DOI). Rendered
  server-side from the stored result.

## 6. Testing

- Unit tests: relaxed heading detection (including `6. References` and the TOC
  line); author–date segmentation including wrapped/blank lines; numbered-marker
  segmentation still works; window fallback; `is_comparable` suppression; HTML
  unescape; CSV and JSON rendering.
- A committed PDF-derived text fixture modelled on the real failure: a
  `6. References` heading, two wrapped author–date entries with DOIs, and one
  entry with no DOI. Regression-tests detection end to end at the text level.
- Store: `result_json` round-trips; migration applies to a fresh and an existing
  database.
- Frontend: build-verified; interactive verification by the user.

## Files touched

- Backend: `biblio.rs` (heading + segmentation), `doi.rs` or a new helper
  (DOI windows), `pipeline.rs` (real reference text + suppression), `compare.rs`
  / `text.rs` (`is_comparable`), `crossref.rs` (unescape), `store.rs` (migration
  + `result_json` + `latest_check`/result accessors), `commands.rs` (new return
  types, export-by-format, reports_dir settings), `report.rs` (unchanged output;
  reused for txt export), a new small `export.rs` for CSV rendering.
- Frontend: `lib/api.js`, `routes/+page.svelte`, and new
  `lib/ReportPane.svelte` (rewritten), `lib/EntryCard.svelte`, `lib/FieldList.svelte`,
  `lib/Settings.svelte` (add reports folder).

## Risks and notes

- Author–date segmentation is heuristic; an unusual entry opening may merge or
  split. The fixture and tests cover the common academic styles; messy PDFs may
  still need the window fallback.
- The display compares full reference text against Crossref values (no field-level
  parsing of the reference); accepted for now, revisit after use.
- CSV flattening loses nested detail; JSON is the lossless export.
