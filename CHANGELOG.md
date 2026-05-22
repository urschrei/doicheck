# Changelog

All notable changes to this project are documented here. The format is based on
[Keep a Changelog](https://keepachangelog.com/), and the project follows
[Semantic Versioning](https://semver.org/). Release-only version bumps are
omitted.

## [Unreleased]

- Update the not-found wording in the UI to "DOI not found on Crossref or
  DataCite" (the entry card label and the sidebar status tooltip), matching the
  text report. A DOI is only marked unresolved after both agencies are checked,
  so the previous Crossref-only wording was misleading. The help text now also
  describes checks against both agencies and credits DataCite as a data source.

## [0.5.0] - 2026-05-22

- Resolve and check DOIs registered with DataCite, not only Crossref. A cited DOI
  that Crossref does not index (datasets, preprints, theses, and ResearchGate
  uploads are typically registered with DataCite) is now resolved via DataCite
  rather than reported as not found, and references without a DOI fall back to a
  DataCite search when Crossref has no match. The report and UI show which agency
  each result came from, and DataCite results are cached separately under the
  same 30-day TTL.
- Cache bibliographic-search lookups for references without a DOI: a suggested
  DOI (title match of 80% or better) is reused on later runs instead of
  re-querying Crossref, and a search hit also seeds the DOI cache so the same
  work resolves from cache if it is later cited with its DOI. Cached lookups
  expire after the 30-day cache TTL.
- Count search-cache hits in the cache tally: a no-DOI suggestion served from the
  cache now shows as a cached lookup in the report and UI, so re-checking a
  document reflects what the cache saved (previously only resolved-DOI cache hits
  were counted).
- Segment author-date bibliographies whose years are unparenthesised (e.g.
  Harvard/EndNote "SURNAME, A. 2020. Title", including all-caps surnames and
  organisation authors, with the year wrapped onto its own line). Such reference
  lists previously collapsed into one or two entries. Author lists that wrap
  across lines now stay within a single entry, and repeated page-footer lines
  (e.g. a name or ID number) are dropped rather than glued onto a reference.
- Fix "Re-check entire doc" for documents selected from the sidebar: the source
  file path is now stored, so re-checking no longer requires the file to be
  freshly opened (with a prompt to locate it if it has since moved).
- Index checks(document_id) so latest-result lookups and document deletes use an
  index rather than scanning the checks table, keeping them fast as check history
  grows.

## [0.4.0] - 2026-05-22

- Fix a reference-checking false negative: the publication year was matched as a
  substring, so a wrong year passed unflagged when its digits appeared inside a
  larger number (e.g. a page or volume). It is now matched as a whole token,
  while still accepting an author-date suffix such as `2020a`.
- Expire cached Crossref records after 30 days, so a later retraction or
  correction is eventually picked up rather than masked by a stale cache entry.
- Guard the CSV export against spreadsheet formula injection: a field beginning
  with `=`, `+`, `-`, or `@` is escaped so spreadsheets treat it as text.
- Log previously-silent failures (PDFium/DOCX extraction, Crossref response
  parsing, cache reads and writes) to make problems diagnosable.
- Harden persistence: a result that fails to serialise is no longer stored as an
  empty, unreadable row, and an unreadable stored result is logged rather than
  silently treated as "no result".
- Drop the unused per-entry `entries` and `discrepancies` tables (the full result
  is stored as JSON); existing databases are migrated automatically.
- Internal: reduce allocations, make `EntryOutcome` handling exhaustive, and
  replace stringly-typed values (document status, export format, DOIs) with
  dedicated types.

## [0.3.2] - 2026-05-21

- Improve bibliography boundary detection: end the reference list before trailing
  matter (declaration, statement, acknowledgements, author biography) so it is
  not absorbed into the last reference (which previously also corrupted that
  reference's DOI); recognise "Resources"/"Sources" headings; strip repeating
  running heads and page numbers before segmentation; and trim the no-heading
  fallback so adjacent references are not conflated.
- Remove the unused VS Code workspace folder (`.vscode/`) from the repository.
- Speed up the Windows release build by using `rd /s /q` for the disk-cleanup step.

## [0.3.1] - 2026-05-21

- Strip trailing punctuation (e.g. the full stop after a DOI) from linkified
  in-text references so the link target is correct.

## [0.3.0] - 2026-05-21

- Enrich report output: identify each flagged reference by a text snippet (and a
  `reference_text` column in CSV), distinguish "DOI not found on Crossref" from a
  metadata mismatch in the status dot, and surface the transient "retry needed"
  state in the text and CSV exports.
- Move the crate to the Rust 2024 edition.

## [0.2.8] - 2026-05-21

- Ad-hoc sign the macOS build; document the quarantine workaround and Developer
  ID notarisation.

## [0.2.7] - 2026-05-21

- Maintenance release.

## [0.2.6] - 2026-05-21

- Flag references citing LLM/chatbot sources (e.g. `utm_source=chatgpt.com`).
- Detect bibliography headings with a trailing colon; stop the section at an
  appendix.
- Draw the flagged-card outline fully red.
- Release workflow: use a `RELEASE_TOKEN` PAT when present.

## [0.2.5] - 2026-05-21

- Fetch Crossref concurrently (configurable, default 5) with DOI de-duplication.
- Add a Settings menu item (Cmd+,) and assorted UI tweaks: selected-document
  highlight, clearer card labels, larger default window.

## [0.2.4] - 2026-05-21

- Fix the Windows PDFium fetch (use the `.tgz` asset).

## [0.2.3] - 2026-05-21

- Extract PDF text with PDFium (falling back to pdf-extract); bundle PDFium at
  runtime, fetched per platform in the release workflow.
- Document which files set the release version.

## [0.2.2] - 2026-05-21

- Maintenance release.

## [0.2.1] - 2026-05-21

- Rejoin URLs split across line wraps in bibliography entries.
- Fix Linux clippy by scoping the macOS-only menu import.

## [0.2.0] - 2026-05-21

- Add dark mode following the system colour scheme.
- Add an in-app Help/About panel, opened from the native About menu item.
- Release workflow: free disk space on the Windows runner before building.

## [0.1.0] - 2026-05-21

Initial release of DOI Checker, a Tauri + Svelte desktop app.

- Extract bibliographies from PDF (PDFium) and DOCX, detecting headings and
  segmenting both author-date and numbered reference lists.
- Extract and normalise DOIs, then check each against Crossref, comparing
  title, author, year, and container and flagging discrepancies.
- Suggest a DOI for references that lack one via Crossref bibliographic search.
- Cache Crossref metadata locally (SQLite); support per-document false-positive
  dismissals and re-checking of transient failures.
- Export reports as plain text, JSON, and CSV; auto-update via GitHub releases.
