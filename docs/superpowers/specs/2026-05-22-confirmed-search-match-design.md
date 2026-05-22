# Confirmed bibliographic-search match design

Date: 2026-05-22

## Problem

A reference that carries no DOI is sent to a bibliographic search (Crossref,
then DataCite). When the search returns a record whose title is fully present in
the reference, the tool has effectively identified the work, yet it still
presents the entry as blue "No DOI - suggestion available", the same as a weak
0.8 partial match. There is no signal that the match is, in practice, certain,
and unlike a cited DOI the matched record is never compared on author, year, or
container, so a confident title match could still hide a wrong year.

## Current behaviour (verified)

- `outcome_for_entry` (`src-tauri/src/pipeline.rs`) handles a no-DOI entry by
  searching Crossref then DataCite. `suggestion_from_hit` seeds the per-source
  DOI cache with the matched record (`cache.put(source, doi, record)`) and
  returns a `SuggestedDoi` when title token coverage reaches
  `SUGGEST_THRESHOLD` (0.8). The outcome is `EntryOutcome::NoDoi { suggested,
  from_cache }`.
- A search match is scored on title only: `token_coverage(raw_text, title)`,
  stored as `SuggestedDoi::title_match` (`u8`, `round(cov * 100)`, clamped
  0-100). The author/year/container `compare()` used for cited DOIs is not run
  for search matches.
- The search cache stores the `SuggestedDoi` JSON keyed by reference text, and
  is written only when a suggestion exists (coverage >= 0.8). On a later run a
  cached suggestion is reused without a network call.
- `EntryOutcome::Resolved { doi, discrepancies, from_cache, source }` carries the
  comparison result for cited DOIs. False-positive dismissals are keyed on
  `(doi, field)` and applied to `Resolved` entries (`model.rs::apply_dismissals`).
- `Counts` (`model.rs`): `checkable` and `resolved` count cited-DOI `Resolved`
  entries; `searched`/`searched_from_cache` count no-DOI bibliographic-search
  lookups; `missing_doi_flagged` counts `NoDoi` entries that produced a
  suggestion; `with_discrepancies` counts `Resolved` entries with active
  discrepancies.
- Text report (`src-tauri/src/report.rs`): a `NoDoi` suggestion is listed under
  "Possibly missing DOIs" as `no DOI; closest {source} match {doi} (title match
  N%)`. Clean resolved entries produce no line, only summary counts.
- CSV (`src-tauri/src/export.rs`): the `doi` column is the reference's cited DOI
  (`ce.entry.doi`, empty for no-DOI entries); a `NoDoi` suggestion's DOI goes in
  the `suggested_doi` column with status `no_doi`.
- Document status (`src-tauri/src/store.rs::list_documents`) is derived from
  counts: `Incomplete` if `network_failed > 0`, else `Failed` if not-found > 0,
  else `HasIssues` if `with_discrepancies > 0`, else `Clean`.
- Frontend: `result.js::classify` maps `Resolved` to `mismatch`/`clean` by
  active discrepancies, and `NoDoi` with a suggestion to `no_doi_suggested`
  (blue). `EntryCard.svelte` reads `entry.outcome.Resolved.source` for the
  agency label. `ReportPane.svelte` buckets entries by `classify`.

## Goals

1. When a no-DOI reference matches a search record at full title coverage (every
   title token present in the reference), run the full author/year/container
   comparison and present the entry by its comparison result: green clean when
   everything matches, amber mismatch when it does not.
2. Carry the provenance so the UI and report can annotate the entry "No DOI:
   matched via bibliography search on {source}".
3. Keep the fact that the reference itself has no DOI: such entries are reported
   alongside the other no-DOI references, and are not counted as cited-DOI
   "Checkable (with DOI)" entries.

## Non-goals

- Changing the 0.8 suggestion threshold or the partial-match (blue) behaviour
  for coverage in [0.8, 1.0).
- Adding a JSON export change beyond the new `via_search` field that
  serialisation picks up automatically.
- Frontend automated tests (no JS test framework exists); the frontend is
  verified by `npm run build`.

## Design

Reuse `EntryOutcome::Resolved` for a confirmed search match, distinguished by a
new flag. This reuses the existing comparison, discrepancy, and dismissal logic;
the "no DOI" framing is carried by the flag and a separate count.

### Data model (`model.rs`)

- Add `via_search: bool` to `EntryOutcome::Resolved`, with `#[serde(default)]`
  so results stored before this field still deserialise (defaulting to `false`).
- `apply_dismissals` already matches all `Resolved`; a `via_search` entry with
  discrepancies supports dismissal unchanged (it has a matched DOI to key on).
- `Counts`: add `matched_via_search: usize`. In `counts()`, a `via_search`
  `Resolved` entry:
  - does **not** increment `checkable` or `resolved` (those mean a cited DOI);
  - increments `matched_via_search`;
  - increments `searched` (and `searched_from_cache` when `from_cache`), since
    it originates from a bibliographic search;
  - increments `with_discrepancies`/`dismissed` on the same terms as any other
    `Resolved` entry when it carries active/dismissed discrepancies.

### Pipeline (`src-tauri/src/pipeline.rs`)

- The trigger is strict full title-token coverage:
  `token_coverage(raw_text, title) == 1.0` (every title token present). The
  rounded `SuggestedDoi::title_match` is **not** used for the decision, so a
  near-miss that rounds up to 100 is not treated as confirmed.
- Refactor the no-DOI branch of `outcome_for_entry` so that, once a
  `SuggestedDoi` and the search's `from_cache` flag are known (whether produced
  fresh or read from the search cache), a single step decides the outcome:
  - if coverage is full, read the matched record from the DOI cache for that
    suggestion's `source` (seeded by `suggestion_from_hit`, same 30-day TTL),
    parse it to `Metadata` per source, run `compare(raw_text, &meta)`, and emit
    `Resolved { doi, discrepancies, from_cache, source, via_search: true }`;
  - otherwise emit `NoDoi { suggested, from_cache }` as today.
- Coverage source per path: on a fresh search the exact coverage is already
  computed in `suggestion_from_hit`; on a search-cache hit only the rounded
  `title_match` is stored, so coverage is recomputed with `token_coverage`
  against the matched record's title (read from the seeded DOI cache) to apply
  the strict trigger consistently. The recomputation uses the same record the
  fresh path scored, so it yields the same value. No search-cache format change.
- If the seeded DOI-cache record is absent (e.g. expired independently), fall
  back to emitting the `NoDoi` suggestion, so the entry degrades to blue rather
  than failing.
- `resolved_outcome` gains the `via_search` parameter (or a thin wrapper sets
  it); cited-DOI call sites pass `false`.
- `tally()` (exhaustive over `EntryOutcome`) accounts for a `via_search`
  `Resolved` as a search-origin lookup (cache vs fetched), consistent with
  `counts()`.

### Text report (`src-tauri/src/report.rs`)

- Summary: add a `Matched via search: N` line driven by
  `counts().matched_via_search`.
- "Possibly missing DOIs" section: list a clean `via_search` entry as
  `no DOI; matched via {source} search: {doi}` (confirmed; no "closest" or
  percentage). Partial `NoDoi` suggestions keep their existing
  `closest {source} match ... (title match N%)` line.
- Discrepancies section: a `via_search` entry with active discrepancies appears
  like any resolved mismatch, with an added "no DOI; matched via {source}
  search" annotation so the reader knows the reference had no cited DOI.

### CSV (`src-tauri/src/export.rs`)

- A `via_search` `Resolved` entry uses status `matched_via_search`, places the
  matched DOI in the `suggested_doi` column (the `doi` column stays empty,
  reflecting that the reference cited none), and lists any active discrepancy
  fields in `unmatched_fields` (empty when clean). A consumer reads a mismatch
  as `status=matched_via_search` with `unmatched_fields` populated.
- Cited-DOI `Resolved` entries keep status `clean`/`mismatch` unchanged.

### Document status (`src-tauri/src/store.rs`)

- No change to `list_documents`. Because a clean `via_search` entry adds nothing
  to `with_discrepancies`/`unresolved`, a document whose only such entries are
  clean stays `Clean`; a `via_search` mismatch raises `with_discrepancies` and
  therefore `HasIssues`, matching cited-DOI behaviour. Covered by a test.

### Frontend (`src/lib/`)

- `result.js`: `classify` is unchanged - a `via_search` `Resolved` is `clean` or
  `mismatch` by its discrepancies, giving green or amber. No new severity kind.
- `EntryCard.svelte`: when `entry.outcome.Resolved.via_search` is set, render a
  dedicated annotation line "No DOI: matched via bibliography search on
  {source}" (source from the existing `resolvedSource` derivation). The matched
  DOI link and any discrepancy list render as for a normal resolved entry.
- `ReportPane.svelte`: no new filter; `via_search` entries fall into the
  existing clean/mismatch buckets via `classify`.

## Testing

- `pipeline.rs`: a full-coverage search match with matching metadata yields
  `Resolved { via_search: true }` with no discrepancies; a full-coverage match
  with a wrong year yields `Resolved { via_search: true }` with a year
  discrepancy; a second run reproduces the `via_search` outcome from cache
  (search-cache hit plus seeded DOI-cache record) with `from_cache: true` and no
  network call; a partial match (a missing title token, including one that would
  round up to 100%) remains a `NoDoi` suggestion.
- `model.rs`: `counts()` places a clean `via_search` entry in
  `matched_via_search` and `searched`, not in `checkable`/`resolved`, and a
  `via_search` mismatch additionally in `with_discrepancies`.
- `report.rs`: the summary shows `Matched via search`, a clean `via_search`
  entry is listed as `matched via {source} search`, and a `via_search` mismatch
  appears in Discrepancies with the no-DOI annotation.
- `export.rs`: a `via_search` row emits status `matched_via_search` with the
  matched DOI in `suggested_doi` and an empty `doi` column.
- `store.rs`: a document with only clean `via_search` entries lists as `Clean`;
  one with a `via_search` mismatch lists as `HasIssues`.
- Frontend: `npm run build` compiles.

## Migration

`via_search` is `#[serde(default)]`, so stored results from earlier versions
load with `via_search = false` and behave exactly as before. No database schema
change.
