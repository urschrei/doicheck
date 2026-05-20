<script>
  import { onMount } from "svelte";
  import * as api from "$lib/api.js";
  import { getCurrentWebview } from "@tauri-apps/api/webview";
  import Sidebar from "$lib/Sidebar.svelte";
  import ReportPane from "$lib/ReportPane.svelte";
  import Settings from "$lib/Settings.svelte";

  let documents = $state([]);
  let report = $state("");
  let currentPath = $state("");
  let busy = $state(false);
  let progress = $state(null);
  let error = $state("");
  let showSettings = $state(false);
  let selectedFingerprint = $state("");

  async function refresh() {
    documents = await api.listDocuments();
  }

  async function runCheck(path) {
    error = "";
    busy = true;
    progress = null;
    currentPath = path;
    try {
      report = await api.checkDocument(path);
      await refresh();
    } catch (e) {
      error = String(e);
    } finally {
      busy = false;
      progress = null;
    }
  }

  // Opening or dropping a file: show the stored report if this document has
  // been checked before, otherwise run a fresh check.
  async function openPath(path) {
    error = "";
    currentPath = path;
    try {
      const stored = await api.openDocument(path);
      if (stored) {
        report = stored;
      } else {
        await runCheck(path);
      }
    } catch (e) {
      error = String(e);
    }
  }

  async function selectDocument(fingerprint) {
    selectedFingerprint = fingerprint;
    const stored = await api.reportByFingerprint(fingerprint);
    if (stored) report = stored;
  }

  onMount(() => {
    refresh();
    let unlistenProgress;
    let unlistenDrag;
    (async () => {
      unlistenProgress = await api.onProgress((p) => (progress = p));
      unlistenDrag = await getCurrentWebview().onDragDropEvent((event) => {
        if (event.payload.type === "drop" && event.payload.paths.length) {
          openPath(event.payload.paths[0]);
        }
      });
    })();
    return () => {
      unlistenProgress?.();
      unlistenDrag?.();
    };
  });
</script>

<main class="layout">
  <Sidebar {documents} onselect={selectDocument} onsettings={() => (showSettings = true)} />
  <section class="pane">
    {#if error}<p class="error">{error}</p>{/if}
    <ReportPane
      {report}
      {busy}
      {progress}
      {currentPath}
      onopen={openPath}
      onrecheck={() => currentPath && runCheck(currentPath)}
    />
  </section>
  {#if showSettings}
    <Settings onclose={() => (showSettings = false)} />
  {/if}
</main>

<style>
  :global(body) { margin: 0; }
  .layout { display: grid; grid-template-columns: 240px 1fr; height: 100vh; font: 13px -apple-system, system-ui, sans-serif; }
  .pane { padding: 16px; overflow: auto; }
  .error { color: #b00020; }
</style>
