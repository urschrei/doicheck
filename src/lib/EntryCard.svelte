<script>
  import { openUrl } from "@tauri-apps/plugin-opener";
  import { classify, SEVERITY, entryDoi, activeDiscrepancies, dismissedDiscrepancies, suggestion, llmSource } from "$lib/result.js";
  import { linkifyParts } from "$lib/linkify.js";

  let { entry, ondismiss, onundismiss } = $props();

  const kind = $derived(classify(entry));
  const sev = $derived(SEVERITY[kind]);
  const doi = $derived(entryDoi(entry));
  const active = $derived(activeDiscrepancies(entry));
  const dismissed = $derived(dismissedDiscrepancies(entry));
  const sugg = $derived(suggestion(entry));
  const llm = $derived(llmSource(entry));
  // Which agency resolved/suggested this entry (Crossref or DataCite).
  const resolvedSource = $derived(entry.outcome.Resolved?.source ?? "Crossref");
  const viaSearch = $derived(entry.outcome.Resolved?.via_search ?? false);
  // Only a confirmed Crossref/DataCite match has a DOI worth opening; an
  // unresolved DOI did not resolve on either agency.
  const resolved = $derived(!!entry.outcome.Resolved);

  // A linkified URL that points at a DOI, so an unresolved entry can flag it.
  const isDoiUrl = (u) => /doi\.org\//i.test(u) || /\/10\.\d{4,}/.test(u);

  // Friendly labels for the discrepancy field tag.
  const FIELD_LABELS = { title: "title", author: "author(s)", year: "year", container: "container" };
  const fieldLabel = (f) => FIELD_LABELS[f] ?? f;

  function open(url) {
    openUrl(url);
  }

  async function copy(text) {
    try {
      await navigator.clipboard.writeText(text);
    } catch (e) {
      console.error("copy failed", e);
    }
  }
</script>

<div class="card" class:flagged={llm} style={llm ? "" : `border-left-color:${sev.colour}`}>
  {#if llm}
    <div class="integrity">Possible AI source — reference URL contains "{llm}"</div>
  {/if}
  <div class="head">
    <span class="badge" style="color:{sev.colour}">&#9679;</span>
    <span class="ord">[Reference: {entry.entry.ordinal}]</span>
    <span class="label" style="color:{sev.colour}">{sev.label}</span>
  </div>

  {#if entry.entry.raw_text}
    <p class="srclabel">Text in document:</p>
    <blockquote class="ref">{#each linkifyParts(entry.entry.raw_text) as p}{#if p.url}<a class="link" class:badlink={!resolved && isDoiUrl(p.url)} href={p.url} onclick={(e) => { e.preventDefault(); open(p.url); }}>{p.url}</a>{:else}{p.t}{/if}{/each}</blockquote>
  {/if}

  {#if viaSearch}
    <p class="suggest">No DOI: matched via bibliography search on {resolvedSource}.</p>
  {/if}

  {#if active.length}
    <ul class="fields mismatchlist">
      {#each active as d (d.field)}
        <li>
          <span class="fname">{fieldLabel(d.field)}</span> should be
          <span class="now" title="{resolvedSource} record">{d.crossref_value}</span>
          <button class="linkbtn" onclick={() => ondismiss?.(doi, d.field)}>mark false positive</button>
        </li>
      {/each}
    </ul>
  {/if}

  {#if dismissed.length}
    <ul class="fields dismissedlist">
      {#each dismissed as d (d.field)}
        <li><b>{fieldLabel(d.field)}:</b> dismissed as false positive
          <button class="linkbtn" onclick={() => onundismiss?.(doi, d.field)}>undo</button></li>
      {/each}
    </ul>
  {/if}

  {#if sugg}
    <p class="suggest">Closest {sugg.source ?? "Crossref"} match:
      <a class="link" href={`https://doi.org/${sugg.doi}`} onclick={(e) => { e.preventDefault(); open(`https://doi.org/${sugg.doi}`); }}>{sugg.doi}</a>
      ({sugg.title_match}%)
      <button onclick={() => copy(sugg.doi)}>copy</button></p>
  {/if}

  {#if doi}
    <div class="actions">
      <button onclick={() => copy(doi)}>copy DOI</button>
      <button onclick={() => open(`https://doi.org/${doi}`)} disabled={!resolved}
        title={resolved ? "" : "DOI did not resolve on Crossref or DataCite"}>open DOI</button>
    </div>
  {/if}
</div>

<style>
  .card { background: var(--bg-elevated); border: 1px solid var(--border-soft); border-left-width: 3px; border-radius: 6px; padding: 8px 10px; margin-bottom: 8px; }
  .card.flagged { border: 2px solid var(--integrity); }
  .integrity { background: var(--integrity); color: #fff; font-weight: 600; font-size: 12px; padding: 3px 8px; border-radius: 4px; margin-bottom: 6px; }
  .head { display: flex; align-items: center; gap: 6px; }
  .ord { font-weight: 600; }
  .label { font-size: 12px; }
  .srclabel { text-transform: uppercase; font-size: 10px; letter-spacing: 0.03em; color: var(--text-muted); margin: 6px 0 2px; }
  .ref { color: var(--text); margin: 0 0 6px; padding: 5px 9px; background: var(--bg-sidebar); border-left: 3px solid var(--border); border-radius: 0 4px 4px 0; }
  .fields { margin: 4px 0; padding-left: 18px; }
  .fields li { margin: 2px 0; }
  .mismatchlist { list-style: none; padding-left: 0; }
  .mismatchlist li { margin: 3px 0; }
  .fname { font-weight: 700; text-decoration: underline; text-underline-offset: 2px; text-transform: capitalize; }
  .now { color: var(--sev-ok); font-weight: 700; }
  .dismissedlist { color: var(--text-muted); }
  .linkbtn { border: 0; background: transparent; color: var(--accent); text-decoration: underline; cursor: pointer; font: inherit; font-size: 11px; padding: 0 0 0 4px; }
  .dismissedlist .linkbtn { color: var(--text-muted); }
  .suggest { font-size: 12px; color: var(--text-muted); margin: 4px 0; }
  .actions { display: flex; gap: 6px; align-items: center; margin: 8px -10px -8px; padding: 8px 10px; border-top: 1px solid var(--border-soft); background: var(--bg-sidebar); border-radius: 0 0 5px 5px; }
  button { font: inherit; font-size: 12px; padding: 2px 8px; }
  button:disabled { opacity: 0.5; cursor: not-allowed; }
  .link { color: var(--accent); text-decoration: underline; cursor: pointer; }
  .badlink { color: var(--sev-fail); }
</style>
