# Bibliography boundary parsing design

Date: 2026-05-21

## Problem

Two real term-paper PDFs parse their reference lists incorrectly. Both are
boundary failures around the bibliography:

1. **Trailing matter absorbed into the last reference.** A paper whose reference
   list is followed by a `DECLARATION` section runs the last reference (Yi, 2020)
   into the entire declaration. The merged entry's DOI is also corrupted: the
   page number `14` immediately after the DOI is glued on, turning
   `10.1093/jtm/taaa159` into `10.1093/jtm/taaa15914`.
2. **Adjacent references conflated.** A paper whose reference list is headed
   `Resources` (not a recognised heading word) is not detected as a bibliography
   at all, so the pipeline falls back to DOI-context windows. The window around
   one DOI sweeps up the preceding reference, conflating two references into one
   displayed entry. A running page-header (`Anderson 9`) sits at the front of the
   swept text, so the entry reads as "Anderson (2008) ... Black (2026)".

## Current behaviour (verified)

Pipeline: `extract::pdf::extract` (PDFium, one `\n` appended per page) →
`biblio::detect(text)`.

- `biblio::detect` (`src-tauri/src/biblio.rs`): if `section_after_heading` finds
  a heading, it splits that section with `split_entries`; otherwise it falls back
  to `doi::extract_with_context`.
- `HEADING_RE` matches a heading line that is essentially the keyword alone
  (optional section number, trailing `:`/`.`), keywords:
  `references|reference list|bibliography|works cited|literature cited`. Takes the
  **last** match. `Resources`/`Sources` are not in the set, so paper 2 yields
  `detected = false`.
- `END_HEADING_RE` ends the section only at `^\s*(?:appendix|appendices)\b`.
  Paper 1 has no appendix, so the section runs to end-of-document and the
  `DECLARATION` lines (none of which look like an entry start) are appended as
  continuation lines of the last reference.
- `is_entry_start` (private): a line starts an entry if it has a numbered marker
  (`NUMBER_MARKER_RE`) or begins uppercase and has a parenthesised year
  (`YEAR_PAREN_RE`) within the first 80 chars. Running heads like `Anderson 9`
  and bare page numbers are not entry starts, so they attach to the preceding
  entry.
- `doi::extract_with_context` (`src-tauri/src/doi.rs`): for each DOI, returns the
  text from the previous DOI (capped at a 600-char window) up to this DOI,
  whitespace-collapsed. When an intervening reference has no DOI (paper 2's
  Atkinson 2008 carries only a JSTOR URL), its text falls inside the next DOI's
  window, conflating the two.

Confirmed by running the real PDFium extractor plus `biblio::detect` on both
PDFs out-of-tree.

## Goals

1. End the reference section before trailing matter (declaration / statement /
   acknowledgements / author biography), so the last reference and its DOI stay
   clean.
2. Recognise `Resources`/`Sources` as bibliography headings so such papers take
   the segmentation path rather than the fallback.
3. Remove running page-headers and bare page numbers so they do not contaminate
   entry text.
4. Harden the no-heading fallback so it does not conflate adjacent references
   even when an intervening reference has no DOI.

## Non-goals

- No structural "detect the reference block by shape" rework; detection still
  keys off heading words plus the DOI fallback.
- No handling of undated (`n.d.`) references as entry starts (pre-existing
  limitation, out of scope).
- The real PDFs are **not** added as test fixtures (they contain student names
  and ID numbers); tests use synthetic text modelled on their structure.
- No change to the extraction layer (`extract/pdf.rs`).

## Design

All changes are in `src-tauri/src/biblio.rs` and `src-tauri/src/doi.rs`; the data
flow stays `detect(text)` → heading path or fallback path.

### 1. End-of-references terminators (`biblio.rs`)

Extend section termination. The appendix matcher stays **loose** (a title may
follow on the same line, e.g. "Appendix A — Interview questions"). The new
terminators are anchored as **whole-line headings** — the line is the heading
phrase and nothing else (`^\s*PHRASE\s*[:.]?\s*$`, case-insensitive, where
`PHRASE` may be one or more words). This prevents a citation that merely starts
with the word (e.g. a real "Declaration of Helsinki (2013) ...") from being
mistaken for a section heading.

New whole-line terminators:
`declaration(s)`, `statement of …`, `acknowledg(e)ment(s)`,
`about the author(s)`, `author biograph…`, `biography`/`biographies`,
`biographical note(s)`.

`acknowledgements` is included: it only fires when it appears *after* the
references heading, where the references are necessarily above it, so stopping is
correct.

`section_after_heading` continues to return the slice from after the heading to
the first terminator (or end of document).

### 2. Heading synonyms (`biblio.rs`)

Add `resources` and `sources` to `HEADING_RE`. Risk is low: the heading must be
essentially the whole line, and the **last** match wins. No other synonyms added.

### 3. Strip running heads (`biblio.rs`)

New `strip_running_heads(text: &str) -> String`, applied once at the top of
`detect` so both the heading path and the fallback operate on cleaned text.

Detection is **repetition-based** to limit false positives:

- Consider lines matching `^\s*(prefix)\s*(\d{1,3})\s*$`, where `prefix` is short
  (empty, or up to a few words / ~30 chars). Empty prefix = a bare page-number
  line; non-empty = a running header such as `Anderson 9`.
- Group by normalised prefix; strip every line of a group whose prefix recurs
  **≥ 3 times** across the document with differing numbers.

So `Anderson 1 … 10` and recurring page numbers are removed, while a one-off
short line such as `Smart Cities Plan 2016` is kept. Numbers are limited to 1–3
digits so a stray four-digit year on its own line is not stripped.

Removing a running-head line between two references does not merge them: the
following reference still begins a line with an entry start.

### 4. Harden the no-heading fallback (`doi.rs`)

In `extract_with_context`, before collapsing whitespace, trim each DOI's window
to begin at the **last reference-start within it**. Iterate the window's lines,
track the byte offset of the last line for which `is_entry_start` is true, and
start the context there; if none is found, keep the whole window (current
behaviour).

This requires `biblio::is_entry_start` to become `pub(crate)` and be called from
`doi.rs`. With it, paper 2's Black DOI window begins at "Black, J. (2026) ..."
and excludes the Atkinson reference (and the `Anderson 9` running head).

## Files to change

- `src-tauri/src/biblio.rs` — `HEADING_RE` synonyms; anchored end terminators in
  `END_HEADING_RE`; `strip_running_heads` + call in `detect`; `is_entry_start`
  made `pub(crate)`; tests.
- `src-tauri/src/doi.rs` — window trimming in `extract_with_context`; tests.

## Testing

Synthetic text modelled on the two papers (no real PDFs):

- **End terminator** (`biblio.rs`): a `DECLARATION` section after the last
  reference is excluded; the last entry contains the reference but not the
  declaration prose, and its DOI is `…/taaa159`, not `…/taaa15914`. A reference
  whose title starts with "Declaration of …" is **not** truncated.
- **Heading synonym** (`biblio.rs`): a `Resources` heading is detected; two
  adjacent references (one DOI-less JSTOR entry, then a DOI entry) split into
  separate entries.
- **Running heads** (`biblio.rs`): a recurring `Anderson N` header and recurring
  bare page numbers are stripped; a one-off `Word(s) + number` line is kept; an
  entry adjacent to a stripped header no longer carries the header text.
- **Fallback** (`doi.rs`): no heading, two references where the first has no DOI
  and the second does → the second's context excludes the first.
- **Regressions**: existing `biblio.rs`/`doi.rs` tests stay green (appendix stop,
  issue-number-in-parens, URL rejoin, heading variants).

Both real PDFs re-checked out-of-tree with a throwaway harness to confirm the
intended segmentation.

## Verification commands

- `cargo nextest r` (Rust tests)
- `cargo fmt` / `cargo clippy`
