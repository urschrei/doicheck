<script>
  import { onMount } from "svelte";
  import { getEmail, setEmail, getReportsDir, setReportsDir, getConcurrency, setConcurrency } from "$lib/api.js";
  import { open } from "@tauri-apps/plugin-dialog";

  let { onclose } = $props();
  let email = $state("");
  let reportsDir = $state("");
  let concurrency = $state(5);

  onMount(async () => {
    email = await getEmail();
    reportsDir = (await getReportsDir()) ?? "";
    concurrency = await getConcurrency();
  });

  async function pickDir() {
    const dir = await open({ directory: true });
    if (dir) reportsDir = dir;
  }

  async function saveAndClose() {
    await setEmail(email);
    await setReportsDir(reportsDir);
    await setConcurrency(concurrency);
    onclose?.();
  }
</script>

<div class="backdrop" role="presentation" onclick={() => onclose?.()}></div>
<div class="sheet">
  <h3>Settings</h3>
  <label>Crossref contact email
    <input bind:value={email} type="email" placeholder="you@example.com" />
  </label>
  <p class="hint">Used for the Crossref polite pool. Leave blank to stay anonymous.</p>
  <label>Default reports folder
    <span class="row">
      <input bind:value={reportsDir} placeholder="(ask each time)" />
      <button onclick={pickDir}>Choose...</button>
    </span>
  </label>
  <p class="hint">Save dialogs will default to this folder.</p>
  <label>Parallel Crossref lookups
    <input bind:value={concurrency} type="number" min="1" max="20" />
  </label>
  <p class="hint">Number of concurrent Crossref requests (1–20).</p>
  <div class="actions">
    <button onclick={() => onclose?.()}>Cancel</button>
    <button class="primary" onclick={saveAndClose}>Save</button>
  </div>
</div>

<style>
  .backdrop { position: fixed; inset: 0; background: var(--backdrop); }
  .sheet { position: fixed; top: 16%; left: 50%; transform: translateX(-50%); background: var(--bg-elevated); color: var(--text); border-radius: 10px; padding: 20px; width: 420px; box-shadow: 0 10px 40px rgba(0,0,0,0.2); border: 1px solid var(--border); }
  label { display: block; font-size: 12px; color: var(--text-muted); margin-top: 8px; }
  input { width: 100%; box-sizing: border-box; margin-top: 4px; padding: 6px; font: inherit; background: var(--bg); color: var(--text); border: 1px solid var(--border); border-radius: 4px; }
  .row { display: flex; gap: 6px; }
  .row input { flex: 1; }
  .hint { color: var(--text-muted); font-size: 11px; }
  .actions { display: flex; justify-content: flex-end; gap: 8px; margin-top: 12px; }
  .primary { background: var(--accent); color: #fff; border: 0; border-radius: 6px; padding: 5px 14px; }
  button { font: inherit; color: var(--text); }
</style>
