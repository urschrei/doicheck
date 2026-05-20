<script>
  import { open, save } from "@tauri-apps/plugin-dialog";
  import { exportReport } from "$lib/api.js";

  let { report = "", busy = false, progress = null, currentPath = "", onopen, onrecheck } = $props();

  async function pickAndCheck() {
    const path = await open({
      multiple: false,
      filters: [{ name: "Documents", extensions: ["pdf", "docx"] }],
    });
    if (path) onopen?.(path);
  }

  async function doExport() {
    const path = await save({
      defaultPath: "doi-report.txt",
      filters: [{ name: "Text", extensions: ["txt"] }],
    });
    if (path) await exportReport(path, report);
  }
</script>

<div class="toolbar">
  <button onclick={pickAndCheck} disabled={busy}>Open</button>
  <button onclick={() => onrecheck?.()} disabled={busy || !currentPath}>Re-check</button>
  <button onclick={doExport} disabled={!report}>Export</button>
</div>

{#if busy}
  <p class="progress">{progress ? `Checking ${progress.done} of ${progress.total}...` : "Working..."}</p>
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
