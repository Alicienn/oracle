/**
 * The dialog that offers an update, and reports on installing it.
 *
 * One component for both entry points — the check at launch and the button in Settings — so
 * there is a single description of what an update is and what accepting one does.
 */

import type { Update } from "@tauri-apps/plugin-updater";
import { h, icon } from "../lib/dom";
import { icons } from "../lib/icons";
import { install, type Progress } from "../lib/updates";
import { reportError } from "../lib/store";
import { button } from "./common";
import { closeModal, showModal } from "./modal";

/** Shows the offer. Resolves when the dialog closes, one way or another. */
export function showUpdatePrompt(update: Update, currentVersion: string): void {
  const status = h("p", { class: "field__hint", style: { marginTop: "16px" } },
    "Oracle will restart once the update is installed. Your running projects are stopped first.",
  );

  const body = h(
    "div",
    null,
    h(
      "div",
      { class: "notice" },
      h("span", { class: "notice__mark notice__mark--accent" }, icon(icons.cloud)),
      h(
        "div",
        null,
        h("strong", null, `Oracle ${update.version} is available`),
        h("p", null, `You are running ${currentVersion}.`),
      ),
    ),
    update.body ? h("div", { class: "release-notes" }, update.body) : null,
    status,
  );

  const accept = button({
    label: "Install and restart",
    variant: "primary",
    onClick: () => void run(),
  });

  const later = button({ label: "Later", variant: "ghost", onClick: () => closeModal() });

  const run = async () => {
    accept.disabled = true;
    later.disabled = true;

    try {
      await install(update, (progress) => {
        status.textContent = describe(progress);
      });
    } catch (error) {
      // The dialog stays open: the user asked for this and needs to see that it failed.
      accept.disabled = false;
      later.disabled = false;
      status.textContent = "The update could not be installed.";
      reportError("Could not install the update", error);
    }
  };

  showModal({
    title: "Update available",
    body,
    actions: [later, accept],
    width: 460,
  });
}

/** A progress line that says something useful even without a content length. */
function describe(progress: Progress): string {
  if (progress.done) return "Installing, then restarting…";

  const mb = (bytes: number) => `${(bytes / 1_000_000).toFixed(1)} MB`;

  return progress.total
    ? `Downloading ${mb(progress.downloaded)} of ${mb(progress.total)}…`
    : `Downloading ${mb(progress.downloaded)}…`;
}
