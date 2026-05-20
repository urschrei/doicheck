# Development guide

Day-to-day reference for working on DOI Checker.

## Stack

- **Backend:** Rust, in a Tauri v2 application (`src-tauri/`). All processing
  (extraction, bibliography parsing, DOI handling, Crossref, comparison, storage)
  lives here as focused modules behind a thin command layer.
- **Frontend:** Svelte 5 (runes) on SvelteKit in SPA mode (`src/`), built by Vite.
  It holds no business logic — it calls Tauri commands and renders the results.
- **Storage:** SQLite via `rusqlite` (bundled), in the platform app-data directory.
- **Targets:** macOS and Windows.

## Prerequisites and setup

- Rust (stable) and Node.js 22+.
- `cargo-nextest` for tests: `cargo install cargo-nextest` (or `cargo test` works too).

```sh
npm install          # frontend dependencies (also needed after any package.json change)
npm run tauri dev    # run the app with hot reload
```

`npm run tauri dev` compiles the Rust backend on first run (slow once, then
incremental) and opens the window. The frontend hot-reloads; backend changes
trigger a recompile and relaunch.

## Common commands

```sh
# Backend (run from src-tauri/)
cargo nextest run                              # tests
cargo clippy --no-deps --all-targets -- -D warnings
cargo fmt                                      # format; --check to verify only

# Frontend (run from the repo root)
npm run build                                  # production build (also a compile check)
```

CI runs exactly these (see `.github/workflows/ci.yml`). `clippy` only lints our
crate (`--no-deps`); the long "Checking …" output is dependency *compilation*,
which is cached.

## Repository layout

```
src/                      Svelte frontend
  routes/+page.svelte     app shell: sidebar + report pane, top-level state
  routes/+layout.js       SvelteKit SPA config (ssr = false)
  lib/api.js              wrappers over Tauri invoke + the progress event
  lib/result.js           helpers for the serialised CheckResult (classify, tallies)
  lib/Sidebar.svelte      document list with status dots and delete
  lib/ReportPane.svelte   toolbar, filters, summary, the entry list, exports
  lib/EntryCard.svelte    one reference entry (links, mismatches, dismiss)
  lib/Settings.svelte     Crossref email + reports folder
  lib/update.js           on-launch update check
src-tauri/src/            Rust backend (see "Backend modules")
docs/                     this guide; design specs/plans under docs/superpowers/
.github/workflows/        ci.yml (test) and release.yml (build + publish)
```

## Backend modules (`src-tauri/src/`)

Each module has one responsibility:

- `model.rs` — shared types passed between stages and to the UI: `FileKind`,
  `ReferenceEntry`, `Discrepancy`, `SuggestedDoi`, `EntryOutcome`, `CheckedEntry`,
  `Counts`, `CheckResult`, `Progress`. `CheckResult::counts()` derives the summary;
  `CheckResult::apply_dismissals()` annotates dismissed discrepancies.
- `ingest.rs` — read a file, compute its `sha256:` fingerprint, classify PDF/DOCX.
- `extract/` — text extraction: `pdf.rs` (pdf-extract), `docx.rs` (zip + quick-xml),
  `mod.rs` (dispatch by `FileKind`, plus `has_usable_text`).
- `doi.rs` — DOI regex, `normalise`, `extract_all`, `first_in`, and
  `extract_with_context` (DOI + surrounding window, used by the no-heading fallback).
- `biblio.rs` — find the references heading and segment entries (author-date or
  numbered, de-wrapping continuation lines); `detect()` falls back to DOI windows
  when no heading is found.
- `text.rs` — normalisation, tokenisation, `token_coverage`, and `is_comparable`.
- `compare.rs` — `Metadata` plus `compare()`: fuzzy-matches Crossref fields against
  the reference text, producing `Discrepancy` records.
- `crossref.rs` — async client: `resolve_json`/`resolve` (with retry/backoff),
  `search`, and `metadata_from_json`. Honours 429 `Retry-After`, retries 5xx and
  transient network errors, treats 404 as definitive `NotFound`.
- `cache.rs` — `DoiCache` trait with `MemoryCache` (tests) and `StoreCache` (SQLite).
- `pipeline.rs` — orchestration. `run()` detects the bibliography then resolves each
  entry through the cache + Crossref, comparing metadata; `recheck_failures()`
  re-resolves only transient failures from a stored result; `resolve_doi_outcome()`
  is the shared cache-first resolver.
- `report.rs` — render a `CheckResult` to the canonical plain-text report.
- `export.rs` — `to_json` and `to_csv`.
- `store.rs` — SQLite schema, migrations, persistence, settings, the DOI cache, and
  dismissals.
- `commands.rs` / `lib.rs` — Tauri command handlers and app wiring (plugins, state).

## Processing pipeline (end to end)

1. A file path arrives from the UI (`check_document`).
2. `ingest` reads it, fingerprints it, and determines the kind.
3. `extract` turns bytes into text; `has_usable_text` guards against image-only PDFs.
4. `biblio::detect` finds the references section and segments it into entries; if no
   heading is found it builds entries from DOI-anchored windows
   (`bibliography_detected = false`).
5. For each entry, `pipeline::run`:
   - with a DOI: `resolve_doi_outcome` checks the cache, else fetches via Crossref
     (with backoff) and caches the JSON; on success it compares metadata against the
     reference text (only when `is_comparable`), recording discrepancies; 404 →
     `Unresolved { network_error: false }`; transient failure →
     `Unresolved { network_error: true }` (never cached).
   - without a DOI: a Crossref title search may suggest one.
   - a `Progress { done, total, cached, fetched }` event is emitted per entry.
6. The resulting `CheckResult` is persisted (`store::save_check`) as structured JSON
   plus a rendered plain-text report, and returned to the UI.

## Data model (SQLite)

- `documents` — id, fingerprint (unique), filename, kind, first_seen, last_checked.
- `checks` — id, document_id, run_at, the count columns, `report_text`, `result_json`.
- `entries`, `discrepancies` — structured per-check rows (the canonical structured
  form is `result_json`; these rows back queries/inspection).
- `settings` — key/value (`crossref_email`, `reports_dir`).
- `crossref_cache` — `doi` (primary key) -> raw Crossref JSON + `fetched_at`. Global,
  shared across documents; never cleared by deleting a document.
- `dismissals` — `(fingerprint, doi, field)` marking a field mismatch as a false
  positive.

Migrations live in `store::migrate` (idempotent `CREATE TABLE IF NOT EXISTS` plus
guarded `ALTER TABLE` for columns added later). `latest_result` is the single point
that loads a stored `CheckResult` and applies the document's dismissals, so display,
exports, and sidebar status stay consistent.

## Tauri command surface

`list_documents`, `get_email`/`set_email`, `get_reports_dir`/`set_reports_dir`,
`open_document` (returns the stored result if the file is known, else null),
`latest_check` (by fingerprint), `check_document` (run + persist + return), 
`recheck_failures`, `export_report(path, fingerprint, format)`,
`delete_document`, `dismiss_discrepancy`/`undismiss_discrepancy`. They are registered
in `lib.rs` `generate_handler!` and wrapped in `src/lib/api.js`.

`check_document` is `async` and emits `progress` events. Note: a `std::sync::Mutex`
guard for the store is never held across an `.await` (Tauri async commands require
`Send` futures, which the compiler enforces).

## Frontend notes

- `EntryOutcome` is serialised by serde's default external tagging, so in JS each
  `entry.outcome` is one of `{ Resolved: {...} } | { Unresolved: {...} } | { NoDoi: {...} }`.
  `result.js` interprets this (`classify`, `entryDoi`, `activeDiscrepancies`,
  `dismissedDiscrepancies`, `cacheTally`).
- `Discrepancy.dismissed` is set by the backend on read; the UI excludes dismissed
  mismatches from counts and the issue list.
- Components communicate with Svelte 5 callback props (not `createEventDispatcher`),
  and use `$state`/`$derived`/`$props`.

## Adding a feature

Typical path for a backend-driven feature:

1. Add or extend the relevant module (e.g. a parser tweak in `biblio.rs`, a new field
   on a `model.rs` type). Write a unit test alongside.
2. If it needs to reach the UI, add/extend a `commands.rs` command, register it in
   `lib.rs`, and add a wrapper in `src/lib/api.js`.
3. Wire the UI in the relevant component; render structured data from the
   `CheckResult` via `result.js`.
4. Run the backend checks and `npm run build`.

Keep modules focused; if a file is growing beyond one responsibility, split it.

## Testing

- Pure logic has inline `#[cfg(test)]` unit tests (model, doi, biblio, text, compare,
  report, export, store, pipeline).
- The Crossref client and the pipeline use `wiremock` to mock HTTP, so tests are
  deterministic and offline.
- The store uses an in-memory SQLite database.
- `biblio` has a fixture modelled on a real pdf-extract output (numbered heading,
  hanging-indent wrapping, an entry with no DOI) to guard segmentation regressions.

## CI and releasing

- `ci.yml` runs on push to `main` and PRs: installs Tauri's Linux deps, builds the
  frontend, then `fmt --check`, `clippy`, and `nextest`. Caches cargo and npm.
- `release.yml` runs on a `v*` tag: builds macOS (universal) and Windows via
  `tauri-action`, signs the updater artifacts, and attaches them to a **draft**
  GitHub release to review and publish.
- **Updater:** the app checks `…/releases/latest/download/latest.json` on launch and
  verifies updates with the public key in `tauri.conf.json` (`plugins.updater.pubkey`).
  The matching private key is a repository secret (`TAURI_SIGNING_PRIVATE_KEY`); it
  must never be committed.
- **To cut a release:** bump the version in `src-tauri/tauri.conf.json` (and
  `Cargo.toml`/`package.json`), commit, `git tag vX.Y.Z`, push the tag, then publish
  the draft release.

## Conventions

- UK spelling; no emoji in code, comments, or docs; factual, non-hyperbolic prose.
- New files end with a trailing newline; no whitespace on blank lines.
- The frozen design specifications and implementation plans for each iteration are
  under `docs/superpowers/` for historical reference.
