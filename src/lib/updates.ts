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
 * Downloads the update and verifies it, without installing.
 *
 * Separating this from the install is what makes the dialog possible: on Windows installing
 * launches the installer and exits the app immediately, so anything meant to be shown
 * *after* a successful update — a tick, a button — has to happen before that point. The
 * download is also the slow half, and the only half worth a progress indicator.
 *
 * `onProgress` is called as bytes arrive. A server that declares no length leaves `total`
 * null, which the caller shows as indeterminate rather than guessing.
 */
export async function download(
  update: Update,
  onProgress: (progress: Progress) => void,
): Promise<void> {
  let downloaded = 0;
  let total: number | null = null;

  await update.download((event: DownloadEvent) => {
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
}

/**
 * Installs what was downloaded and opens the new version.
 *
 * Does not return on Windows: the installer is launched, this process exits, and the
 * installer starts the new build. Elsewhere the install completes in place and the restart
 * is ours to ask for.
 */
export async function installAndOpen(update: Update): Promise<void> {
  await update.install({ restartAfterInstall: true });

  // Reached only where the install did not replace the process.
  await relaunch();
}
