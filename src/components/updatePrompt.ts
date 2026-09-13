/**
 * The dialog that offers an update, downloads it, and opens the new version.
 *
 * Three states in one dialog rather than three dialogs: offer, downloading, ready. The
 * dialog is the thing the user is watching, so it stays put and its contents change under
 * them — a modal that closes and reopens twice during one operation reads as three
 * unrelated events.
 *
 * The split between downloading and installing is not cosmetic. On Windows installing
 * launches the installer and exits the app, so a tick and a button shown *after* the
 * install would never be seen. Everything the user is shown therefore happens after the
 * download and before the install, and the final button is what commits to it.
 */

import type { Update } from "@tauri-apps/plugin-updater";
import { h, fill, icon } from "../lib/dom";
import { icons } from "../lib/icons";
import { download, installAndOpen, type Progress } from "../lib/updates";
import { reportError } from "../lib/store";
import { button } from "./common";
import { closeModal, setModalTitle, showModal } from "./modal";
import { progressRing } from "./progressRing";

export function showUpdatePrompt(update: Update, currentVersion: string): void {
  const body = h("div", null);
  const footer = h("div", { class: "modal__foot" });

  // ---- Offer -------------------------------------------------------------

  const offer = () => {
    setModalTitle("Update available");
    fill(
      body,
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
    );

    fill(
      footer,
      button({ label: "Later", variant: "ghost", onClick: () => closeModal() }),
      button({ label: "Download", variant: "primary", onClick: () => void run() }),
    );
  };

  // ---- Downloading -------------------------------------------------------

  const ring = progressRing();
  const line = h("p", { class: "update__line" }, "Starting…");

  const working = () => {
    setModalTitle("Downloading update");
    fill(
      body,
      h(
        "div",
        { class: "update" },
        ring.element,
        h("strong", null, `Oracle ${update.version}`),
        line,
      ),
    );

    // No buttons at all while bytes are moving. Cancelling mid-download would need the
    // plugin to support it, and a button that looks live but does nothing is worse than none.
    fill(footer);
  };

  const onProgress = (progress: Progress) => {
    if (progress.done) {
      line.textContent = "Downloaded and verified.";
      return;
    }

    ring.set(progress.total ? progress.downloaded / progress.total : null);
    line.textContent = describe(progress);
  };

  // ---- Ready -------------------------------------------------------------

  const ready = () => {
    setModalTitle("Ready to install");
    line.textContent = "Ready to install.";

    fill(
      body,
      h(
        "div",
        { class: "update" },
        ring.element,
        h("strong", null, `Oracle ${update.version} is ready`),
        line,
        h(
          "p",
          { class: "field__hint", style: { textAlign: "center" } },
          "Oracle closes for a moment while the installer runs, then opens again.",
        ),
      ),
    );

    fill(
      footer,
      button({ label: "Not now", variant: "ghost", onClick: () => closeModal() }),
      button({
        label: "Install and open",
        variant: "primary",
        onClick: () => void commit(),
      }),
    );

    // After the ring is back in the document: re-parenting an element restarts its CSS
    // animations, so completing it first would play the tick into a detached node and then
    // replay it, or drop it entirely.
    ring.succeed();
  };

  const run = async () => {
    working();

    try {
      await download(update, onProgress);
      ready();
    } catch (error) {
      failed("The download did not finish.");
      reportError("Could not download the update", error);
    }
  };

  const commit = async () => {
    line.textContent = "Installing…";
    fill(footer);

    try {
      await installAndOpen(update);
    } catch (error) {
      failed("The update could not be installed.");
      reportError("Could not install the update", error);
    }
  };

  const failed = (message: string) => {
    line.textContent = message;
    fill(
      footer,
      button({ label: "Close", variant: "ghost", onClick: () => closeModal() }),
      button({ label: "Try again", variant: "primary", onClick: () => void run() }),
    );
  };

  offer();

  showModal({
    title: "Update available",
    body,
    // The footer is swapped as the state changes, so the dialog owns an element rather than
    // a fixed list of buttons.
    footer,
    width: 460,
  });
}

/** A progress line that says something useful even without a content length. */
function describe(progress: Progress): string {
  const mb = (bytes: number) => `${(bytes / 1_000_000).toFixed(1)} MB`;

  return progress.total
    ? `${mb(progress.downloaded)} of ${mb(progress.total)}`
    : `${mb(progress.downloaded)} downloaded`;
}
