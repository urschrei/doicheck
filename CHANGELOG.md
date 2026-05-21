# Changelog

All notable changes to this project are documented here. The format is based on
[Keep a Changelog](https://keepachangelog.com/), and the project follows
[Semantic Versioning](https://semver.org/). Release-only version bumps are
omitted.

## [Unreleased]

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
