import { check } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";
import { ask } from "@tauri-apps/plugin-dialog";

// Check GitHub releases for a newer version; if found, offer to install and
// relaunch. Best-effort: any failure (offline, no release yet) is swallowed.
export async function checkForUpdate() {
  try {
    const update = await check();
    if (!update) return;
    const ok = await ask(
      `Version ${update.version} is available (you have ${update.currentVersion}). Install and restart now?`,
      { title: "Update available", kind: "info" },
    );
    if (!ok) return;
    await update.downloadAndInstall();
    await relaunch();
  } catch (e) {
    console.error("update check failed", e);
  }
}
