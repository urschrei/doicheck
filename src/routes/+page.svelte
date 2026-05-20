<script>
  import { onMount } from "svelte";
  import * as api from "$lib/api.js";
  import { getCurrentWebview } from "@tauri-apps/api/webview";
  import { ask } from "@tauri-apps/plugin-dialog";
  import Sidebar from "$lib/Sidebar.svelte";
  import ReportPane from "$lib/ReportPane.svelte";
  import Settings from "$lib/Settings.svelte";
  import { checkForUpdate } from "$lib/update.js";

  let documents = $state([]);
  let result = $state(null);
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
      result = await api.checkDocument(path);
      await refresh();
    } catch (e) {
      error = String(e);
    } finally {
      busy = false;
      progress = null;
    }
  }

  async function recheckFailures() {
    if (!result) return;
    error = "";
    busy = true;
    progress = null;
    try {
      result = await api.recheckFailures(result.fingerprint);
      await refresh();
    } catch (e) {
      error = String(e);
    } finally {
      busy = false;
      progress = null;
    }
  }

  // Opening or dropping a file: show the stored result if this document has
  // been checked before, otherwise run a fresh check.
  async function openPath(path) {
    error = "";
    currentPath = path;
    try {
      const stored = await api.openDocument(path);
      if (stored) {
        result = stored;
      } else {
        await runCheck(path);
      }
    } catch (e) {
      error = String(e);
    }
  }

  async function selectDocument(fingerprint) {
    selectedFingerprint = fingerprint;
    const stored = await api.latestCheck(fingerprint);
    if (stored) result = stored;
  }

  async function deleteDocument(fingerprint) {
    const ok = await ask("Remove this document and its reports? Cached DOIs are kept.", {
      title: "Remove document",
      kind: "warning",
    });
    if (!ok) return;
    await api.deleteDocument(fingerprint);
    if (selectedFingerprint === fingerprint) {
      result = null;
      selectedFingerprint = "";
    }
    await refresh();
  }

  async function dismissDiscrepancy(doi, field) {
    if (!result) return;
    await api.dismissDiscrepancy(result.fingerprint, doi, field);
    result = await api.latestCheck(result.fingerprint);
    await refresh();
  }

  async function undismissDiscrepancy(doi, field) {
    if (!result) return;
    await api.undismissDiscrepancy(result.fingerprint, doi, field);
    result = await api.latestCheck(result.fingerprint);
    await refresh();
  }

  onMount(() => {
    refresh();
    checkForUpdate();
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
  <Sidebar {documents} onselect={selectDocument} onsettings={() => (showSettings = true)} ondelete={deleteDocument} />
  <section class="pane">
    {#if error}<p class="error">{error}</p>{/if}
    <ReportPane
      {result}
      {busy}
      {progress}
      {currentPath}
      onopen={openPath}
      onrecheck={() => currentPath && runCheck(currentPath)}
      onrecheckfailures={recheckFailures}
      ondismiss={dismissDiscrepancy}
      onundismiss={undismissDiscrepancy}
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
