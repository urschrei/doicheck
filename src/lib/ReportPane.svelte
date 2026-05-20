<script>
  import { save, open } from "@tauri-apps/plugin-dialog";
  import { exportReport, getReportsDir, setReportsDir } from "$lib/api.js";
  import { classify, SEVERITY, cacheTally } from "$lib/result.js";
  import EntryCard from "$lib/EntryCard.svelte";

  let { result = null, busy = false, progress = null, currentPath = "", onopen, onrecheck, onrecheckfailures } = $props();

  let filter = $state("all");
  let query = $state("");
  let showClean = $state(false);
  let exportOpen = $state(false);

  const classified = $derived(
    (result?.entries ?? []).map((e) => ({ entry: e, kind: classify(e) })),
  );
  const counts = $derived.by(() => {
    const c = { clean: 0, mismatch: 0, unresolved: 0, network: 0, no_doi: 0, no_doi_suggested: 0 };
    for (const x of classified) c[x.kind]++;
    return c;
  });
  const issues = $derived(
    classified
      .filter((x) => x.kind !== "clean")
      .filter((x) => filter === "all" || matchesFilter(x.kind, filter))
      .filter((x) => !query || JSON.stringify(x.entry).toLowerCase().includes(query.toLowerCase()))
      .sort((a, b) => SEVERITY[a.kind].order - SEVERITY[b.kind].order),
  );
  const cleanOnes = $derived(classified.filter((x) => x.kind === "clean"));
  const totalIssues = $derived(classified.filter((x) => x.kind !== "clean").length);
  const tally = $derived(cacheTally(result));

  function matchesFilter(kind, f) {
    if (f === "unresolved") return kind === "unresolved" || kind === "network";
    if (f === "no_doi") return kind === "no_doi" || kind === "no_doi_suggested";
    return kind === f;
  }

  async function pickAndCheck() {
    const path = await open({ multiple: false, filters: [{ name: "Documents", extensions: ["pdf", "docx"] }] });
    if (path) onopen?.(path);
  }

  function smartName(ext) {
    const stem = (result?.filename ?? "report").replace(/\.[^.]+$/, "");
    const date = new Date().toISOString().slice(0, 10);
    return `${stem}-doi-report-${date}.${ext}`;
  }

  async function doExport(format, ext) {
    if (!result) return;
    const dir = await getReportsDir();
    const path = await save({
      defaultPath: dir ? `${dir}/${smartName(ext)}` : smartName(ext),
      filters: [{ name: ext.toUpperCase(), extensions: [ext] }],
    });
    if (!path) return;
    await exportReport(path, result.fingerprint, format);
    const slash = Math.max(path.lastIndexOf("/"), path.lastIndexOf("\\"));
    if (slash > 0) await setReportsDir(path.slice(0, slash));
  }
</script>

<div class="toolbar">
  <button class="primary" onclick={pickAndCheck} disabled={busy}>Open</button>
  <button class="secondary" onclick={() => onrecheck?.()} disabled={busy || !currentPath}>Re-check entire doc</button>
  {#if counts.network > 0}
    <button class="secondary" onclick={() => onrecheckfailures?.()} disabled={busy || !result}>Re-check failures ({counts.network})</button>
  {/if}
  <span class="spacer"></span>
  <div class="exportwrap">
    <button class="secondary" onclick={() => (exportOpen = !exportOpen)} disabled={!result}>Export &#9662;</button>
    {#if exportOpen}
      <button class="menu-backdrop" aria-label="Close menu" onclick={() => (exportOpen = false)}></button>
      <div class="menu">
        <button onclick={() => { exportOpen = false; doExport("txt", "txt"); }}>Save report (.txt)</button>
        <button onclick={() => { exportOpen = false; doExport("json", "json"); }}>JSON</button>
        <button onclick={() => { exportOpen = false; doExport("csv", "csv"); }}>CSV</button>
      </div>
    {/if}
  </div>
</div>

{#if busy}
  <p class="progress">{progress ? `Checking ${progress.done} of ${progress.total} — ${progress.cached} cached, ${progress.fetched} fetched` : "Working..."}</p>
{/if}

{#if result}
  <div class="summary">
    <button class:active={filter === "all"} onclick={() => (filter = "all")}>All issues {totalIssues}</button>
    <button class:active={filter === "unresolved"} onclick={() => (filter = "unresolved")}>Unresolved {counts.unresolved + counts.network}</button>
    <button class:active={filter === "mismatch"} onclick={() => (filter = "mismatch")}>Mismatch {counts.mismatch}</button>
    <button class:active={filter === "no_doi"} onclick={() => (filter = "no_doi")}>No DOI {counts.no_doi + counts.no_doi_suggested}</button>
    <input placeholder="Search..." bind:value={query} />
  </div>
  {#if tally.cached + tally.fetched > 0}
    <p class="tally">Resolved: {tally.cached} from cache, {tally.fetched} from Crossref</p>
  {/if}
  {#if counts.network > 0}
    <p class="warn">{counts.network} entr{counts.network === 1 ? "y" : "ies"} couldn't be checked (network or capacity). Use "Re-check failures" when you're back online; everything already resolved is cached.</p>
  {/if}
  {#if !result.bibliography_detected}
    <p class="note">No bibliography heading detected; results came from a whole-document scan.</p>
  {/if}

  {#each issues as x (x.entry.entry.ordinal)}
    <EntryCard entry={x.entry} />
  {/each}
  {#if issues.length === 0}
    <p class="note">No issues to show.</p>
  {/if}

  {#if cleanOnes.length}
    <button class="clean-toggle" onclick={() => (showClean = !showClean)}>
      {showClean ? "▾" : "▸"} {cleanOnes.length} entries matched cleanly
    </button>
    {#if showClean}
      {#each cleanOnes as x (x.entry.entry.ordinal)}
        <EntryCard entry={x.entry} />
      {/each}
    {/if}
  {/if}
{:else if !busy}
  <div class="empty">Open a PDF or .docx, or drop one on the window.</div>
{/if}

<style>
  .toolbar { display: flex; gap: 8px; align-items: center; margin-bottom: 12px; }
  .spacer { flex: 1; }
  button { font: inherit; padding: 4px 12px; }
  .primary { background: #0a84ff; color: #fff; border: 1px solid #0a84ff; border-radius: 6px; }
  .secondary { background: #fff; color: #222; border: 1px solid #c4c4c4; border-radius: 6px; }
  .secondary:disabled { color: #aaa; border-color: #e0e0e0; }
  .exportwrap { position: relative; }
  .menu-backdrop { position: fixed; inset: 0; background: transparent; border: 0; padding: 0; cursor: default; }
  .menu { position: absolute; right: 0; top: 110%; z-index: 2; background: #fff; border: 1px solid #ddd; border-radius: 8px; box-shadow: 0 6px 24px rgba(0,0,0,0.15); display: flex; flex-direction: column; min-width: 150px; overflow: hidden; }
  .menu button { background: #fff; border: 0; border-radius: 0; text-align: left; padding: 8px 12px; }
  .menu button:hover { background: #f0f4ff; }
  .summary { display: flex; gap: 6px; align-items: center; margin-bottom: 10px; flex-wrap: wrap; }
  .summary button { font-size: 12px; padding: 2px 10px; border-radius: 12px; border: 1px solid #ccc; background: #fff; }
  .summary button.active { border-color: #0a52c2; color: #0a52c2; }
  .summary input { margin-left: auto; padding: 3px 8px; font: inherit; }
  .clean-toggle { background: #f4faf5; color: #1a7f37; border: 1px solid #d6ecd9; border-radius: 6px; width: 100%; text-align: left; padding: 6px 10px; }
  .empty { color: #888; border: 2px dashed #ccc; border-radius: 8px; padding: 32px; text-align: center; }
  .note { color: #888; }
  .tally { color: #555; font-size: 12px; margin: 0 0 8px; }
  .warn { color: #9a6700; background: #fffaf0; border: 1px solid #f0e0b0; border-radius: 6px; padding: 6px 10px; }
  .progress { color: #555; }
</style>
