<script>
  import { onMount } from "svelte";
  import { openUrl } from "@tauri-apps/plugin-opener";
  import { getVersion } from "@tauri-apps/api/app";

  let { onclose, initialTab = "guide" } = $props();
  let tab = $state(initialTab);
  let version = $state("");

  onMount(async () => {
    try {
      version = await getVersion();
    } catch {
      version = "";
    }
  });

  function open(url) {
    openUrl(url);
  }
</script>

<div class="backdrop" role="presentation" onclick={() => onclose?.()}></div>
<div class="sheet">
  <div class="tabs">
    <button class:active={tab === "guide"} onclick={() => (tab = "guide")}>Using DOI Checker</button>
    <button class:active={tab === "about"} onclick={() => (tab = "about")}>About</button>
    <button class="close" aria-label="Close" onclick={() => onclose?.()}>&#10005;</button>
  </div>

  <div class="body">
    {#if tab === "guide"}
      <h3>Checking a document</h3>
      <ul>
        <li>Drag a PDF or <code>.docx</code> onto the window, or use <b>Open</b>.</li>
        <li>The bibliography is detected, its DOIs are checked against Crossref and
          DataCite, and the returned metadata is compared with each reference.</li>
        <li>A document you have checked before opens straight to its stored report;
          the sidebar lists everything you have checked.</li>
      </ul>

      <h3>Reading the results</h3>
      <ul>
        <li>Entries are shown as cards, problems first. Severity: red = the DOI did not
          resolve, amber = a metadata mismatch, blue = no DOI (with a suggested match
          where one was found), green = clean.</li>
        <li>The sidebar dot mirrors this; an orange &#8635; means the last check was
          interrupted (network/capacity) and has entries still to check.</li>
        <li>DOIs and URLs are clickable and open in your browser.</li>
      </ul>

      <h3>Mismatches and false positives</h3>
      <ul>
        <li>A flagged field shows what Crossref or DataCite holds; if it is wrong, use
          <b>mark false positive</b>. Dismissals are remembered for that document and
          can be undone.</li>
      </ul>

      <h3>Re-checking, cache and offline</h3>
      <ul>
        <li>Resolved DOIs are cached locally and shared across documents, so each DOI is
          fetched once; the report shows how many came from the cache.</li>
        <li><b>Re-check failures</b> retries only the entries that could not be checked
          (e.g. after losing connectivity); <b>Re-check entire doc</b> re-runs the whole
          document.</li>
      </ul>

      <h3>Saving and settings</h3>
      <ul>
        <li>Export the report as text, JSON, or CSV; the save dialog remembers your
          reports folder.</li>
        <li>Settings (the gear) holds your Crossref contact email and default reports
          folder. Remove a document with the &#10005; on its sidebar row — cached DOIs
          are kept.</li>
        <li>The app checks for a newer version on launch and offers to install it.</li>
      </ul>
    {:else}
      <h3>DOI Checker{version ? ` ${version}` : ""}</h3>
      <p>Check the DOIs in a document's bibliography against Crossref and DataCite.</p>

      <p class="field"><span>Author</span> Stephan Hügel</p>
      <p class="field"><span>Affiliation</span> Department of Geography, Trinity College Dublin</p>
      <p class="field"><span>Contact</span>
        <button class="link" onclick={() => open("mailto:shugel@tcd.ie")}>shugel@tcd.ie</button></p>
      <p class="field"><span>Source</span>
        <button class="link" onclick={() => open("https://github.com/urschrei/doicheck")}>github.com/urschrei/doicheck</button></p>
      <p class="field"><span>Licence</span>
        <button class="link" onclick={() => open("https://blueoakcouncil.org/license/1.0.0")}>Blue Oak Model License 1.0.0</button></p>

      <p class="meta">Built with Tauri, Svelte, and Rust. Bibliographic data from
        <button class="link" onclick={() => open("https://www.crossref.org")}>Crossref</button>
        and <button class="link" onclick={() => open("https://datacite.org")}>DataCite</button>.</p>
      <p class="meta">© 2026 Stephan Hügel. Provided as is, without warranty.</p>
    {/if}
  </div>
</div>

<style>
  .backdrop { position: fixed; inset: 0; background: var(--backdrop); }
  .sheet { position: fixed; top: 10%; left: 50%; transform: translateX(-50%); background: var(--bg-elevated); color: var(--text); border-radius: 10px; width: 520px; max-width: 90vw; max-height: 80vh; display: flex; flex-direction: column; box-shadow: 0 10px 40px rgba(0,0,0,0.2); overflow: hidden; border: 1px solid var(--border); }
  .tabs { display: flex; align-items: center; gap: 4px; border-bottom: 1px solid var(--border-soft); padding: 6px 8px; }
  .tabs button { font: inherit; font-size: 13px; padding: 4px 10px; border: 0; background: transparent; border-radius: 6px; cursor: pointer; color: var(--text-muted); }
  .tabs button.active { background: var(--accent-soft-bg); color: var(--accent); }
  .tabs .close { margin-left: auto; color: var(--text-muted); }
  .body { padding: 14px 18px; overflow: auto; }
  h3 { margin: 14px 0 6px; font-size: 13px; }
  h3:first-child { margin-top: 0; }
  ul { margin: 0; padding-left: 18px; }
  li { margin: 3px 0; }
  code { font-family: ui-monospace, Menlo, monospace; }
  .field { margin: 6px 0; }
  .field span { display: inline-block; width: 90px; color: var(--text-muted); }
  .meta { color: var(--text-muted); font-size: 12px; margin: 10px 0 0; }
  .link { border: 0; background: transparent; color: var(--accent); text-decoration: underline; cursor: pointer; font: inherit; padding: 0; }
</style>
