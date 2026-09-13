/**
 * The update indicator in the title bar.
 *
 * An update is worth telling someone about; it is not worth a modal over whatever they
 * opened the app to do. So it arrives as a line of text they can ignore, and the whole
 * sequence — download, verify, install — happens in place, in that same small space.
 *
 * The only thing that does take a dialog is the install itself, because it is the one step
 * with consequences: Oracle closes, and anything it is running has to be stopped first.
 */

import type { Update } from "@tauri-apps/plugin-updater";
import { h, fill, icon, qs } from "../lib/dom";
import { icons } from "../lib/icons";
import { download, type Progress } from "../lib/updates";
import { reportError } from "../lib/store";
import { confirmInstall } from "./installUpdate";
import { progressRing } from "./progressRing";

const RING_SIZE = 15;

/**
 * Shows the indicator, and owns it from there on.
 *
 * Mounted rather than rendered: the chip walks through states of its own and must not be
 * rebuilt by the render loop, which would restart its animations once a second.
 */
export function mountUpdateChip(update: Update): void {
  const ring = progressRing(RING_SIZE);
  const label = h("span", null, "Download update");

  const chip = h("button", {
    class: "chip",
    type: "button",
    title: `Oracle ${update.version} is available`,
  });

  const available = () => {
    chip.disabled = false;
    chip.dataset.state = "available";
    label.textContent = "Download update";
    fill(chip, icon(icons.cloud), label);
    chip.onclick = () => void run();
  };

  const downloading = (progress: Progress) => {
    ring.set(progress.total ? progress.downloaded / progress.total : null);

    label.textContent = progress.total
      ? `${Math.round((progress.downloaded / progress.total) * 100)}%`
      : "Downloading…";
  };

  const ready = () => {
    chip.disabled = false;
    chip.dataset.state = "ready";
    label.textContent = "Install update";
    chip.title = `Oracle ${update.version} is downloaded and ready to install`;
    chip.onclick = () => void confirmInstall(update);

    // The ring is already in the chip; completing it here rather than rebuilding keeps the
    // tick animation from being cut off by a re-render.
    ring.succeed();
  };

  const run = async () => {
    chip.disabled = true;
    chip.dataset.state = "working";
    chip.title = `Downloading Oracle ${update.version}`;
    fill(chip, ring.element, label);
    label.textContent = "Starting…";

    try {
      await download(update, (progress) => {
        if (progress.done) return;
        downloading(progress);
      });
      ready();
    } catch (error) {
      chip.disabled = false;
      chip.dataset.state = "failed";
      label.textContent = "Retry download";
      fill(chip, icon(icons.alert), label);
      chip.onclick = () => void run();
      reportError("Could not download the update", error);
    }
  };

  available();
  fill(qs("#update"), chip);
}
