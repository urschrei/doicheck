<script>
  import { openUrl } from "@tauri-apps/plugin-opener";
  import { classify, SEVERITY, entryDoi, discrepancies, suggestion } from "$lib/result.js";

  let { entry } = $props();

  const kind = $derived(classify(entry));
  const sev = $derived(SEVERITY[kind]);
  const doi = $derived(entryDoi(entry));
  const discs = $derived(discrepancies(entry));
  const sugg = $derived(suggestion(entry));

  async function copy(text) {
    try {
      await navigator.clipboard.writeText(text);
    } catch (e) {
      console.error("copy failed", e);
    }
  }
</script>

<div class="card" style="border-left-color:{sev.colour}">
  <div class="head">
    <span class="badge" style="color:{sev.colour}">&#9679;</span>
    <span class="ord">[{entry.entry.ordinal}]</span>
    <span class="label" style="color:{sev.colour}">{sev.label}</span>
  </div>

  {#if entry.entry.raw_text}
    <p class="ref">{entry.entry.raw_text}</p>
  {/if}

  {#if discs.length}
    <ul class="fields">
      {#each discs as d (d.field)}
        <li><b>{d.field}:</b> Crossref says &ldquo;{d.crossref_value}&rdquo; &mdash; not found in your reference</li>
      {/each}
    </ul>
  {/if}

  {#if sugg}
    <p class="suggest">Closest Crossref match: <code>{sugg.doi}</code> ({sugg.title_match}%)
      <button onclick={() => copy(sugg.doi)}>copy</button></p>
  {/if}

  {#if doi}
    <div class="actions">
      <button onclick={() => openUrl(`https://doi.org/${doi}`)}>open DOI</button>
      <button onclick={() => copy(doi)}>copy DOI</button>
    </div>
  {/if}
</div>

<style>
  .card { border: 1px solid #eee; border-left-width: 3px; border-radius: 6px; padding: 8px 10px; margin-bottom: 8px; }
  .head { display: flex; align-items: center; gap: 6px; }
  .ord { font-weight: 600; }
  .label { font-size: 12px; }
  .ref { color: #444; margin: 4px 0; }
  .fields { margin: 4px 0; padding-left: 18px; }
  .fields li { margin: 2px 0; }
  .suggest code { font-family: ui-monospace, Menlo, monospace; }
  .actions { display: flex; gap: 6px; margin-top: 4px; }
  button { font: inherit; font-size: 12px; padding: 2px 8px; }
</style>
