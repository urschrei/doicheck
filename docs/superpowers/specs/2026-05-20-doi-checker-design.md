# DOI Checker — Design

Date: 2026-05-20
Status: Approved

## Purpose

A desktop application that takes a single PDF or DOCX file, locates its
bibliography, extracts every usable DOI, checks each DOI against Crossref, and
produces a plain-text report. When a DOI resolves, the Crossref metadata is
compared against the reference text and inconsistencies are recorded. References
with no DOI are checked against Crossref by title search to flag entries that
probably should carry a DOI.

All metadata and reports are stored in SQLite so a document can be re-checked or
a prior report retrieved when the document has been seen before.

## Decisions

These were settled during brainstorming and constrain the rest of the design:

- Platform: macOS and Windows.
- Frontend: Svelte (thin UI; no business logic).
- Backend: Rust, in a Tauri v2 application.
- Metadata comparison: fuzzy match of Crossref fields against the raw reference
  text. No structured parsing of references into discrete fields.
- DOI extraction scope: detect the bibliography section and split it into
  entries; fall back to a whole-document DOI scan if no section is found.
- References without a DOI: query Crossref by title (bibliographic search) and
  flag entries that probably should have a DOI.
- Crossref contact email: a Settings field, pre-filled with `urschrei@gmail.com`,
  used in the polite-pool User-Agent.
- Re-check behaviour: recognise a document by fingerprint, show the most recent
  stored report, and offer a Re-check button that appends a new report.
- Report output: in-app view plus on-demand export to a `.txt` file.
- Window layout: source-list sidebar (documents on the left, report on the
  right).

## Architecture

Tauri v2, single window.

- Frontend (Svelte): source-list sidebar plus report pane. Communicates with the
  Rust backend through Tauri `invoke` commands and listens for progress events.
  Holds no business logic.
- Backend (Rust): all processing, in a library crate split into focused modules,
  with a thin command layer wiring the modules to the UI.

### Library choices

Each is chosen to keep cross-platform packaging simple (pure-Rust or bundled, no
system dependencies):

- HTTP: `reqwest` (async, rustls) on the `tokio` runtime.
- SQLite: `rusqlite` with the bundled feature, plus small embedded migrations.
- DOCX: `docx-rs`, or unzip and parse `word/document.xml` directly.
- PDF: `pdf-extract` for the first version. Risk: PDF text-extraction quality
  varies and image-only or scanned PDFs have no text layer (no OCR in v1). If
  quality proves inadequate, the upgrade path is `pdfium-render`, which is more
  capable but ships a per-platform binary.
- Fuzzy matching: `strsim`, with normalisation (lowercase, strip diacritics,
  collapse whitespace and punctuation).

### Backend modules

- `ingest`: read file bytes, compute the fingerprint, determine file kind.
- `extract`: text extraction for PDF and DOCX.
- `biblio`: bibliography detection and segmentation into entries.
- `doi`: DOI extraction and normalisation.
- `crossref`: async client for DOI resolution and bibliographic title search,
  with polite-pool User-Agent, bounded concurrency, and backoff.
- `compare`: fuzzy comparison of Crossref metadata against reference text.
- `report`: build the report structure and render plain text.
- `store`: SQLite access, schema, and migrations.
- `commands`: Tauri command handlers; emit progress events.

## Processing pipeline

```
file -> fingerprint -> [DB lookup by fingerprint]
                          | seen?  -> show latest report (+ Re-check button)
                          | new / Re-check
   extract text -> detect bibliography -> segment into entries
        -> per entry: find DOI(s), classify checkable / no-DOI
        -> checkable:  Crossref resolve -> fuzzy-compare metadata vs reference
                       text -> discrepancies
        -> no-DOI:     Crossref title search -> strong match with DOI? ->
                       flag "likely missing DOI"
   -> build report -> persist (document + check + entries + discrepancies) ->
      return to UI
```

- Fingerprint: SHA-256 of the file bytes.
- Bibliography detection: locate a References / Bibliography / Works Cited /
  Literature Cited heading near the end of the document; segment entries by
  numbering markers (`[n]`, `n.`) or hanging-indent and blank-line heuristics.
- Fallback: if no section is found, scan the whole document for DOIs. In that
  case entry counts are reported as "n/a (no bibliography detected)".
- DOI extraction: regex (`10.\d{4,9}/...`), strip trailing punctuation,
  lowercase, deduplicate.
- Crossref: polite-pool User-Agent carrying the configured email; modest bounded
  concurrency via a semaphore; retry with backoff; honour HTTP 429 and
  `Retry-After`. Title search uses `query.bibliographic` with a score threshold
  to avoid false "missing DOI" flags.
- Compare: normalise, then fuzzy-match Crossref title, first-author surname,
  year, and container title against the raw reference text; each field below the
  threshold is recorded as a discrepancy.
- Progress events are emitted per entry so the UI can show "checking 12 of 41".

## Data model (SQLite)

- `documents`: id, fingerprint (unique), filename, kind (pdf/docx), first_seen,
  last_checked.
- `checks`: id, document_id, run_at, the count fields, and the rendered
  `report_text`.
- `entries`: id, check_id, ordinal, raw_text, doi (nullable), status.
- `discrepancies`: id, entry_id, field, reference_value, crossref_value, kind.
- `settings`: key/value (holds `crossref_email`, default `urschrei@gmail.com`).

The plain-text report is the canonical artefact, stored in `checks.report_text`.
The structured rows back the richer sidebar display and let counts be re-derived.

The database lives in the platform application-data directory provided by Tauri.

## Report format

Plain text:

```
DOI Check Report
Document:     thesis.pdf
Fingerprint:  sha256:a3f1...
Date / Time:  2026-05-20 18:40:12

Summary
  Bibliography entries:        52
  Checkable (with DOI):        41
  Resolved on Crossref:        37
  Not resolved:                 4
  Entries with discrepancies:   3
  No-DOI entries flagged:       2

Discrepancies
  [12] 10.xxxx/yyy  title: ref "..." vs Crossref "..."
  [27] 10.xxxx/zzz  not found on Crossref

Possibly missing DOIs
  [33] no DOI; closest Crossref match 10.1000/xyz (score 78)
```

Export-to-`.txt` writes this exact text on demand.

## UI and interaction

Single window, source-list sidebar layout:

- Sidebar: documents seen before, each with a status dot (clean / has-issues /
  failed) and a last-checked date. Selecting one shows its latest report.
- Main pane: drop zone / empty state when nothing is selected; otherwise summary
  count cards, the discrepancies list, and the possibly-missing-DOIs list, with
  Re-check and Export in the toolbar.
- Settings: a small sheet or window with the Crossref email field.
- Native menus and conventions follow the Apple Human Interface Guidelines where
  Tauri allows; the same UI runs on Windows.

## Error handling

Errors are surfaced in the report or UI, never crashes:

- Empty or near-empty extraction: "no extractable text (image-only PDF?)".
- No bibliography detected: whole-document fallback, with entry counts shown as
  n/a.
- Network failure: entries marked "check failed (network)", distinct from "not
  found".
- Rate limiting: back off and retry, honouring `Retry-After`.

## Testing

- Unit tests: DOI normalisation, heading detection, entry segmentation, fuzzy
  comparison, report rendering, DOCX extraction.
- Crossref client tested against a mock HTTP server (`wiremock`).
- Store tested against an in-memory SQLite database.
- Fixtures: sample reference-list text, a small DOCX, and a small PDF.
- Property-based tests (Hegel) for DOI normalisation round-trips and report-count
  invariants.

## Risks and notes

- PDF text-extraction quality varies; image-only PDFs are unsupported in v1 (no
  OCR). `pdfium-render` is the upgrade path if needed.
- Reference segmentation is heuristic; counts are approximate for messy
  documents.
- Reverse title-search matching needs a score threshold to avoid false
  "missing DOI" flags.
