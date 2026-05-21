<script>
  import { openUrl } from "@tauri-apps/plugin-opener";
  import { classify, SEVERITY, entryDoi, activeDiscrepancies, dismissedDiscrepancies, suggestion, llmSource } from "$lib/result.js";

  let { entry, ondismiss, onundismiss } = $props();

  const kind = $derived(classify(entry));
  const sev = $derived(SEVERITY[kind]);
  const doi = $derived(entryDoi(entry));
  const active = $derived(activeDiscrepancies(entry));
  const dismissed = $derived(dismissedDiscrepancies(entry));
  const sugg = $derived(suggestion(entry));
  const llm = $derived(llmSource(entry));

  const URL_RE = /(https?:\/\/[^\s)]+)/g;
  function parts(text) {
    const out = [];
    let last = 0;
    for (const m of text.matchAll(URL_RE)) {
      if (m.index > last) out.push({ t: text.slice(last, m.index) });
      out.push({ url: m[0] });
      last = m.index + m[0].length;
    }
    if (last < text.length) out.push({ t: text.slice(last) });
    return out;
  }

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
    <blockquote class="ref">{#each parts(entry.entry.raw_text) as p}{#if p.url}<a class="link" href={p.url} onclick={(e) => { e.preventDefault(); open(p.url); }}>{p.url}</a>{:else}{p.t}{/if}{/each}</blockquote>
  {/if}

  {#if active.length}
    <ul class="fields">
      {#each active as d (d.field)}
        <li><b>{d.field}:</b> Crossref says &ldquo;{d.crossref_value}&rdquo; &mdash; not found in your reference
          <button class="linkbtn" onclick={() => ondismiss?.(doi, d.field)}>mark false positive</button></li>
      {/each}
    </ul>
  {/if}

  {#if dismissed.length}
    <ul class="fields dismissedlist">
      {#each dismissed as d (d.field)}
        <li><b>{d.field}:</b> dismissed as false positive
          <button class="linkbtn" onclick={() => onundismiss?.(doi, d.field)}>undo</button></li>
      {/each}
    </ul>
  {/if}

  {#if sugg}
    <p class="suggest">Closest Crossref match:
      <a class="link" href={`https://doi.org/${sugg.doi}`} onclick={(e) => { e.preventDefault(); open(`https://doi.org/${sugg.doi}`); }}>{sugg.doi}</a>
      ({sugg.title_match}%)
      <button onclick={() => copy(sugg.doi)}>copy</button></p>
  {/if}

  {#if doi}
    <div class="actions">
      <a class="link" href={`https://doi.org/${doi}`} onclick={(e) => { e.preventDefault(); open(`https://doi.org/${doi}`); }}>{doi}</a>
      <button onclick={() => copy(doi)}>copy DOI</button>
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
  .dismissedlist { color: var(--text-muted); }
  .linkbtn { border: 0; background: transparent; color: var(--accent); text-decoration: underline; cursor: pointer; font: inherit; font-size: 11px; padding: 0 0 0 4px; }
  .dismissedlist .linkbtn { color: var(--text-muted); }
  .actions { display: flex; gap: 6px; margin-top: 4px; align-items: center; }
  button { font: inherit; font-size: 12px; padding: 2px 8px; }
  .link { color: var(--accent); text-decoration: underline; cursor: pointer; }
</style>
