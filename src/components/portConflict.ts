/**
 * The dialog shown when a project cannot start because its port is taken.
 *
 * Kept apart from the generic error toast on purpose: this is the one launch failure where
 * Oracle knows both what went wrong and what to do about it, so the user should be offered
 * the fix rather than told to go and find it.
 */

import type { PortConflict, ProjectView } from "../lib/api";
import { h } from "../lib/dom";
import { button } from "./common";
import { closeModal, showModal } from "./modal";

/**
 * Asks whether to start on a different port.
 *
 * Resolves to the port the user accepted, or `null` if they declined — which covers
 * Cancel, Escape, a click on the backdrop, and the case where no free port was found.
 */
export function askAboutPort(
  project: ProjectView,
  conflict: PortConflict,
): Promise<number | null> {
  return new Promise((resolve) => {
    let decided = false;

    const settle = (value: number | null) => {
      if (decided) return;
      decided = true;
      resolve(value);
    };

    const body = h(
      "div",
      null,
      h(
        "p",
        null,
        `Port ${conflict.port} is already held by ${conflict.holder}, so ${project.name} cannot use it.`,
      ),
      conflict.suggestion !== null
        ? h(
            "p",
            { class: "field__hint" },
            `Oracle can start it on port ${conflict.suggestion} instead. That applies to this run only — the project keeps ${conflict.port} in its settings, and the new port is passed to the command as PORT.`,
          )
        : h(
            "p",
            { class: "field__hint" },
            "Every port Oracle tried nearby is also taken. Stop whatever is using this range, or give the project a different port in its settings.",
          ),
    );

    const actions = [
      button({
        label: "Cancel",
        variant: "ghost",
        onClick: () => {
          settle(null);
          closeModal();
        },
      }),
      button({
        label: "Settings",
        onClick: () => {
          settle(null);
          closeModal();
          // Imported here rather than at the top: the form imports the actions module,
          // which imports this one, and resolving that cycle lazily keeps both usable.
          void import("./projectForm").then((module) => module.showProjectForm(project));
        },
      }),
    ];

    if (conflict.suggestion !== null) {
      const port = conflict.suggestion;
      actions.push(
        button({
          label: `Start on ${port}`,
          variant: "primary",
          onClick: () => {
            settle(port);
            closeModal();
          },
        }),
      );
    }

    showModal({
      title: "Port already in use",
      body,
      actions,
      width: 460,
      // Covers Escape and the backdrop, neither of which routes through a button.
      onClose: () => settle(null),
    });
  });
}
