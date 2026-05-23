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
  unresolved: { label: "DOI not found on Crossref or DataCite", colour: "var(--sev-fail)", order: 0 },
  network: { label: "Check failed (network)", colour: "var(--sev-fail)", order: 1 },
  mismatch: { label: "Metadata mismatch", colour: "var(--sev-warn)", order: 2 },
  no_doi_suggested: { label: "No DOI — suggestion available", colour: "var(--sev-info)", order: 3 },
  no_doi: { label: "No DOI found", colour: "var(--sev-info)", order: 4 },
  clean: { label: "Matched", colour: "var(--sev-ok)", order: 5 },
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

// The Unresolved outcome payload (doi, network_error, registration, suggested),
// or null for any other outcome.
export function unresolved(entry) {
  return entry.outcome.Unresolved ?? null;
}

// Interpret an Unresolved entry's doi.org registration status. The Rust enum
// serialises as the string "Unknown"/"Unregistered" or { Agency: name }.
// Returns one of: { kind: "agency", agency }, { kind: "unregistered" },
// { kind: "network" } (could not be checked), or { kind: "unknown" }.
export function registrationState(entry) {
  const u = unresolved(entry);
  if (!u) return { kind: "unknown" };
  if (u.network_error) return { kind: "network" };
  const r = u.registration;
  if (r && typeof r === "object" && "Agency" in r) return { kind: "agency", agency: r.Agency };
  if (r === "Unregistered") return { kind: "unregistered" };
  return { kind: "unknown" };
}

export function llmSource(entry) {
  return entry.llm_source || null;
}

// Crossref lookups served from the local cache vs fetched fresh, across both
// DOI resolves (Resolved) and bibliographic searches for no-DOI refs (NoDoi).
export function cacheTally(result) {
  let cached = 0;
  let fetched = 0;
  for (const e of result?.entries ?? []) {
    const lookup = e.outcome.Resolved ?? e.outcome.NoDoi;
    if (lookup) {
      if (lookup.from_cache) cached += 1;
      else fetched += 1;
    }
  }
  return { cached, fetched };
}
