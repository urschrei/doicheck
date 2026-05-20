<script>
  import { onMount } from "svelte";
  import { getEmail, setEmail } from "$lib/api.js";

  let { onclose } = $props();
  let email = $state("");

  onMount(async () => {
    email = await getEmail();
  });

  async function saveAndClose() {
    await setEmail(email);
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
  <div class="actions">
    <button onclick={() => onclose?.()}>Cancel</button>
    <button class="primary" onclick={saveAndClose}>Save</button>
  </div>
</div>

<style>
  .backdrop { position: fixed; inset: 0; background: rgba(0,0,0,0.2); }
  .sheet { position: fixed; top: 20%; left: 50%; transform: translateX(-50%); background: #fff; border-radius: 10px; padding: 20px; width: 360px; box-shadow: 0 10px 40px rgba(0,0,0,0.2); }
  label { display: block; font-size: 12px; color: #555; }
  input { width: 100%; box-sizing: border-box; margin-top: 4px; padding: 6px; font: inherit; }
  .hint { color: #888; font-size: 11px; }
  .actions { display: flex; justify-content: flex-end; gap: 8px; margin-top: 12px; }
  .primary { background: #0a84ff; color: #fff; border: 0; border-radius: 6px; padding: 5px 14px; }
</style>
