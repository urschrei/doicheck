<script>
  import { onMount } from "svelte";
  import * as api from "$lib/api.js";
  import { getCurrentWebview } from "@tauri-apps/api/webview";
  import { ask } from "@tauri-apps/plugin-dialog";
  import Sidebar from "$lib/Sidebar.svelte";
  import ReportPane from "$lib/ReportPane.svelte";
  import Settings from "$lib/Settings.svelte";
  import Help from "$lib/Help.svelte";
  import { checkForUpdate } from "$lib/update.js";

  let documents = $state([]);
  let result = $state(null);
  let currentPath = $state("");
  let busy = $state(false);
  let progress = $state(null);
  let error = $state("");
  let showSettings = $state(false);
  let showHelp = $state(false);
  let helpTab = $state("guide");
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
      selectedFingerprint = result.fingerprint;
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
        selectedFingerprint = stored.fingerprint;
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
    let unlistenAbout;
    let unlistenSettings;
    (async () => {
      unlistenProgress = await api.onProgress((p) => (progress = p));
      unlistenDrag = await getCurrentWebview().onDragDropEvent((event) => {
        if (event.payload.type === "drop" && event.payload.paths.length) {
          openPath(event.payload.paths[0]);
        }
      });
      unlistenAbout = await api.onOpenAbout(() => {
        helpTab = "about";
        showHelp = true;
      });
      unlistenSettings = await api.onOpenSettings(() => {
        showSettings = true;
      });
    })();
    return () => {
      unlistenProgress?.();
      unlistenDrag?.();
      unlistenAbout?.();
      unlistenSettings?.();
    };
  });
</script>

<main class="layout">
  <Sidebar {documents} selected={selectedFingerprint} onselect={selectDocument} onsettings={() => (showSettings = true)} onhelp={() => { helpTab = "guide"; showHelp = true; }} ondelete={deleteDocument} />
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
  {#if showHelp}
    <Help onclose={() => (showHelp = false)} initialTab={helpTab} />
  {/if}
</main>

<style>
  :global(:root) {
    color-scheme: light;
    --bg: #ffffff;
    --bg-sidebar: #f7f7f7;
    --bg-elevated: #ffffff;
    --bg-hover: #ececec;
    --border: #e3e3e3;
    --border-soft: #eeeeee;
    --text: #1a1a1a;
    --text-muted: #888888;
    --accent: #0a52c2;
    --accent-soft-bg: #eef2ff;
    --sel-bg: #0a52c2;
    --sel-fg: #ffffff;
    --backdrop: rgba(0, 0, 0, 0.2);
    --danger: #b00020;
    --integrity: #d70015;
    --sev-fail: #ff5f57;
    --sev-warn: #febc2e;
    --sev-info: #3b82f6;
    --sev-ok: #28c840;
    --sev-incomplete: #ff9f0a;
  }

  @media (prefers-color-scheme: dark) {
    :global(:root) {
      color-scheme: dark;
      --bg: #1e1e1e;
      --bg-sidebar: #252526;
      --bg-elevated: #2a2a2b;
      --bg-hover: #333335;
      --border: #3a3a3c;
      --border-soft: #333335;
      --text: #e8e8e8;
      --text-muted: #9a9a9a;
      --accent: #5a9bff;
      --accent-soft-bg: #1f2c47;
      --sel-bg: #2d5fa6;
      --sel-fg: #ffffff;
      --backdrop: rgba(0, 0, 0, 0.5);
      --danger: #ff6b6b;
      --integrity: #ff453a;
      --sev-fail: #ff6b66;
      --sev-warn: #f5c542;
      --sev-info: #6aa8ff;
      --sev-ok: #3ad44f;
      --sev-incomplete: #ffac3a;
    }
  }

  :global(body) {
    margin: 0;
    background: var(--bg);
    color: var(--text);
  }

  .layout { display: grid; grid-template-columns: 240px 1fr; height: 100vh; font: 13px -apple-system, system-ui, sans-serif; }
  .pane { padding: 16px; overflow: auto; }
  .error { color: var(--danger); }
</style>
