/**
 * Self-update.
 *
 * Oracle ships as an installer people download once, so without this an old build stays old
 * forever. The plugin does the transport; what lives here is the policy: when to look, what
 * to say, and what a failure means.
 *
 * Nothing here is silent. An update is downloaded and installed only after the user agrees,
 * because installing means replacing the running application and restarting it — not
 * something to do under someone while they are watching a dev server boot.
 */

import { check, type DownloadEvent, type Update } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";

/** How far along an install is, for the progress line in the dialog. */
export interface Progress {
  /** Bytes written so far. */
  downloaded: number;
  /** Total bytes, when the server declared a length. */
  total: number | null;
  done: boolean;
}

/**
 * Asks the release feed whether there is something newer.
 *
 * Returns `null` both when the app is current and when the check could not be made — a
 * machine that is offline, or a GitHub that is down, is not a situation worth interrupting
 * anyone over. A manual check reports the failure itself; see `checkNow`.
 */
export async function findUpdate(): Promise<Update | null> {
  try {
    return await check();
  } catch {
    return null;
  }
}

/**
 * The same check, but surfacing why it failed.
 *
 * Used from Settings, where the user asked the question and is owed an answer.
 */
export async function checkNow(): Promise<{ update: Update | null; error: string | null }> {
  try {
    return { update: await check(), error: null };
  } catch (error) {
    return { update: null, error: String(error) };
  }
}

/**
 * Downloads and installs an update, then restarts into it.
 *
 * `onProgress` is called as bytes arrive so a slow connection does not look like a hang.
 * The function does not return on success: the process is replaced.
 */
export async function install(
  update: Update,
  onProgress: (progress: Progress) => void,
): Promise<void> {
  let downloaded = 0;
  let total: number | null = null;

  await update.downloadAndInstall((event: DownloadEvent) => {
    switch (event.event) {
      case "Started":
        total = event.data.contentLength ?? null;
        onProgress({ downloaded: 0, total, done: false });
        break;
      case "Progress":
        downloaded += event.data.chunkLength;
        onProgress({ downloaded, total, done: false });
        break;
      case "Finished":
        onProgress({ downloaded, total, done: true });
        break;
    }
  });

  // The installer has already replaced the files on disk; this window is running the old
  // build until it is restarted.
  await relaunch();
}
