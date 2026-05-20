// Helpers for interpreting a serialised CheckResult. EntryOutcome is an
// externally tagged enum, so each entry.outcome is an object with one of the
// keys "Resolved" | "Unresolved" | "NoDoi".

export function classify(entry) {
  const o = entry.outcome;
  if (o.Resolved) return o.Resolved.discrepancies.some((d) => !d.dismissed) ? "mismatch" : "clean";
  if (o.Unresolved) return o.Unresolved.network_error ? "network" : "unresolved";
  if (o.NoDoi) return o.NoDoi.suggested ? "no_doi_suggested" : "no_doi";
  return "clean";
}

export const SEVERITY = {
  unresolved: { label: "DOI not found on Crossref", colour: "#b00020", order: 0 },
  network: { label: "Check failed (network)", colour: "#b00020", order: 1 },
  mismatch: { label: "Metadata mismatch", colour: "#9a6700", order: 2 },
  no_doi_suggested: { label: "No DOI — suggestion available", colour: "#0a52c2", order: 3 },
  no_doi: { label: "No DOI found", colour: "#0a52c2", order: 4 },
  clean: { label: "Matched", colour: "#1a7f37", order: 5 },
};

// The DOI of an entry regardless of outcome (for links/copy).
export function entryDoi(entry) {
  const o = entry.outcome;
  if (o.Resolved) return o.Resolved.doi;
  if (o.Unresolved) return o.Unresolved.doi;
  return entry.entry.doi || "";
}

export function discrepancies(entry) {
  return entry.outcome.Resolved ? entry.outcome.Resolved.discrepancies : [];
}

export function activeDiscrepancies(entry) {
  return entry.outcome.Resolved
    ? entry.outcome.Resolved.discrepancies.filter((d) => !d.dismissed)
    : [];
}

export function dismissedDiscrepancies(entry) {
  return entry.outcome.Resolved
    ? entry.outcome.Resolved.discrepancies.filter((d) => d.dismissed)
    : [];
}

export function suggestion(entry) {
  return entry.outcome.NoDoi ? entry.outcome.NoDoi.suggested : null;
}

// How many resolved entries came from the local cache vs a fresh Crossref fetch.
export function cacheTally(result) {
  let cached = 0;
  let fetched = 0;
  for (const e of result?.entries ?? []) {
    if (e.outcome.Resolved) {
      if (e.outcome.Resolved.from_cache) cached += 1;
      else fetched += 1;
    }
  }
  return { cached, fetched };
}
