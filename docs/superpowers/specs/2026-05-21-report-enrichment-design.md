# Report enrichment design

Date: 2026-05-21

## Problem

The reporting output is harder to act on than it needs to be:

1. **Opaque entry identifiers.** The text report and CSV identify each flagged
   reference only by its ordinal (e.g. `[12]`). A reader cannot tell which
   reference that is without counting through the bibliography.
2. **Ambiguous yellow status dot.** The sidebar document dot shows yellow
   (`has-issues`) whenever there are mismatches *or* DOIs not found on Crossref.
   These differ in severity: a metadata mismatch is a minor discrepancy, while a
   DOI that does not exist on Crossref may be wrong or fabricated.
3. **Retry state absent from exports.** Transient failures (timeout, backoff,
   5xx) are surfaced in the UI (orange `↻` sidebar dot, "Re-check failures"
   button, warning banner) but not in the text or CSV exports, so a saved report
   does not show what still needs re-checking.

## Current behaviour (verified)

- `EntryOutcome` (`src-tauri/src/model.rs`) cleanly separates failure kinds:
  - 404 from Crossref → `CrossrefError::NotFound` → `Unresolved { network_error: false }`
    → counted in `Counts::unresolved` ("DOI not found on Crossref").
  - timeout / backoff / 5xx (after retries) → `CrossrefError::Network` →
    `Unresolved { network_error: true }` → counted in `Counts::network_failed`
    ("needs retry"). 404 is deliberately **not** retry-needed.
- Text report (`src-tauri/src/report.rs`): each issue line is
  `[ordinal] DOI  field: ref X vs Crossref "Y"`; no reference text.
- CSV (`src-tauri/src/export.rs`): columns
  `ordinal,doi,status,unmatched_fields,suggested_doi,llm_source`; `status` already
  emits a distinct `network_error` value for transient failures.
- JSON (`src-tauri/src/export.rs`): lossless serialisation of `CheckResult`;
  already contains `raw_text` and the `network_error` flag. **No JSON change.**
- Sidebar dot (`src/lib/Sidebar.svelte`): maps statuses
  `incomplete`→orange `↻`, `has-issues`→yellow `●`, `failed`→red `●`,
  default→green `●`. The `failed`→red branch already exists but is never emitted
  by the backend.
- Document status (`src-tauri/src/store.rs::list_documents`): currently
  `incomplete` (if `network_failed > 0`) → `has-issues`
  (if `with_discrepancies > 0 || unresolved > 0`) → `clean`.
- Per-entry severity (`src/lib/result.js::SEVERITY`): already distinguishes
  `unresolved` (red) from `mismatch` (yellow). Entry cards are not changing.

## Goals

1. Add a human-readable reference snippet to the text report and CSV.
2. Make the document status dot distinguish "DOI not found on Crossref" (red)
   from "metadata mismatch" (yellow).
3. Surface the transient "needs retry" state in the text report and CSV.

## Non-goals

- No change to JSON output (already complete).
- No per-entry card "needs retry" marker (entry cards stay as they are).
- No new document-level summary metric line for retry counts (a text *note*
  line is in scope; a formal summary counter is not).
- No change to how failures are classified in the pipeline/Crossref client.
- No reference-text parsing into author/year/title; the raw snippet is used.

## Design

### 1. Reference snippet (text + CSV)

New helper, e.g. `snippet(raw_text: &str) -> String`:

- Collapse runs of whitespace (including newlines) to single spaces, trim.
- Truncate to ~80 characters on a char boundary; append `…` when truncated.

**Text report** — each listed entry uses a two-line layout: ordinal + snippet on
line 1, the existing detail indented beneath. Multiple discrepancies for one
entry are grouped under a single snippet line.

```
Discrepancies
  [12] Smith, J. et al. (2020). Neural things in modern journals. Journal of…
       10.1/yyy  title: ref "(title not found in reference)" vs Crossref "Neural Things"
  [45] Jones, A. (2019). A paper whose DOI does not exist…
       10.2/zzz  DOI not found on Crossref
  [46] Brown, B. (2021). A paper we could not reach…
       10.3/www  could not be checked — retry needed

Possibly missing DOIs
  [33] Lee, C. (2018). Untitled work without a DOI…
       no DOI; closest Crossref match 10.1000/xyz (title match 82%)
```

The "POSSIBLE AI SOURCE" sub-line is retained, indented under its entry.

**CSV** — add a `reference_text` column as column 2, carrying the *full*
(untruncated) `raw_text`, quoted via the existing `csv_field` helper. New header:

```
ordinal,reference_text,doi,status,unmatched_fields,suggested_doi,llm_source
```

### 2. Status dot: red for not-found, yellow for mismatch

`store.rs::list_documents` status precedence becomes:

1. `network_failed > 0` → `incomplete` (orange `↻`) — partial result, re-check first.
2. else `unresolved - network_failed > 0` → `failed` (red `●`) — genuine not-found.
3. else `with_discrepancies > 0` → `has-issues` (yellow `●`) — mismatch only.
4. else → `clean` (green `●`).

`Counts::unresolved` already includes both kinds and `Counts::network_failed` is
the transient subset, so genuine not-found is `unresolved - network_failed`;
no `Counts` change is required.

Frontend (`src/lib/Sidebar.svelte`): the `failed`→red branch already exists;
update only its tooltip text from "Check failed" to
"DOI not found on Crossref".

### 3. Retry state in text + CSV

**Text** (`report.rs`):

- Reword the transient per-entry detail from `check failed (network)` to
  `could not be checked — retry needed`.
- Reword the genuine 404 detail to `DOI not found on Crossref`.
- When `network_failed > 0`, emit a note line after the Summary block listing
  affected ordinals, e.g.
  `Note: 1 entry could not be checked (network or capacity) and should be re-checked: [46]`
  (pluralise "entry"/"entries" correctly).

**CSV** (`export.rs`):

- Rename the transient `status` value from `network_error` to `retry_needed`.
  The genuine-not-found value stays `not_found`. (CSV is not a stable API; JSON
  retains the `network_error` boolean.)

## Files to change

- `src-tauri/src/report.rs` — snippet helper or call; two-line layout; reworded
  details; retry note line; update tests.
- `src-tauri/src/export.rs` — `reference_text` column; `retry_needed` rename;
  update tests.
- `src-tauri/src/store.rs` — `failed` status precedence; update tests.
- `src/lib/Sidebar.svelte` — `failed` tooltip text.
- The snippet helper lives in `report.rs` (or a small shared spot if `export.rs`
  also needs truncation; CSV uses full text, so likely report-only).

## Testing

- `report.rs`: snippet truncation (short string unchanged, long string truncated
  with `…`, whitespace collapsed); two-line layout substrings; `retry needed`
  vs `DOI not found on Crossref` wording; retry note line present/absent and
  pluralised.
- `export.rs`: CSV header includes `reference_text`; row carries quoted full
  text; transient status renders `retry_needed`; not-found renders `not_found`;
  JSON round-trip unchanged.
- `store.rs`: `failed` when a genuine not-found exists with no transient
  failures; `incomplete` still wins when a transient failure is also present;
  `has-issues` when only mismatches; `clean` otherwise.
- Frontend: `npm run build` compiles.

## Verification commands

- `cargo nextest r` (Rust tests)
- `cargo fmt` / `cargo clippy`
- `npm run build`
