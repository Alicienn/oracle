/**
 * The one step in updating that needs asking about.
 *
 * Installing is not "apply a patch": the installer replaces Oracle and Oracle has to be
 * gone while it does. Anything Oracle is running — dev servers, containers, whatever the
 * user launched — are its own child processes, and leaving them alive across an install
 * orphans them: still holding their ports, with nothing left to stop them from.
 *
 * So they are stopped first, deliberately and visibly, and the dialog says so before the
 * user commits rather than after.
 */

import type { Update } from "@tauri-apps/plugin-updater";
import { h, fill, icon } from "../lib/dom";
import { icons } from "../lib/icons";
import { stopAll } from "../lib/actions";
import { get, reportError } from "../lib/store";
import { installAndOpen } from "../lib/updates";
import { isLive, button } from "./common";
import { closeModal, showModal } from "./modal";

export function confirmInstall(update: Update): void {
  const live = get().projects.filter((project) =>
    isLive(project.status, get().pending[project.id]),
  );

  const status = h("p", { class: "field__hint", style: { marginTop: "16px" } });
  const footer = h("div", { class: "modal__foot" });

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
        h("strong", null, `Install Oracle ${update.version}`),
        h("p", null, "Oracle closes while the installer runs, then opens again."),
      ),
    ),
    // Named, not counted: "3 projects will be stopped" leaves the user wondering which, and
    // the answer changes whether they accept now or after lunch.
    live.length > 0
      ? h(
          "div",
          { class: "warning" },
          h("span", { class: "warning__mark" }, icon(icons.alert)),
          h(
            "div",
            null,
            h(
              "strong",
              null,
              live.length === 1
                ? "1 project is running and will be stopped"
                : `${live.length} projects are running and will be stopped`,
            ),
            h("p", null, live.map((project) => project.name).join(", ")),
          ),
        )
      : null,
    update.body ? h("div", { class: "release-notes" }, update.body) : null,
    status,
  );

  const run = async () => {
    fill(footer);

    try {
      if (live.length > 0) {
        status.textContent = "Stopping projects…";
        // Sequential, and each one waits out its grace period: an installer that starts
        // while a dev server is still shutting down is how a port stays held.
        await stopAll();
      }

      status.textContent = "Starting the installer…";
      await installAndOpen(update);
    } catch (error) {
      status.textContent = "The update could not be installed.";
      offer();
      reportError("Could not install the update", error);
    }
  };

  const offer = () => {
    fill(
      footer,
      button({ label: "Cancel", variant: "ghost", onClick: () => closeModal() }),
      button({
        label: live.length > 0 ? "Stop and install" : "Install and open",
        variant: "primary",
        onClick: () => void run(),
      }),
    );
  };

  offer();

  showModal({ title: "Install update", body, footer, width: 460 });
}
