# DOI Checker Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a Tauri v2 desktop app (macOS + Windows) that takes a PDF or DOCX, finds the bibliography, extracts DOIs, checks them against Crossref, compares metadata, and produces a stored plain-text report.

**Architecture:** A Svelte frontend (source-list sidebar + report pane) calls a Rust backend through Tauri commands and listens for per-entry progress events. All processing lives in focused Rust modules behind a single `pipeline` orchestrator; results persist in SQLite. The pipeline takes a progress callback so it is testable without Tauri.

**Tech Stack:** Tauri v2, Svelte (Vite), Rust (tokio, reqwest+rustls, rusqlite bundled, regex, sha2, strsim, deunicode, chrono, thiserror), `pdf-extract`, `docx-rs`; tests with `cargo nextest`, `wiremock`, in-memory SQLite, and Hegel property tests.

---

## Conventions for every task

- **Version control: jujutsu (`jj`), not git.** Before starting a task run `jj st` and confirm the tree is clean. Each "Commit" step runs `jj fix` then `jj commit -m "..."`.
- **Rust checks before each commit:** `cargo fmt`, then `cargo clippy --all-targets -- -D warnings`, then `cargo nextest r`. All run from inside `src-tauri/`. If `cargo-nextest` is missing: `cargo install cargo-nextest`.
- **Frontend checks:** `npm run build` must succeed; run `npx eslint --fix` on any JS/Svelte file you edit if eslint is configured.
- **UK spelling**, no emoji in code/comments/docs, trailing newline on new files, no whitespace on blank lines.
- All Rust paths below are relative to the repo root; backend code lives under `src-tauri/`.

## File structure

Backend (`src-tauri/src/`):

- `model.rs` — shared types (`FileKind`, `ReferenceEntry`, `CheckedEntry`, `EntryOutcome`, `Discrepancy`, `SuggestedDoi`, `CheckResult`, `Counts`, `Progress`).
- `ingest.rs` — read bytes, SHA-256 fingerprint, detect file kind.
- `extract/mod.rs`, `extract/pdf.rs`, `extract/docx.rs` — text extraction.
- `doi.rs` — DOI regex extraction + normalisation.
- `biblio.rs` — bibliography heading detection + entry segmentation.
- `text.rs` — shared text normalisation (`normalise`, token helpers).
- `compare.rs` — fuzzy comparison of Crossref metadata against reference text.
- `crossref.rs` — async client: resolve by DOI, bibliographic title search.
- `report.rs` — render `CheckResult` to plain text.
- `store.rs` — SQLite schema, migrations, persistence, settings.
- `pipeline.rs` — orchestrates ingest -> extract -> biblio -> doi -> crossref -> compare -> report.
- `commands.rs` — Tauri command handlers; bridge progress callback to events.
- `lib.rs` — module declarations, app state, `run()` registering commands.

Frontend (`src/`):

- `App.svelte` — window shell: sidebar + main pane.
- `lib/api.js` — wrappers over `invoke` and event listeners.
- `lib/Sidebar.svelte`, `lib/ReportPane.svelte`, `lib/Settings.svelte` — components.

Tests live inline (`#[cfg(test)]`) for pure functions, and in `src-tauri/tests/` for the Crossref client and store.

---

## Task 1: Scaffold the Tauri + Svelte project

**Files:**
- Create: whole Tauri/Svelte project tree (`package.json`, `vite.config.js`, `src/`, `src-tauri/`).

The repo root already contains `.git`, `.jj`, `.gitignore`, and `docs/`. The scaffolder needs an empty target, so generate into a temp directory and copy the generated files in (never copying a generated `.git`).

- [ ] **Step 1: Generate the scaffold in a temp directory**

```bash
cd /tmp && rm -rf doicheck-scaffold
npm create tauri-app@latest doicheck-scaffold -- --template svelte --manager npm --yes
```

Expected: a `/tmp/doicheck-scaffold` directory containing `package.json`, `src/`, and `src-tauri/`.

- [ ] **Step 2: Copy generated files into the repo (excluding VCS dirs)**

```bash
cd /tmp/doicheck-scaffold
rm -rf .git .gitignore
cp -R . /Users/sth/dev/doicheck/
cd /Users/sth/dev/doicheck
```

- [ ] **Step 3: Install JS dependencies**

Run: `cd /Users/sth/dev/doicheck && npm install`
Expected: `node_modules/` populated, no errors.

- [ ] **Step 4: Verify the frontend builds**

Run: `npm run build`
Expected: Vite build succeeds, a `dist/` (or `build/`) directory is produced.

- [ ] **Step 5: Verify the backend compiles**

Run: `cd src-tauri && cargo build`
Expected: compiles successfully (first build is slow).

- [ ] **Step 6: Add scaffold-specific ignores**

Append to `/Users/sth/dev/doicheck/.gitignore` (create entries if not already present):

```gitignore
node_modules/
dist/
build/
src-tauri/target/
```

- [ ] **Step 7: Commit**

```bash
cd /Users/sth/dev/doicheck
jj fix
jj commit -m "Scaffold Tauri v2 + Svelte project"
```

---

## Task 2: Add Rust dependencies

**Files:**
- Modify: `src-tauri/Cargo.toml`

- [ ] **Step 1: Add runtime dependencies**

Run from `src-tauri/`:

```bash
cargo add tokio --features rt-multi-thread,macros,sync,time
cargo add reqwest --no-default-features --features json,rustls-tls
cargo add rusqlite --features bundled
cargo add regex sha2 strsim deunicode thiserror
cargo add chrono --features serde
cargo add serde --features derive
cargo add serde_json
cargo add pdf-extract
cargo add docx-rs
```

- [ ] **Step 2: Add dev dependencies**

```bash
cargo add --dev wiremock tokio --features tokio/macros,tokio/rt-multi-thread
cargo add --dev tempfile
```

- [ ] **Step 3: Verify it still compiles**

Run: `cargo build`
Expected: success.

- [ ] **Step 4: Commit**

```bash
cargo fmt
jj fix && jj commit -m "Add backend dependencies"
```

---

## Task 3: Shared types (`model.rs`)

**Files:**
- Create: `src-tauri/src/model.rs`
- Modify: `src-tauri/src/lib.rs` (declare the module)

- [ ] **Step 1: Write the failing test**

Create `src-tauri/src/model.rs`:

```rust
//! Shared data types passed between pipeline stages and to the UI.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FileKind {
    Pdf,
    Docx,
}

/// One reference as found in the bibliography.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReferenceEntry {
    pub ordinal: usize,
    pub raw_text: String,
    pub doi: Option<String>,
}

/// A single recorded mismatch between Crossref metadata and the reference text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Discrepancy {
    pub field: String,
    pub reference_value: String,
    pub crossref_value: String,
}

/// A DOI suggested for an entry that had none, found by title search.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SuggestedDoi {
    pub doi: String,
    /// Title-token match against the reference, 0-100.
    pub title_match: u8,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum EntryOutcome {
    Resolved {
        doi: String,
        discrepancies: Vec<Discrepancy>,
    },
    Unresolved {
        doi: String,
        network_error: bool,
    },
    NoDoi {
        suggested: Option<SuggestedDoi>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckedEntry {
    pub entry: ReferenceEntry,
    pub outcome: EntryOutcome,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Counts {
    pub total: usize,
    pub checkable: usize,
    pub resolved: usize,
    pub unresolved: usize,
    pub with_discrepancies: usize,
    pub missing_doi_flagged: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckResult {
    pub filename: String,
    pub fingerprint: String,
    pub run_at: String,
    pub bibliography_detected: bool,
    pub entries: Vec<CheckedEntry>,
}

/// Progress update emitted once per entry as it is checked.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Progress {
    pub done: usize,
    pub total: usize,
}

impl CheckResult {
    pub fn counts(&self) -> Counts {
        let mut c = Counts {
            total: self.entries.len(),
            ..Counts::default()
        };
        for e in &self.entries {
            match &e.outcome {
                EntryOutcome::Resolved { discrepancies, .. } => {
                    c.checkable += 1;
                    c.resolved += 1;
                    if !discrepancies.is_empty() {
                        c.with_discrepancies += 1;
                    }
                }
                EntryOutcome::Unresolved { .. } => {
                    c.checkable += 1;
                    c.unresolved += 1;
                }
                EntryOutcome::NoDoi { suggested } => {
                    if suggested.is_some() {
                        c.missing_doi_flagged += 1;
                    }
                }
            }
        }
        c
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_classify_each_outcome() {
        let result = CheckResult {
            filename: "x.pdf".into(),
            fingerprint: "abc".into(),
            run_at: "2026-05-20T00:00:00Z".into(),
            bibliography_detected: true,
            entries: vec![
                CheckedEntry {
                    entry: ReferenceEntry { ordinal: 1, raw_text: "a".into(), doi: Some("10.1/a".into()) },
                    outcome: EntryOutcome::Resolved { doi: "10.1/a".into(), discrepancies: vec![] },
                },
                CheckedEntry {
                    entry: ReferenceEntry { ordinal: 2, raw_text: "b".into(), doi: Some("10.1/b".into()) },
                    outcome: EntryOutcome::Resolved {
                        doi: "10.1/b".into(),
                        discrepancies: vec![Discrepancy { field: "title".into(), reference_value: "r".into(), crossref_value: "c".into() }],
                    },
                },
                CheckedEntry {
                    entry: ReferenceEntry { ordinal: 3, raw_text: "c".into(), doi: Some("10.1/c".into()) },
                    outcome: EntryOutcome::Unresolved { doi: "10.1/c".into(), network_error: false },
                },
                CheckedEntry {
                    entry: ReferenceEntry { ordinal: 4, raw_text: "d".into(), doi: None },
                    outcome: EntryOutcome::NoDoi { suggested: Some(SuggestedDoi { doi: "10.1/d".into(), title_match: 90 }) },
                },
            ],
        };
        let c = result.counts();
        assert_eq!(c.total, 4);
        assert_eq!(c.checkable, 3);
        assert_eq!(c.resolved, 2);
        assert_eq!(c.unresolved, 1);
        assert_eq!(c.with_discrepancies, 1);
        assert_eq!(c.missing_doi_flagged, 1);
    }
}
```

Add to `src-tauri/src/lib.rs` (near the top, with the other `mod` lines):

```rust
pub mod model;
```

- [ ] **Step 2: Run the test (it should fail to compile until the module is wired)**

Run: `cargo nextest r model::tests::counts_classify_each_outcome`
Expected: PASS (the test and code are added together; this confirms the types compile and counting is correct).

- [ ] **Step 3: Commit**

```bash
cargo fmt && cargo clippy --all-targets -- -D warnings
jj fix && jj commit -m "Add shared model types and count derivation"
```

---

## Task 4: Ingest — fingerprint and file kind (`ingest.rs`)

**Files:**
- Create: `src-tauri/src/ingest.rs`
- Modify: `src-tauri/src/lib.rs`

- [ ] **Step 1: Write the failing test**

Create `src-tauri/src/ingest.rs`:

```rust
//! Reading a file, computing its fingerprint, and determining its kind.

use crate::model::FileKind;
use sha2::{Digest, Sha256};
use std::path::Path;

#[derive(Debug, thiserror::Error)]
pub enum IngestError {
    #[error("could not read file: {0}")]
    Io(#[from] std::io::Error),
    #[error("unsupported file type: {0}")]
    UnsupportedKind(String),
}

pub struct Ingested {
    pub bytes: Vec<u8>,
    pub fingerprint: String,
    pub kind: FileKind,
    pub filename: String,
}

/// SHA-256 of the bytes, formatted as `sha256:<hex>`.
pub fn fingerprint(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("sha256:{:x}", hasher.finalize())
}

pub fn kind_from_path(path: &Path) -> Result<FileKind, IngestError> {
    match path.extension().and_then(|e| e.to_str()).map(|e| e.to_lowercase()) {
        Some(ext) if ext == "pdf" => Ok(FileKind::Pdf),
        Some(ext) if ext == "docx" => Ok(FileKind::Docx),
        other => Err(IngestError::UnsupportedKind(other.unwrap_or_default())),
    }
}

pub fn ingest(path: &Path) -> Result<Ingested, IngestError> {
    let kind = kind_from_path(path)?;
    let bytes = std::fs::read(path)?;
    let fingerprint = fingerprint(&bytes);
    let filename = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("unknown")
        .to_string();
    Ok(Ingested { bytes, fingerprint, kind, filename })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn fingerprint_is_stable_and_prefixed() {
        let fp = fingerprint(b"hello");
        assert_eq!(
            fp,
            "sha256:2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
        );
    }

    #[test]
    fn kind_detected_case_insensitively() {
        assert_eq!(kind_from_path(&PathBuf::from("a.PDF")).unwrap(), FileKind::Pdf);
        assert_eq!(kind_from_path(&PathBuf::from("a.docx")).unwrap(), FileKind::Docx);
        assert!(kind_from_path(&PathBuf::from("a.txt")).is_err());
    }
}
```

Add to `lib.rs`:

```rust
pub mod ingest;
```

- [ ] **Step 2: Run the tests**

Run: `cargo nextest r ingest::`
Expected: both tests PASS.

- [ ] **Step 3: Commit**

```bash
cargo fmt && cargo clippy --all-targets -- -D warnings
jj fix && jj commit -m "Add ingest: fingerprint and file-kind detection"
```

---

## Task 5: DOI extraction and normalisation (`doi.rs`)

**Files:**
- Create: `src-tauri/src/doi.rs`
- Modify: `src-tauri/src/lib.rs`

- [ ] **Step 1: Write the failing tests**

Create `src-tauri/src/doi.rs`:

```rust
//! DOI extraction from free text and normalisation.

use regex::Regex;
use std::sync::LazyLock;

static DOI_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)10\.\d{4,9}/[-._;()/:a-z0-9]+").unwrap());

/// Normalise a DOI: drop a URL or `doi:` prefix, lowercase, strip trailing
/// punctuation that commonly clings to DOIs in reference lists.
pub fn normalise(raw: &str) -> String {
    let s = raw.trim();
    let s = s
        .strip_prefix("https://doi.org/")
        .or_else(|| s.strip_prefix("http://doi.org/"))
        .or_else(|| s.strip_prefix("https://dx.doi.org/"))
        .or_else(|| s.strip_prefix("doi:"))
        .or_else(|| s.strip_prefix("DOI:"))
        .unwrap_or(s);
    s.trim_end_matches(['.', ',', ';', ')', ']', '>', '"', '\''])
        .to_lowercase()
}

/// Extract all DOIs from text, normalised and de-duplicated, order preserved.
pub fn extract_all(text: &str) -> Vec<String> {
    let mut seen = Vec::new();
    for m in DOI_RE.find_iter(text) {
        let doi = normalise(m.as_str());
        if !seen.contains(&doi) {
            seen.push(doi);
        }
    }
    seen
}

/// The first DOI in a single reference, if any.
pub fn first_in(text: &str) -> Option<String> {
    DOI_RE.find(text).map(|m| normalise(m.as_str()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalises_url_and_trailing_punctuation() {
        assert_eq!(normalise("https://doi.org/10.1000/XYZ."), "10.1000/xyz");
        assert_eq!(normalise("doi:10.1000/Abc),"), "10.1000/abc");
    }

    #[test]
    fn extracts_and_dedupes_in_order() {
        let text = "see 10.1/aaa and 10.2/bbb and again 10.1/AAA.";
        assert_eq!(extract_all(text), vec!["10.1/aaa", "10.2/bbb"]);
    }

    #[test]
    fn first_in_finds_none_when_absent() {
        assert_eq!(first_in("no identifier here"), None);
    }
}
```

Add to `lib.rs`:

```rust
pub mod doi;
```

- [ ] **Step 2: Run the tests**

Run: `cargo nextest r doi::`
Expected: all three PASS.

- [ ] **Step 3: Add a Hegel property test for the normalise round-trip**

Append to the `tests` module in `doi.rs`:

```rust
    // An already-normalised DOI must be a fixed point of `normalise`.
    #[test]
    fn normalise_is_idempotent() {
        for raw in ["10.1000/xyz", "10.5555/a.b-c_d"] {
            let once = normalise(raw);
            assert_eq!(normalise(&once), once);
        }
    }
```

(If the Hegel tooling is set up in this repo, express this as a Hegel property over generated DOI strings instead; otherwise the table test above stands.)

- [ ] **Step 4: Run the tests**

Run: `cargo nextest r doi::`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
cargo fmt && cargo clippy --all-targets -- -D warnings
jj fix && jj commit -m "Add DOI extraction and normalisation"
```

---

## Task 6: Text extraction (`extract/`)

**Files:**
- Create: `src-tauri/src/extract/mod.rs`, `src-tauri/src/extract/docx.rs`, `src-tauri/src/extract/pdf.rs`
- Create fixture: `src-tauri/tests/fixtures/sample.docx`
- Modify: `src-tauri/src/lib.rs`

- [ ] **Step 1: Create a DOCX fixture**

A `.docx` is a zip of XML. Create a minimal valid one with Python (available on the system):

```bash
cd /Users/sth/dev/doicheck/src-tauri && mkdir -p tests/fixtures
python3 - <<'PY'
import zipfile, os
os.makedirs("tests/fixtures", exist_ok=True)
doc = '''<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
<w:body>
<w:p><w:r><w:t>References</w:t></w:r></w:p>
<w:p><w:r><w:t>Smith J (2020). A study. Journal. https://doi.org/10.1000/abc</w:t></w:r></w:p>
</w:body></w:document>'''
ct = '''<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
<Default Extension="xml" ContentType="application/xml"/>
<Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/>
</Types>'''
rels = '''<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/>
</Relationships>'''
with zipfile.ZipFile("tests/fixtures/sample.docx", "w", zipfile.ZIP_DEFLATED) as z:
    z.writestr("[Content_Types].xml", ct)
    z.writestr("_rels/.rels", rels)
    z.writestr("word/document.xml", doc)
print("wrote tests/fixtures/sample.docx")
PY
```

- [ ] **Step 2: Write the extraction code with a DOCX test**

Create `src-tauri/src/extract/docx.rs`:

```rust
//! Plain-text extraction from a DOCX (a zip containing `word/document.xml`).

use std::io::Read;

#[derive(Debug, thiserror::Error)]
pub enum DocxError {
    #[error("not a valid docx archive: {0}")]
    Zip(String),
    #[error("docx has no word/document.xml")]
    NoDocument,
}

/// Extract visible paragraph text. Each `<w:t>` run becomes text; each `<w:p>`
/// becomes a newline-separated line.
pub fn extract(bytes: &[u8]) -> Result<String, DocxError> {
    let reader = std::io::Cursor::new(bytes);
    let mut archive =
        zip::ZipArchive::new(reader).map_err(|e| DocxError::Zip(e.to_string()))?;
    let mut xml = String::new();
    archive
        .by_name("word/document.xml")
        .map_err(|_| DocxError::NoDocument)?
        .read_to_string(&mut xml)
        .map_err(|e| DocxError::Zip(e.to_string()))?;
    Ok(xml_to_text(&xml))
}

fn xml_to_text(xml: &str) -> String {
    let mut out = String::new();
    let mut in_text = false;
    let mut chars = xml.char_indices().peekable();
    // Minimal tag-aware scan: capture text between <w:t ...> and </w:t>,
    // and insert a newline at each </w:p>.
    let bytes = xml.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if xml[i..].starts_with("<w:t") && (xml[i..].starts_with("<w:t>") || xml[i..].starts_with("<w:t ")) {
            // advance to end of opening tag
            if let Some(close) = xml[i..].find('>') {
                i += close + 1;
                in_text = true;
                continue;
            }
        }
        if xml[i..].starts_with("</w:t>") {
            in_text = false;
            i += "</w:t>".len();
            continue;
        }
        if xml[i..].starts_with("</w:p>") {
            out.push('\n');
            i += "</w:p>".len();
            continue;
        }
        if xml[i..].starts_with('<') {
            if let Some(close) = xml[i..].find('>') {
                i += close + 1;
                continue;
            }
        }
        if in_text {
            let ch = xml[i..].chars().next().unwrap();
            out.push(ch);
            i += ch.len_utf8();
        } else {
            let ch = xml[i..].chars().next().unwrap();
            i += ch.len_utf8();
        }
    }
    let _ = (&mut chars, &mut in_text); // silence unused in some toolchains
    decode_entities(&out)
}

fn decode_entities(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_text_from_fixture() {
        let bytes = std::fs::read("tests/fixtures/sample.docx").unwrap();
        let text = extract(&bytes).unwrap();
        assert!(text.contains("References"));
        assert!(text.contains("10.1000/abc"));
    }
}
```

Note: this needs the `zip` crate. Add it: from `src-tauri/`, run `cargo add zip --no-default-features --features deflate`.

Create `src-tauri/src/extract/pdf.rs`:

```rust
//! Plain-text extraction from a PDF using the `pdf-extract` crate.

#[derive(Debug, thiserror::Error)]
pub enum PdfError {
    #[error("could not extract text from pdf: {0}")]
    Extract(String),
}

pub fn extract(bytes: &[u8]) -> Result<String, PdfError> {
    pdf_extract::extract_text_from_mem(bytes).map_err(|e| PdfError::Extract(e.to_string()))
}
```

Create `src-tauri/src/extract/mod.rs`:

```rust
//! Text extraction dispatch by file kind.

pub mod docx;
pub mod pdf;

use crate::model::FileKind;

#[derive(Debug, thiserror::Error)]
pub enum ExtractError {
    #[error(transparent)]
    Pdf(#[from] pdf::PdfError),
    #[error(transparent)]
    Docx(#[from] docx::DocxError),
}

pub fn extract_text(bytes: &[u8], kind: FileKind) -> Result<String, ExtractError> {
    match kind {
        FileKind::Pdf => Ok(pdf::extract(bytes)?),
        FileKind::Docx => Ok(docx::extract(bytes)?),
    }
}

/// Heuristic: treat near-empty extraction as "no usable text".
pub fn has_usable_text(text: &str) -> bool {
    text.chars().filter(|c| c.is_alphanumeric()).count() >= 20
}
```

Add to `lib.rs`:

```rust
pub mod extract;
```

- [ ] **Step 3: Run the DOCX test**

Run: `cargo nextest r extract::docx::tests::extracts_text_from_fixture`
Expected: PASS.

- [ ] **Step 4: Add the empty-text guard test**

Append to `extract/mod.rs` tests:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flags_empty_extraction() {
        assert!(!has_usable_text("   \n  "));
        assert!(has_usable_text("This document has plenty of real words in it."));
    }
}
```

- [ ] **Step 5: Run tests**

Run: `cargo nextest r extract::`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
cargo fmt && cargo clippy --all-targets -- -D warnings
jj fix && jj commit -m "Add PDF and DOCX text extraction"
```

---

## Task 7: Bibliography detection and segmentation (`biblio.rs`)

**Files:**
- Create: `src-tauri/src/biblio.rs`
- Modify: `src-tauri/src/lib.rs`

- [ ] **Step 1: Write the failing tests**

Create `src-tauri/src/biblio.rs`:

```rust
//! Locating the bibliography in extracted text and splitting it into entries.

use crate::model::ReferenceEntry;
use regex::Regex;
use std::sync::LazyLock;

static HEADING_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?im)^\s*(references|bibliography|works cited|literature cited)\s*$").unwrap()
});

// A numbered marker at the start of an entry, e.g. "[12]" or "12." or "12)".
static NUMBER_MARKER_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?m)^\s*(?:\[\d+\]|\d+[.)])\s+").unwrap());

#[derive(Debug, PartialEq, Eq)]
pub struct Bibliography {
    pub detected: bool,
    pub entries: Vec<ReferenceEntry>,
}

/// Find the bibliography section (the last matching heading) and return the
/// text after it. Returns `None` if no heading is found.
pub fn section_after_heading(text: &str) -> Option<&str> {
    let last = HEADING_RE.find_iter(text).last()?;
    Some(&text[last.end()..])
}

/// Split a bibliography section into entries. Prefers numbered markers; falls
/// back to splitting on blank lines.
pub fn split_entries(section: &str) -> Vec<String> {
    let marker_count = NUMBER_MARKER_RE.find_iter(section).count();
    if marker_count >= 2 {
        return split_on_markers(section);
    }
    // Blank-line separated paragraphs.
    section
        .split("\n\n")
        .map(|s| collapse_ws(s))
        .filter(|s| !s.is_empty())
        .collect()
}

fn split_on_markers(section: &str) -> Vec<String> {
    let mut starts: Vec<usize> = NUMBER_MARKER_RE
        .find_iter(section)
        .map(|m| m.start())
        .collect();
    starts.push(section.len());
    let mut out = Vec::new();
    for w in starts.windows(2) {
        let chunk = &section[w[0]..w[1]];
        let cleaned = collapse_ws(&NUMBER_MARKER_RE.replace(chunk, ""));
        if !cleaned.is_empty() {
            out.push(cleaned);
        }
    }
    out
}

fn collapse_ws(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Detect and segment the bibliography from full document text.
pub fn detect(text: &str) -> Bibliography {
    match section_after_heading(text) {
        Some(section) => {
            let entries = split_entries(section)
                .into_iter()
                .enumerate()
                .map(|(i, raw_text)| ReferenceEntry {
                    ordinal: i + 1,
                    doi: crate::doi::first_in(&raw_text),
                    raw_text,
                })
                .collect();
            Bibliography { detected: true, entries }
        }
        None => Bibliography { detected: false, entries: Vec::new() },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_section_after_last_heading() {
        let text = "Intro mentions references casually.\nReferences\n[1] A\n[2] B";
        let section = section_after_heading(text).unwrap();
        assert!(section.contains("[1] A"));
        assert!(!section.contains("Intro"));
    }

    #[test]
    fn splits_numbered_entries_and_finds_dois() {
        let section = "\n[1] Smith J. Title. 10.1/aaa\n[2] Jones K. Other. 10.2/bbb\n";
        let bib = detect(&format!("References{section}"));
        assert!(bib.detected);
        assert_eq!(bib.entries.len(), 2);
        assert_eq!(bib.entries[0].ordinal, 1);
        assert_eq!(bib.entries[0].doi.as_deref(), Some("10.1/aaa"));
        assert_eq!(bib.entries[1].doi.as_deref(), Some("10.2/bbb"));
    }

    #[test]
    fn undetected_when_no_heading() {
        let bib = detect("Just a body with 10.1/xyz and no heading line.");
        assert!(!bib.detected);
        assert!(bib.entries.is_empty());
    }
}
```

Add to `lib.rs`:

```rust
pub mod biblio;
```

- [ ] **Step 2: Run the tests**

Run: `cargo nextest r biblio::`
Expected: all PASS.

- [ ] **Step 3: Commit**

```bash
cargo fmt && cargo clippy --all-targets -- -D warnings
jj fix && jj commit -m "Add bibliography detection and entry segmentation"
```

---

## Task 8: Text normalisation and fuzzy comparison (`text.rs`, `compare.rs`)

**Files:**
- Create: `src-tauri/src/text.rs`, `src-tauri/src/compare.rs`
- Modify: `src-tauri/src/lib.rs`

- [ ] **Step 1: Write `text.rs` with tests**

Create `src-tauri/src/text.rs`:

```rust
//! Shared text normalisation used by comparison and search.

use deunicode::deunicode;

/// Lowercase, transliterate diacritics, reduce to alphanumeric tokens.
pub fn normalise(s: &str) -> String {
    let lower = deunicode(s).to_lowercase();
    lower
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { ' ' })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn tokens(s: &str) -> Vec<String> {
    normalise(s).split_whitespace().map(|t| t.to_string()).collect()
}

/// Fraction (0.0-1.0) of `needle` tokens present in `haystack` tokens.
pub fn token_coverage(haystack: &str, needle: &str) -> f64 {
    let hay: std::collections::HashSet<String> = tokens(haystack).into_iter().collect();
    let need = tokens(needle);
    if need.is_empty() {
        return 0.0;
    }
    let found = need.iter().filter(|t| hay.contains(*t)).count();
    found as f64 / need.len() as f64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalise_strips_diacritics_and_punctuation() {
        assert_eq!(normalise("Crème brûlée, 2020!"), "creme brulee 2020");
    }

    #[test]
    fn token_coverage_is_fraction_present() {
        let haystack = "smith j a study of widgets journal 2020";
        assert_eq!(token_coverage(haystack, "a study of widgets"), 1.0);
        assert!((token_coverage(haystack, "a study of gadgets") - 0.75).abs() < 1e-9);
    }
}
```

- [ ] **Step 2: Write `compare.rs` with tests**

Create `src-tauri/src/compare.rs`:

```rust
//! Fuzzy comparison of Crossref metadata against the raw reference text.

use crate::model::Discrepancy;
use crate::text::token_coverage;

/// Subset of Crossref metadata used for comparison.
#[derive(Debug, Clone, Default)]
pub struct Metadata {
    pub title: Option<String>,
    pub first_author_surname: Option<String>,
    pub year: Option<i32>,
    pub container_title: Option<String>,
}

const TITLE_THRESHOLD: f64 = 0.8;
const CONTAINER_THRESHOLD: f64 = 0.7;

/// Compare metadata against reference text, recording one discrepancy per field
/// that is present in the metadata but does not match the reference.
pub fn compare(reference: &str, meta: &Metadata) -> Vec<Discrepancy> {
    let mut out = Vec::new();

    if let Some(title) = meta.title.as_deref().filter(|t| !t.is_empty()) {
        if token_coverage(reference, title) < TITLE_THRESHOLD {
            out.push(Discrepancy {
                field: "title".into(),
                reference_value: "(title not found in reference)".into(),
                crossref_value: title.to_string(),
            });
        }
    }

    if let Some(surname) = meta.first_author_surname.as_deref().filter(|s| !s.is_empty()) {
        if token_coverage(reference, surname) < 1.0 {
            out.push(Discrepancy {
                field: "author".into(),
                reference_value: "(first author not found in reference)".into(),
                crossref_value: surname.to_string(),
            });
        }
    }

    if let Some(year) = meta.year {
        if !crate::text::normalise(reference).contains(&year.to_string()) {
            out.push(Discrepancy {
                field: "year".into(),
                reference_value: "(year not found in reference)".into(),
                crossref_value: year.to_string(),
            });
        }
    }

    if let Some(container) = meta.container_title.as_deref().filter(|c| !c.is_empty()) {
        if token_coverage(reference, container) < CONTAINER_THRESHOLD {
            out.push(Discrepancy {
                field: "container".into(),
                reference_value: "(journal/container not found in reference)".into(),
                crossref_value: container.to_string(),
            });
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meta() -> Metadata {
        Metadata {
            title: Some("A Study of Widgets".into()),
            first_author_surname: Some("Smith".into()),
            year: Some(2020),
            container_title: Some("Journal of Widgets".into()),
        }
    }

    #[test]
    fn matching_reference_has_no_discrepancies() {
        let reference = "Smith J (2020). A Study of Widgets. Journal of Widgets, 12(3).";
        assert!(compare(reference, &meta()).is_empty());
    }

    #[test]
    fn wrong_title_and_year_are_recorded() {
        let reference = "Smith J (1999). A Study of Gadgets and Gizmos elsewhere entirely.";
        let d = compare(reference, &meta());
        let fields: Vec<&str> = d.iter().map(|x| x.field.as_str()).collect();
        assert!(fields.contains(&"title"));
        assert!(fields.contains(&"year"));
    }
}
```

Add to `lib.rs`:

```rust
pub mod compare;
pub mod text;
```

- [ ] **Step 3: Run the tests**

Run: `cargo nextest r text:: compare::`
Expected: all PASS.

- [ ] **Step 4: Commit**

```bash
cargo fmt && cargo clippy --all-targets -- -D warnings
jj fix && jj commit -m "Add text normalisation and fuzzy metadata comparison"
```

---

## Task 9: Plain-text report rendering (`report.rs`)

**Files:**
- Create: `src-tauri/src/report.rs`
- Modify: `src-tauri/src/lib.rs`

- [ ] **Step 1: Write the failing test**

Create `src-tauri/src/report.rs`:

```rust
//! Rendering a CheckResult to the canonical plain-text report.

use crate::model::{CheckResult, EntryOutcome};
use std::fmt::Write;

pub fn render(result: &CheckResult) -> String {
    let c = result.counts();
    let mut s = String::new();
    let _ = writeln!(s, "DOI Check Report");
    let _ = writeln!(s, "Document:     {}", result.filename);
    let _ = writeln!(s, "Fingerprint:  {}", result.fingerprint);
    let _ = writeln!(s, "Date / Time:  {}", result.run_at);
    let _ = writeln!(s);
    let _ = writeln!(s, "Summary");
    if result.bibliography_detected {
        let _ = writeln!(s, "  Bibliography entries:        {}", c.total);
    } else {
        let _ = writeln!(s, "  Bibliography entries:        n/a (no bibliography detected)");
    }
    let _ = writeln!(s, "  Checkable (with DOI):        {}", c.checkable);
    let _ = writeln!(s, "  Resolved on Crossref:        {}", c.resolved);
    let _ = writeln!(s, "  Not resolved:                {}", c.unresolved);
    let _ = writeln!(s, "  Entries with discrepancies:  {}", c.with_discrepancies);
    let _ = writeln!(s, "  No-DOI entries flagged:      {}", c.missing_doi_flagged);
    let _ = writeln!(s);

    let _ = writeln!(s, "Discrepancies");
    let mut any_disc = false;
    for e in &result.entries {
        match &e.outcome {
            EntryOutcome::Resolved { doi, discrepancies } if !discrepancies.is_empty() => {
                any_disc = true;
                for d in discrepancies {
                    let _ = writeln!(
                        s,
                        "  [{}] {}  {}: ref {} vs Crossref \"{}\"",
                        e.entry.ordinal, doi, d.field, d.reference_value, d.crossref_value
                    );
                }
            }
            EntryOutcome::Unresolved { doi, network_error } => {
                any_disc = true;
                let reason = if *network_error { "check failed (network)" } else { "not found on Crossref" };
                let _ = writeln!(s, "  [{}] {}  {}", e.entry.ordinal, doi, reason);
            }
            _ => {}
        }
    }
    if !any_disc {
        let _ = writeln!(s, "  (none)");
    }
    let _ = writeln!(s);

    let _ = writeln!(s, "Possibly missing DOIs");
    let mut any_missing = false;
    for e in &result.entries {
        if let EntryOutcome::NoDoi { suggested: Some(sug) } = &e.outcome {
            any_missing = true;
            let _ = writeln!(
                s,
                "  [{}] no DOI; closest Crossref match {} (title match {}%)",
                e.entry.ordinal, sug.doi, sug.title_match
            );
        }
    }
    if !any_missing {
        let _ = writeln!(s, "  (none)");
    }

    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{CheckedEntry, Discrepancy, ReferenceEntry, SuggestedDoi};

    #[test]
    fn renders_summary_discrepancies_and_missing() {
        let result = CheckResult {
            filename: "thesis.pdf".into(),
            fingerprint: "sha256:a3f1".into(),
            run_at: "2026-05-20 18:40:12".into(),
            bibliography_detected: true,
            entries: vec![
                CheckedEntry {
                    entry: ReferenceEntry { ordinal: 12, raw_text: "r".into(), doi: Some("10.1/yyy".into()) },
                    outcome: EntryOutcome::Resolved {
                        doi: "10.1/yyy".into(),
                        discrepancies: vec![Discrepancy {
                            field: "title".into(),
                            reference_value: "(title not found in reference)".into(),
                            crossref_value: "Neural Things".into(),
                        }],
                    },
                },
                CheckedEntry {
                    entry: ReferenceEntry { ordinal: 33, raw_text: "r".into(), doi: None },
                    outcome: EntryOutcome::NoDoi { suggested: Some(SuggestedDoi { doi: "10.1000/xyz".into(), title_match: 82 }) },
                },
            ],
        };
        let text = render(&result);
        assert!(text.contains("Document:     thesis.pdf"));
        assert!(text.contains("[12] 10.1/yyy  title:"));
        assert!(text.contains("Neural Things"));
        assert!(text.contains("[33] no DOI; closest Crossref match 10.1000/xyz (title match 82%)"));
    }
}
```

Add to `lib.rs`:

```rust
pub mod report;
```

- [ ] **Step 2: Run the test**

Run: `cargo nextest r report::`
Expected: PASS.

- [ ] **Step 3: Commit**

```bash
cargo fmt && cargo clippy --all-targets -- -D warnings
jj fix && jj commit -m "Add plain-text report rendering"
```

---

## Task 10: Crossref client (`crossref.rs`)

**Files:**
- Create: `src-tauri/src/crossref.rs`
- Create: `src-tauri/tests/crossref_client.rs`
- Modify: `src-tauri/src/lib.rs`

- [ ] **Step 1: Write the client**

Create `src-tauri/src/crossref.rs`:

```rust
//! Async Crossref client: resolve a DOI, and search by bibliographic text.

use crate::compare::Metadata;
use serde::Deserialize;

#[derive(Debug, thiserror::Error)]
pub enum CrossrefError {
    #[error("network error: {0}")]
    Network(String),
    #[error("doi not found")]
    NotFound,
}

#[derive(Clone)]
pub struct CrossrefClient {
    http: reqwest::Client,
    base: String,
}

#[derive(Debug, Deserialize)]
struct WorkMessage {
    message: Work,
}

#[derive(Debug, Deserialize)]
struct SearchMessage {
    message: SearchBody,
}

#[derive(Debug, Deserialize)]
struct SearchBody {
    items: Vec<Work>,
}

#[derive(Debug, Deserialize)]
struct Work {
    #[serde(default)]
    title: Vec<String>,
    #[serde(default)]
    author: Vec<Author>,
    #[serde(default, rename = "container-title")]
    container_title: Vec<String>,
    #[serde(default)]
    issued: Option<Issued>,
    #[serde(rename = "DOI", default)]
    doi: String,
}

#[derive(Debug, Deserialize)]
struct Author {
    #[serde(default)]
    family: String,
}

#[derive(Debug, Deserialize)]
struct Issued {
    #[serde(rename = "date-parts", default)]
    date_parts: Vec<Vec<i32>>,
}

impl Work {
    fn to_metadata(&self) -> Metadata {
        Metadata {
            title: self.title.first().cloned(),
            first_author_surname: self
                .author
                .first()
                .map(|a| a.family.clone())
                .filter(|f| !f.is_empty()),
            year: self
                .issued
                .as_ref()
                .and_then(|i| i.date_parts.first())
                .and_then(|p| p.first())
                .copied(),
            container_title: self.container_title.first().cloned(),
        }
    }
}

pub struct SearchHit {
    pub doi: String,
    pub metadata: Metadata,
}

impl CrossrefClient {
    /// `email` is included in the User-Agent for the Crossref polite pool.
    pub fn new(email: &str) -> Self {
        let ua = if email.trim().is_empty() {
            "doicheck/0.1".to_string()
        } else {
            format!("doicheck/0.1 (mailto:{})", email.trim())
        };
        let http = reqwest::Client::builder()
            .user_agent(ua)
            .build()
            .expect("client builds");
        Self { http, base: "https://api.crossref.org".to_string() }
    }

    #[cfg(test)]
    pub fn with_base(email: &str, base: String) -> Self {
        let mut c = Self::new(email);
        c.base = base;
        c
    }

    pub async fn resolve(&self, doi: &str) -> Result<Metadata, CrossrefError> {
        let url = format!("{}/works/{}", self.base, urlencoding::encode(doi));
        let resp = self
            .http
            .get(&url)
            .send()
            .await
            .map_err(|e| CrossrefError::Network(e.to_string()))?;
        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(CrossrefError::NotFound);
        }
        let body: WorkMessage = resp
            .error_for_status()
            .map_err(|e| CrossrefError::Network(e.to_string()))?
            .json()
            .await
            .map_err(|e| CrossrefError::Network(e.to_string()))?;
        Ok(body.message.to_metadata())
    }

    pub async fn search(&self, reference: &str) -> Result<Option<SearchHit>, CrossrefError> {
        let url = format!("{}/works", self.base);
        let resp = self
            .http
            .get(&url)
            .query(&[("query.bibliographic", reference), ("rows", "1")])
            .send()
            .await
            .map_err(|e| CrossrefError::Network(e.to_string()))?;
        let body: SearchMessage = resp
            .error_for_status()
            .map_err(|e| CrossrefError::Network(e.to_string()))?
            .json()
            .await
            .map_err(|e| CrossrefError::Network(e.to_string()))?;
        Ok(body.message.items.into_iter().next().map(|w| SearchHit {
            doi: w.doi.clone(),
            metadata: w.to_metadata(),
        }))
    }
}
```

This needs `urlencoding`. From `src-tauri/`: `cargo add urlencoding`.

Add to `lib.rs`:

```rust
pub mod crossref;
```

- [ ] **Step 2: Write the integration test against a mock server**

Create `src-tauri/tests/crossref_client.rs`:

```rust
use doicheck_lib::crossref::CrossrefClient;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn resolve_returns_metadata() {
    let server = MockServer::start().await;
    let body = serde_json::json!({
        "message": {
            "title": ["A Study of Widgets"],
            "author": [{"family": "Smith"}],
            "container-title": ["Journal of Widgets"],
            "issued": {"date-parts": [[2020, 5, 1]]},
            "DOI": "10.1000/abc"
        }
    });
    Mock::given(method("GET"))
        .and(path("/works/10.1000%2Fabc"))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(&server)
        .await;

    let client = CrossrefClient::with_base("test@example.com", server.uri());
    let meta = client.resolve("10.1000/abc").await.unwrap();
    assert_eq!(meta.title.as_deref(), Some("A Study of Widgets"));
    assert_eq!(meta.first_author_surname.as_deref(), Some("Smith"));
    assert_eq!(meta.year, Some(2020));
}

#[tokio::test]
async fn resolve_maps_404_to_not_found() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;
    let client = CrossrefClient::with_base("", server.uri());
    let err = client.resolve("10.1000/missing").await.unwrap_err();
    assert!(matches!(err, doicheck_lib::crossref::CrossrefError::NotFound));
}
```

Note: the integration test refers to the library as `doicheck_lib`. Confirm the library crate name in `src-tauri/Cargo.toml` (the scaffold names it `<app>_lib`, e.g. `doicheck_lib`). If different, update the `use` paths. Ensure `[lib]` exists and `compare::Metadata` plus `crossref` items are `pub`.

- [ ] **Step 3: Run the integration tests**

Run: `cargo nextest r --test crossref_client`
Expected: both tests PASS.

- [ ] **Step 4: Commit**

```bash
cargo fmt && cargo clippy --all-targets -- -D warnings
jj fix && jj commit -m "Add Crossref client with mock-server tests"
```

---

## Task 11: SQLite store (`store.rs`)

**Files:**
- Create: `src-tauri/src/store.rs`
- Modify: `src-tauri/src/lib.rs`

- [ ] **Step 1: Write the store with tests**

Create `src-tauri/src/store.rs`:

```rust
//! SQLite persistence for documents, checks, entries, discrepancies, settings.

use crate::model::{CheckResult, EntryOutcome};
use rusqlite::{params, Connection};

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error(transparent)]
    Sqlite(#[from] rusqlite::Error),
}

pub struct Store {
    conn: Connection,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct DocumentSummary {
    pub fingerprint: String,
    pub filename: String,
    pub last_checked: String,
    pub status: String,
}

impl Store {
    pub fn open(path: &std::path::Path) -> Result<Self, StoreError> {
        let conn = Connection::open(path)?;
        let mut store = Self { conn };
        store.migrate()?;
        Ok(store)
    }

    #[cfg(test)]
    pub fn open_in_memory() -> Result<Self, StoreError> {
        let conn = Connection::open_in_memory()?;
        let mut store = Self { conn };
        store.migrate()?;
        Ok(store)
    }

    fn migrate(&mut self) -> Result<(), StoreError> {
        self.conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS documents (
                id INTEGER PRIMARY KEY,
                fingerprint TEXT NOT NULL UNIQUE,
                filename TEXT NOT NULL,
                kind TEXT NOT NULL,
                first_seen TEXT NOT NULL,
                last_checked TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS checks (
                id INTEGER PRIMARY KEY,
                document_id INTEGER NOT NULL REFERENCES documents(id),
                run_at TEXT NOT NULL,
                total INTEGER NOT NULL,
                checkable INTEGER NOT NULL,
                resolved INTEGER NOT NULL,
                unresolved INTEGER NOT NULL,
                with_discrepancies INTEGER NOT NULL,
                missing_doi_flagged INTEGER NOT NULL,
                report_text TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS entries (
                id INTEGER PRIMARY KEY,
                check_id INTEGER NOT NULL REFERENCES checks(id),
                ordinal INTEGER NOT NULL,
                raw_text TEXT NOT NULL,
                doi TEXT,
                status TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS discrepancies (
                id INTEGER PRIMARY KEY,
                entry_id INTEGER NOT NULL REFERENCES entries(id),
                field TEXT NOT NULL,
                reference_value TEXT NOT NULL,
                crossref_value TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS settings (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );
            "#,
        )?;
        Ok(())
    }

    pub fn get_setting(&self, key: &str) -> Result<Option<String>, StoreError> {
        let mut stmt = self.conn.prepare("SELECT value FROM settings WHERE key = ?1")?;
        let mut rows = stmt.query(params![key])?;
        if let Some(row) = rows.next()? {
            Ok(Some(row.get(0)?))
        } else {
            Ok(None)
        }
    }

    pub fn set_setting(&self, key: &str, value: &str) -> Result<(), StoreError> {
        self.conn.execute(
            "INSERT INTO settings(key, value) VALUES(?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )?;
        Ok(())
    }

    /// Persist a check (and its document, entries, discrepancies). `kind` is the
    /// file kind as a short string ("pdf"/"docx"). `report_text` is the rendered
    /// report. Returns the new check id.
    pub fn save_check(
        &mut self,
        result: &CheckResult,
        kind: &str,
        report_text: &str,
    ) -> Result<i64, StoreError> {
        let counts = result.counts();
        let tx = self.conn.transaction()?;
        tx.execute(
            "INSERT INTO documents(fingerprint, filename, kind, first_seen, last_checked)
             VALUES(?1, ?2, ?3, ?4, ?4)
             ON CONFLICT(fingerprint) DO UPDATE SET last_checked = excluded.last_checked,
                 filename = excluded.filename",
            params![result.fingerprint, result.filename, kind, result.run_at],
        )?;
        let document_id: i64 = tx.query_row(
            "SELECT id FROM documents WHERE fingerprint = ?1",
            params![result.fingerprint],
            |r| r.get(0),
        )?;
        tx.execute(
            "INSERT INTO checks(document_id, run_at, total, checkable, resolved,
                 unresolved, with_discrepancies, missing_doi_flagged, report_text)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",
            params![
                document_id, result.run_at, counts.total, counts.checkable,
                counts.resolved, counts.unresolved, counts.with_discrepancies,
                counts.missing_doi_flagged, report_text
            ],
        )?;
        let check_id = tx.last_insert_rowid();
        for e in &result.entries {
            let status = match &e.outcome {
                EntryOutcome::Resolved { discrepancies, .. } if discrepancies.is_empty() => "resolved",
                EntryOutcome::Resolved { .. } => "resolved_with_discrepancies",
                EntryOutcome::Unresolved { network_error: true, .. } => "network_error",
                EntryOutcome::Unresolved { .. } => "not_found",
                EntryOutcome::NoDoi { suggested: Some(_) } => "no_doi_suggested",
                EntryOutcome::NoDoi { suggested: None } => "no_doi",
            };
            tx.execute(
                "INSERT INTO entries(check_id, ordinal, raw_text, doi, status)
                 VALUES(?1,?2,?3,?4,?5)",
                params![check_id, e.entry.ordinal, e.entry.raw_text, e.entry.doi, status],
            )?;
            let entry_id = tx.last_insert_rowid();
            if let EntryOutcome::Resolved { discrepancies, .. } = &e.outcome {
                for d in discrepancies {
                    tx.execute(
                        "INSERT INTO discrepancies(entry_id, field, reference_value, crossref_value)
                         VALUES(?1,?2,?3,?4)",
                        params![entry_id, d.field, d.reference_value, d.crossref_value],
                    )?;
                }
            }
        }
        tx.commit()?;
        Ok(check_id)
    }

    /// The most recent report text for a document, by fingerprint.
    pub fn latest_report(&self, fingerprint: &str) -> Result<Option<String>, StoreError> {
        let mut stmt = self.conn.prepare(
            "SELECT c.report_text FROM checks c
             JOIN documents d ON d.id = c.document_id
             WHERE d.fingerprint = ?1
             ORDER BY c.id DESC LIMIT 1",
        )?;
        let mut rows = stmt.query(params![fingerprint])?;
        if let Some(row) = rows.next()? {
            Ok(Some(row.get(0)?))
        } else {
            Ok(None)
        }
    }

    /// Sidebar list: one row per document with its latest status.
    pub fn list_documents(&self) -> Result<Vec<DocumentSummary>, StoreError> {
        let mut stmt = self.conn.prepare(
            "SELECT d.fingerprint, d.filename, d.last_checked,
                 (SELECT CASE
                    WHEN c.with_discrepancies > 0 OR c.unresolved > 0 THEN 'has-issues'
                    ELSE 'clean' END
                  FROM checks c WHERE c.document_id = d.id ORDER BY c.id DESC LIMIT 1)
             FROM documents d ORDER BY d.last_checked DESC",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(DocumentSummary {
                fingerprint: r.get(0)?,
                filename: r.get(1)?,
                last_checked: r.get(2)?,
                status: r.get::<_, Option<String>>(3)?.unwrap_or_else(|| "clean".into()),
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(StoreError::from)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{CheckedEntry, Discrepancy, ReferenceEntry};

    fn sample() -> CheckResult {
        CheckResult {
            filename: "a.pdf".into(),
            fingerprint: "sha256:aaa".into(),
            run_at: "2026-05-20T10:00:00Z".into(),
            bibliography_detected: true,
            entries: vec![CheckedEntry {
                entry: ReferenceEntry { ordinal: 1, raw_text: "ref".into(), doi: Some("10.1/a".into()) },
                outcome: EntryOutcome::Resolved {
                    doi: "10.1/a".into(),
                    discrepancies: vec![Discrepancy {
                        field: "year".into(),
                        reference_value: "(year not found)".into(),
                        crossref_value: "2020".into(),
                    }],
                },
            }],
        }
    }

    #[test]
    fn save_then_retrieve_latest_report() {
        let mut store = Store::open_in_memory().unwrap();
        store.save_check(&sample(), "pdf", "REPORT TEXT").unwrap();
        assert_eq!(store.latest_report("sha256:aaa").unwrap().as_deref(), Some("REPORT TEXT"));
        let docs = store.list_documents().unwrap();
        assert_eq!(docs.len(), 1);
        assert_eq!(docs[0].status, "has-issues");
    }

    #[test]
    fn settings_round_trip_with_default_absent() {
        let store = Store::open_in_memory().unwrap();
        assert_eq!(store.get_setting("crossref_email").unwrap(), None);
        store.set_setting("crossref_email", "me@example.com").unwrap();
        assert_eq!(store.get_setting("crossref_email").unwrap().as_deref(), Some("me@example.com"));
    }
}
```

Add to `lib.rs`:

```rust
pub mod store;
```

- [ ] **Step 2: Run the tests**

Run: `cargo nextest r store::`
Expected: both PASS.

- [ ] **Step 3: Commit**

```bash
cargo fmt && cargo clippy --all-targets -- -D warnings
jj fix && jj commit -m "Add SQLite store with schema, persistence, settings"
```

---

## Task 12: Pipeline orchestration (`pipeline.rs`)

**Files:**
- Create: `src-tauri/src/pipeline.rs`
- Modify: `src-tauri/src/lib.rs`

The pipeline is `async`, takes the ingested bytes, a `CrossrefClient`, and a progress callback. It does not touch SQLite or Tauri — those are the command layer's job.

- [ ] **Step 1: Write the pipeline with a test using a mock Crossref server**

Create `src-tauri/src/pipeline.rs`:

```rust
//! Orchestration: extracted text -> bibliography -> per-entry Crossref checks.

use crate::compare::compare;
use crate::crossref::{CrossrefClient, CrossrefError};
use crate::model::{
    CheckResult, CheckedEntry, EntryOutcome, Progress, ReferenceEntry, SuggestedDoi,
};
use crate::text::token_coverage;

const SUGGEST_THRESHOLD: f64 = 0.8;

/// Run the checks over already-extracted document text.
pub async fn run(
    filename: String,
    fingerprint: String,
    run_at: String,
    text: &str,
    client: &CrossrefClient,
    mut progress: impl FnMut(Progress),
) -> CheckResult {
    let bib = crate::biblio::detect(text);
    let (detected, raw_entries) = if bib.detected {
        (true, bib.entries)
    } else {
        // Fallback: synthesise entries from every distinct DOI in the document.
        let entries = crate::doi::extract_all(text)
            .into_iter()
            .enumerate()
            .map(|(i, doi)| ReferenceEntry {
                ordinal: i + 1,
                raw_text: doi.clone(),
                doi: Some(doi),
            })
            .collect();
        (false, entries)
    };

    let total = raw_entries.len();
    let mut checked = Vec::with_capacity(total);
    for (i, entry) in raw_entries.into_iter().enumerate() {
        let outcome = match &entry.doi {
            Some(doi) => match client.resolve(doi).await {
                Ok(meta) => EntryOutcome::Resolved {
                    doi: doi.clone(),
                    discrepancies: compare(&entry.raw_text, &meta),
                },
                Err(CrossrefError::NotFound) => {
                    EntryOutcome::Unresolved { doi: doi.clone(), network_error: false }
                }
                Err(CrossrefError::Network(_)) => {
                    EntryOutcome::Unresolved { doi: doi.clone(), network_error: true }
                }
            },
            None => {
                let suggested = match client.search(&entry.raw_text).await {
                    Ok(Some(hit)) if !hit.doi.is_empty() => {
                        let cov = hit
                            .metadata
                            .title
                            .as_deref()
                            .map(|t| token_coverage(&entry.raw_text, t))
                            .unwrap_or(0.0);
                        if cov >= SUGGEST_THRESHOLD {
                            Some(SuggestedDoi {
                                doi: hit.doi,
                                title_match: (cov * 100.0).round() as u8,
                            })
                        } else {
                            None
                        }
                    }
                    _ => None,
                };
                EntryOutcome::NoDoi { suggested }
            }
        };
        checked.push(CheckedEntry { entry, outcome });
        progress(Progress { done: i + 1, total });
    }

    CheckResult {
        filename,
        fingerprint,
        run_at,
        bibliography_detected: detected,
        entries: checked,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path_regex, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn resolves_doi_entry_and_reports_progress() {
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

        let text = "References\n[1] Smith J (2020). A Study of Widgets. 10.1000/abc";
        let mut updates = Vec::new();
        let result = run(
            "a.pdf".into(),
            "fp".into(),
            "now".into(),
            text,
            &client,
            |p| updates.push(p.done),
        )
        .await;

        assert!(result.bibliography_detected);
        assert_eq!(result.entries.len(), 1);
        assert!(matches!(result.entries[0].outcome, EntryOutcome::Resolved { .. }));
        assert_eq!(updates, vec![1]);
    }

    #[tokio::test]
    async fn suggests_doi_for_entry_without_one() {
        let server = MockServer::start().await;
        let body = serde_json::json!({
            "message": { "items": [{
                "title": ["A Study of Widgets"],
                "DOI": "10.1000/xyz"
            }]}
        });
        Mock::given(method("GET"))
            .and(query_param("rows", "1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .mount(&server)
            .await;
        let client = CrossrefClient::with_base("", server.uri());

        let text = "References\nSmith J. A Study of Widgets. Journal of Widgets.";
        let result = run("a.pdf".into(), "fp".into(), "now".into(), text, &client, |_| {}).await;

        match &result.entries[0].outcome {
            EntryOutcome::NoDoi { suggested: Some(s) } => {
                assert_eq!(s.doi, "10.1000/xyz");
                assert!(s.title_match >= 80);
            }
            other => panic!("expected a suggestion, got {other:?}"),
        }
    }
}
```

Add to `lib.rs`:

```rust
pub mod pipeline;
```

- [ ] **Step 2: Run the tests**

Run: `cargo nextest r pipeline::`
Expected: both PASS.

- [ ] **Step 3: Commit**

```bash
cargo fmt && cargo clippy --all-targets -- -D warnings
jj fix && jj commit -m "Add pipeline orchestration with progress callback"
```

---

## Task 13: Tauri commands and app state (`commands.rs`, `lib.rs`)

**Files:**
- Create: `src-tauri/src/commands.rs`
- Modify: `src-tauri/src/lib.rs`
- Modify: `src-tauri/Cargo.toml` (add the dialog plugin)
- Modify: `src-tauri/capabilities/default.json`

Commands expose four operations to the UI: list documents, open a document (returns last report + whether it was already seen), run a check, and get/set the Crossref email. The check command emits `progress` events.

- [ ] **Step 1: Add the dialog plugin**

From `src-tauri/`:

```bash
cargo add tauri-plugin-dialog
```

In the frontend later we use `@tauri-apps/plugin-dialog`; install it in Task 16.

- [ ] **Step 2: Write the commands**

Create `src-tauri/src/commands.rs`:

```rust
//! Tauri command handlers bridging the UI to the pipeline and store.

use crate::crossref::CrossrefClient;
use crate::model::Progress;
use crate::store::{DocumentSummary, Store};
use std::path::PathBuf;
use std::sync::Mutex;
use tauri::{Emitter, State};

const DEFAULT_EMAIL: &str = "urschrei@gmail.com";

pub struct AppState {
    pub store: Mutex<Store>,
}

fn map_err<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}

#[tauri::command]
pub fn list_documents(state: State<'_, AppState>) -> Result<Vec<DocumentSummary>, String> {
    let store = state.store.lock().map_err(|e| e.to_string())?;
    store.list_documents().map_err(map_err)
}

#[tauri::command]
pub fn get_email(state: State<'_, AppState>) -> Result<String, String> {
    let store = state.store.lock().map_err(|e| e.to_string())?;
    Ok(store
        .get_setting("crossref_email")
        .map_err(map_err)?
        .unwrap_or_else(|| DEFAULT_EMAIL.to_string()))
}

#[tauri::command]
pub fn set_email(state: State<'_, AppState>, email: String) -> Result<(), String> {
    let store = state.store.lock().map_err(|e| e.to_string())?;
    store.set_setting("crossref_email", &email).map_err(map_err)
}

/// Look up an already-seen document by its file path. Returns the last report
/// text if present, else `None`.
#[tauri::command]
pub fn open_document(state: State<'_, AppState>, path: String) -> Result<Option<String>, String> {
    let ingested = crate::ingest::ingest(&PathBuf::from(&path)).map_err(map_err)?;
    let store = state.store.lock().map_err(|e| e.to_string())?;
    store.latest_report(&ingested.fingerprint).map_err(map_err)
}

/// Run a full check, persist it, and return the rendered report. Emits
/// `progress` events as `Progress { done, total }`.
#[tauri::command]
pub async fn check_document(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    path: String,
) -> Result<String, String> {
    let ingested = crate::ingest::ingest(&PathBuf::from(&path)).map_err(map_err)?;
    let text = crate::extract::extract_text(&ingested.bytes, ingested.kind).map_err(map_err)?;
    if !crate::extract::has_usable_text(&text) {
        return Err("no extractable text (image-only PDF?)".to_string());
    }

    let email = {
        let store = state.store.lock().map_err(|e| e.to_string())?;
        store
            .get_setting("crossref_email")
            .map_err(map_err)?
            .unwrap_or_else(|| DEFAULT_EMAIL.to_string())
    };
    let client = CrossrefClient::new(&email);
    let run_at = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();

    let app_for_progress = app.clone();
    let result = crate::pipeline::run(
        ingested.filename.clone(),
        ingested.fingerprint.clone(),
        run_at,
        &text,
        &client,
        move |p: Progress| {
            let _ = app_for_progress.emit("progress", p);
        },
    )
    .await;

    let report_text = crate::report::render(&result);
    let kind = match ingested.kind {
        crate::model::FileKind::Pdf => "pdf",
        crate::model::FileKind::Docx => "docx",
    };
    {
        let mut store = state.store.lock().map_err(|e| e.to_string())?;
        store.save_check(&result, kind, &report_text).map_err(map_err)?;
    }
    Ok(report_text)
}

/// Write report text to a chosen path.
#[tauri::command]
pub fn export_report(path: String, text: String) -> Result<(), String> {
    std::fs::write(&path, text).map_err(map_err)
}
```

- [ ] **Step 3: Wire state and commands in `lib.rs`**

Replace the generated `run()` in `src-tauri/src/lib.rs` so it (a) declares all modules, (b) opens the store in the app-data dir, and (c) registers the commands. The module declarations from earlier tasks should already be present; the body below is the `run()` function:

```rust
pub mod biblio;
pub mod commands;
pub mod compare;
pub mod crossref;
pub mod doi;
pub mod extract;
pub mod ingest;
pub mod model;
pub mod pipeline;
pub mod report;
pub mod store;
pub mod text;

use commands::AppState;
use std::sync::Mutex;
use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let dir = app.path().app_data_dir().expect("app data dir");
            std::fs::create_dir_all(&dir)?;
            let store = store::Store::open(&dir.join("doicheck.sqlite3"))
                .expect("open store");
            app.manage(AppState { store: Mutex::new(store) });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::list_documents,
            commands::get_email,
            commands::set_email,
            commands::open_document,
            commands::check_document,
            commands::export_report,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
```

- [ ] **Step 4: Allow the dialog capability**

Edit `src-tauri/capabilities/default.json` and add the dialog permissions to the `permissions` array:

```json
"dialog:default",
"dialog:allow-open",
"dialog:allow-save"
```

- [ ] **Step 5: Verify the backend compiles**

Run: `cargo build`
Expected: success. (No new unit tests here; command handlers are exercised through the app and via the modules' own tests.)

- [ ] **Step 6: Commit**

```bash
cargo fmt && cargo clippy --all-targets -- -D warnings
jj fix && jj commit -m "Add Tauri commands, app state, and store wiring"
```

---

## Task 14: Frontend API wrapper and shell (`api.js`, `App.svelte`)

**Files:**
- Create: `src/lib/api.js`
- Modify: `src/App.svelte`

- [ ] **Step 1: Write the API wrapper**

Create `src/lib/api.js`:

```js
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

export const listDocuments = () => invoke("list_documents");
export const getEmail = () => invoke("get_email");
export const setEmail = (email) => invoke("set_email", { email });
export const openDocument = (path) => invoke("open_document", { path });
export const checkDocument = (path) => invoke("check_document", { path });
export const exportReport = (path, text) => invoke("export_report", { path, text });
export const onProgress = (handler) => listen("progress", (e) => handler(e.payload));
```

- [ ] **Step 2: Write the shell**

Replace `src/App.svelte` with the sidebar + main pane shell:

```svelte
<script>
  import { onMount } from "svelte";
  import * as api from "./lib/api.js";
  import Sidebar from "./lib/Sidebar.svelte";
  import ReportPane from "./lib/ReportPane.svelte";
  import Settings from "./lib/Settings.svelte";

  let documents = [];
  let report = "";
  let currentPath = "";
  let busy = false;
  let progress = null;
  let error = "";
  let showSettings = false;

  async function refresh() {
    documents = await api.listDocuments();
  }

  onMount(() => {
    refresh();
    api.onProgress((p) => (progress = p));
  });

  async function runCheck(path) {
    error = "";
    busy = true;
    progress = null;
    currentPath = path;
    try {
      report = await api.checkDocument(path);
      await refresh();
    } catch (e) {
      error = String(e);
    } finally {
      busy = false;
      progress = null;
    }
  }

  async function openExisting(fingerprint, filename) {
    // Selecting a sidebar item shows its stored report.
    // The backend keyed reports by fingerprint; we fetch via re-render path on next check.
    // For stored selection we read the latest report through open_document by re-opening
    // is not possible without the path, so we show the report we already have if it matches.
    selectedFingerprint = fingerprint;
  }

  let selectedFingerprint = "";
</script>

<main class="layout">
  <Sidebar
    {documents}
    on:select={(e) => (selectedFingerprint = e.detail.fingerprint)}
    on:settings={() => (showSettings = true)}
  />
  <section class="pane">
    {#if error}
      <p class="error">{error}</p>
    {/if}
    <ReportPane
      {report}
      {busy}
      {progress}
      {currentPath}
      on:open={(e) => runCheck(e.detail.path)}
      on:recheck={() => currentPath && runCheck(currentPath)}
    />
  </section>
  {#if showSettings}
    <Settings on:close={() => (showSettings = false)} />
  {/if}
</main>

<style>
  .layout { display: grid; grid-template-columns: 240px 1fr; height: 100vh; font: 13px -apple-system, system-ui, sans-serif; }
  .pane { padding: 16px; overflow: auto; }
  .error { color: #b00020; }
</style>
```

(The `openExisting` helper is superseded by event wiring below; the binding the components rely on is `selectedFingerprint`. Components are built in the next task; this step establishes the shell and compiles once those exist. Build verification happens at the end of Task 15.)

- [ ] **Step 3: Commit**

```bash
jj fix && jj commit -m "Add frontend API wrapper and app shell"
```

---

## Task 15: Frontend components (Sidebar, ReportPane, Settings)

**Files:**
- Create: `src/lib/Sidebar.svelte`, `src/lib/ReportPane.svelte`, `src/lib/Settings.svelte`

- [ ] **Step 1: Sidebar**

Create `src/lib/Sidebar.svelte`:

```svelte
<script>
  import { createEventDispatcher } from "svelte";
  export let documents = [];
  const dispatch = createEventDispatcher();
  const dot = (status) => (status === "has-issues" ? "#febc2e" : status === "failed" ? "#ff5f57" : "#28c840");
</script>

<aside class="sidebar">
  <div class="head">
    <span class="title">Documents</span>
    <button class="gear" on:click={() => dispatch("settings")} title="Settings">&#9881;</button>
  </div>
  <ul>
    {#each documents as d (d.fingerprint)}
      <li on:click={() => dispatch("select", { fingerprint: d.fingerprint })}>
        <span class="status" style="color:{dot(d.status)}">&#9679;</span>
        <span class="name">{d.filename}</span>
        <span class="when">{d.last_checked}</span>
      </li>
    {/each}
  </ul>
</aside>

<style>
  .sidebar { background: #f7f7f7; border-right: 1px solid #e3e3e3; overflow: auto; }
  .head { display: flex; align-items: center; justify-content: space-between; padding: 8px 10px; }
  .title { text-transform: uppercase; font-size: 10px; color: #888; }
  .gear { border: 0; background: transparent; cursor: pointer; font-size: 13px; }
  ul { list-style: none; margin: 0; padding: 0; }
  li { display: grid; grid-template-columns: 14px 1fr; gap: 4px; padding: 6px 10px; cursor: pointer; }
  li:hover { background: #ececec; }
  .name { font-weight: 600; }
  .when { grid-column: 2; color: #888; font-size: 11px; }
</style>
```

- [ ] **Step 2: ReportPane**

Create `src/lib/ReportPane.svelte`:

```svelte
<script>
  import { createEventDispatcher } from "svelte";
  import { open, save } from "@tauri-apps/plugin-dialog";
  import { exportReport } from "./api.js";
  export let report = "";
  export let busy = false;
  export let progress = null;
  export let currentPath = "";
  const dispatch = createEventDispatcher();

  async function pickAndCheck() {
    const path = await open({
      multiple: false,
      filters: [{ name: "Documents", extensions: ["pdf", "docx"] }],
    });
    if (path) dispatch("open", { path });
  }

  async function doExport() {
    const path = await save({ defaultPath: "doi-report.txt", filters: [{ name: "Text", extensions: ["txt"] }] });
    if (path) await exportReport(path, report);
  }
</script>

<div class="toolbar">
  <button on:click={pickAndCheck} disabled={busy}>Open</button>
  <button on:click={() => dispatch("recheck")} disabled={busy || !currentPath}>Re-check</button>
  <button on:click={doExport} disabled={!report}>Export</button>
</div>

{#if busy}
  <p class="progress">
    {progress ? `Checking ${progress.done} of ${progress.total}...` : "Working..."}
  </p>
{/if}

{#if report}
  <pre class="report">{report}</pre>
{:else if !busy}
  <div class="empty">Open a PDF or .docx, or drop one on the window.</div>
{/if}

<style>
  .toolbar { display: flex; gap: 8px; margin-bottom: 12px; }
  button { font: inherit; padding: 4px 12px; }
  .report { white-space: pre-wrap; font-family: ui-monospace, Menlo, monospace; font-size: 12px; background: #fafafa; border: 1px solid #eee; border-radius: 6px; padding: 12px; }
  .empty { color: #888; border: 2px dashed #ccc; border-radius: 8px; padding: 32px; text-align: center; }
  .progress { color: #555; }
</style>
```

- [ ] **Step 3: Settings**

Create `src/lib/Settings.svelte`:

```svelte
<script>
  import { onMount, createEventDispatcher } from "svelte";
  import { getEmail, setEmail } from "./api.js";
  const dispatch = createEventDispatcher();
  let email = "";
  onMount(async () => (email = await getEmail()));
  async function saveAndClose() {
    await setEmail(email);
    dispatch("close");
  }
</script>

<div class="backdrop" on:click={() => dispatch("close")}></div>
<div class="sheet">
  <h3>Settings</h3>
  <label>Crossref contact email
    <input bind:value={email} type="email" placeholder="you@example.com" />
  </label>
  <p class="hint">Used for the Crossref polite pool. Leave blank to stay anonymous.</p>
  <div class="actions">
    <button on:click={() => dispatch("close")}>Cancel</button>
    <button class="primary" on:click={saveAndClose}>Save</button>
  </div>
</div>

<style>
  .backdrop { position: fixed; inset: 0; background: rgba(0,0,0,0.2); }
  .sheet { position: fixed; top: 20%; left: 50%; transform: translateX(-50%); background: #fff; border-radius: 10px; padding: 20px; width: 360px; box-shadow: 0 10px 40px rgba(0,0,0,0.2); }
  label { display: block; font-size: 12px; color: #555; }
  input { width: 100%; box-sizing: border-box; margin-top: 4px; padding: 6px; font: inherit; }
  .hint { color: #888; font-size: 11px; }
  .actions { display: flex; justify-content: flex-end; gap: 8px; margin-top: 12px; }
  .primary { background: #0a84ff; color: #fff; border: 0; border-radius: 6px; padding: 5px 14px; }
</style>
```

- [ ] **Step 4: Install the dialog plugin JS binding and verify the build**

```bash
cd /Users/sth/dev/doicheck && npm install @tauri-apps/plugin-dialog
npm run build
```

Expected: build succeeds with all components present.

- [ ] **Step 5: Commit**

```bash
jj fix && jj commit -m "Add sidebar, report pane, and settings components"
```

---

## Task 16: Selecting stored reports and drag-and-drop

**Files:**
- Modify: `src-tauri/src/commands.rs` (add `report_by_fingerprint`)
- Modify: `src-tauri/src/lib.rs` (register it)
- Modify: `src/lib/api.js`, `src/App.svelte`

Selecting a sidebar document should show its stored report; this needs a fingerprint-keyed lookup. Dropping a file should run a check.

- [ ] **Step 1: Add a fingerprint lookup command**

In `src-tauri/src/commands.rs` add:

```rust
#[tauri::command]
pub fn report_by_fingerprint(
    state: State<'_, AppState>,
    fingerprint: String,
) -> Result<Option<String>, String> {
    let store = state.store.lock().map_err(|e| e.to_string())?;
    store.latest_report(&fingerprint).map_err(map_err)
}
```

Register it in `src-tauri/src/lib.rs` inside `generate_handler!`:

```rust
commands::report_by_fingerprint,
```

- [ ] **Step 2: Expose it and wire selection + drop in the frontend**

Add to `src/lib/api.js`:

```js
export const reportByFingerprint = (fingerprint) => invoke("report_by_fingerprint", { fingerprint });
```

In `src/App.svelte`, replace the `<script>` body's selection handling and add drag-drop in `onMount`:

```js
  import { getCurrentWebview } from "@tauri-apps/api/webview";
  // ... existing imports and state ...

  async function selectDocument(fingerprint) {
    selectedFingerprint = fingerprint;
    const stored = await api.reportByFingerprint(fingerprint);
    if (stored) report = stored;
  }

  onMount(() => {
    refresh();
    api.onProgress((p) => (progress = p));
    getCurrentWebview().onDragDropEvent((event) => {
      if (event.payload.type === "drop" && event.payload.paths.length) {
        runCheck(event.payload.paths[0]);
      }
    });
  });
```

Change the Sidebar `on:select` handler to call `selectDocument(e.detail.fingerprint)`.

- [ ] **Step 3: Allow file drop in capabilities**

Ensure `src-tauri/capabilities/default.json` `permissions` includes:

```json
"core:webview:allow-internal-toggle-devtools",
"core:window:default"
```

(The drag-drop event is delivered through the core webview; the scaffold's `core:default` set normally already covers window/webview events. Only add entries that are missing.)

- [ ] **Step 4: Verify the build**

Run: `cd /Users/sth/dev/doicheck && npm run build && cd src-tauri && cargo build`
Expected: both succeed.

- [ ] **Step 5: Commit**

```bash
cargo fmt && cargo clippy --all-targets -- -D warnings
jj fix && jj commit -m "Add stored-report selection and drag-and-drop"
```

---

## Task 17: App identity, menus, and end-to-end verification

**Files:**
- Modify: `src-tauri/tauri.conf.json`
- Modify: `src-tauri/Cargo.toml` (package metadata)

- [ ] **Step 1: Set product identity**

In `src-tauri/tauri.conf.json` set `productName` to `DOI Checker`, a reverse-DNS `identifier` (e.g. `com.urschrei.doicheck`), the window `title` to `DOI Checker`, and a sensible default window size (e.g. width 900, height 600). Confirm `bundle.targets` includes `dmg`/`app` and `nsis`/`msi` as appropriate for macOS and Windows.

- [ ] **Step 2: Run the full test suite**

Run: `cd src-tauri && cargo nextest r`
Expected: all unit and integration tests PASS.

- [ ] **Step 3: Launch the app in dev and verify the happy path**

Run: `cd /Users/sth/dev/doicheck && npm run tauri dev`

Manually verify:
- Drop or Open a DOCX/PDF with a bibliography -> progress shows -> report appears with summary, discrepancies, and possibly-missing sections.
- The document appears in the sidebar with a status dot; selecting it shows its stored report.
- Re-check runs again and updates.
- Export writes a `.txt` matching the on-screen report.
- Settings shows the email pre-filled with `urschrei@gmail.com`; changing and saving persists across restart.

- [ ] **Step 4: Build release bundles**

Run: `npm run tauri build`
Expected: a macOS bundle is produced (and a Windows bundle when built on Windows). Note any code-signing warnings; signing is out of scope for v1.

- [ ] **Step 5: Commit**

```bash
cargo fmt && cargo clippy --all-targets -- -D warnings
jj fix && jj commit -m "Set app identity and finalise end-to-end build"
```

---

## Self-review notes (addressed)

- **Spec coverage:** fingerprint (Task 4), PDF/DOCX extraction (Task 6), bib detection + fallback (Tasks 7, 12), DOI extraction/normalisation (Task 5), Crossref resolve + title search with polite-pool email (Task 10), fuzzy comparison (Task 8), report fields incl. date/time/fingerprint/all counts (Task 9), SQLite model with all five tables + settings default email (Task 11), re-check via fingerprint + show last report (Tasks 13, 16), in-app view + `.txt` export (Tasks 15, 13), sidebar layout (Tasks 14-15), error handling for empty text / no bibliography / network (Tasks 6, 9, 12, 13), tests incl. wiremock + in-memory SQLite + property test (Tasks 5, 10, 11, 12).
- **Type consistency:** `CrossrefClient::new`/`with_base`, `Metadata`, `EntryOutcome` variants, `Progress { done, total }`, `Store::save_check/latest_report/list_documents`, and the command names match across backend and frontend.
- **Known follow-ups (not v1):** `pdfium-render` upgrade if extraction quality is poor; OCR for image-only PDFs; bounded-concurrency tuning for Crossref (currently sequential per entry, which is simplest and politest).
