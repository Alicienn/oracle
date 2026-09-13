/**
 * What changed, offered after an update.
 *
 * An update that installs silently and reopens leaves the user in a version they did not
 * read about. This is the other half of the update flow: a line in the title bar, next to
 * where the download offer was, saying the notes are there if wanted.
 *
 * It carries its own cross, because "I have read enough" is a thing someone should be able
 * to say without opening anything. Either gesture — reading the notes or dismissing them —
 * records the version as seen, and the chip does not come back until the next update.
 */

import { h, fill, icon, qs } from "../lib/dom";
import { icons } from "../lib/icons";
import { api } from "../lib/api";
import { get, saveSettings } from "../lib/store";
import { showModal } from "./modal";

/**
 * Shows the chip if this launch is the first on a new version.
 *
 * Silent on a first run: an install that upgraded from nothing has no release notes worth
 * pushing at someone, which is why the stored version starts empty rather than at the
 * running one.
 */
export async function offerChangelog(): Promise<void> {
  const { version, settings } = get();
  const seen = settings.lastSeenVersion;

  if (!version || seen === version) return;

  if (!seen) {
    // First run. Record where we are so the next update is recognised as one.
    await saveSettings({ lastSeenVersion: version });
    return;
  }

  const notes = await api.changelog();
  if (!notes) {
    // Nothing written for this version; nothing to offer, and nothing to keep asking about.
    await saveSettings({ lastSeenVersion: version });
    return;
  }

  mount(version, notes);
}

function mount(version: string, notes: string): void {
  const host = qs("#changelog");

  const dismiss = () => {
    fill(host);
    void saveSettings({ lastSeenVersion: version });
  };

  const chip = h(
    "span",
    { class: "chip chip--closable", dataset: { state: "news" } },
    h(
      "button",
      {
        class: "chip__main",
        type: "button",
        title: `What changed in Oracle ${version}`,
        onClick: () => show(version, notes, dismiss),
      },
      icon(icons.info),
      h("span", null, "Changelog"),
    ),
    h(
      "button",
      {
        class: "chip__close",
        type: "button",
        "aria-label": "Dismiss the changelog",
        title: "Dismiss",
        onClick: dismiss,
      },
      icon(icons.close),
    ),
  );

  fill(host, chip);
}

function show(version: string, notes: string, onRead: () => void): void {
  showModal({
    title: `What's new in ${version}`,
    body: h("div", { class: "release-notes release-notes--tall" }, notes),
    width: 520,
    // Reading them counts as having seen them, however the dialog is closed.
    onClose: onRead,
  });
}
