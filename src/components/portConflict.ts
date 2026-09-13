/**
 * The dialog shown when a project cannot start because its port is taken.
 *
 * Kept apart from the generic error toast on purpose: this is the one launch failure where
 * Oracle knows both what went wrong and what to do about it, so the user should be offered
 * the fix rather than told to go and find it.
 */

import type { PortConflict, ProjectView } from "../lib/api";
import { h, icon } from "../lib/dom";
import { icons } from "../lib/icons";
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
        "div",
        { class: "notice" },
        h("span", { class: "notice__mark" }, icon(icons.alert)),
        h(
          "div",
          null,
          h("strong", null, `Port ${conflict.port} is taken`),
          // One phrasing for both kinds of holder: an Oracle project answers with its name,
          // anything else with a description of the process.
          h("p", null, `Held by ${conflict.holder}.`),
        ),
      ),
      conflict.suggestion !== null
        ? h(
            "div",
            { class: "swap" },
            h("div", { class: "swap__port" }, String(conflict.port), h("span", null, "in use")),
            h("span", { class: "swap__arrow", "aria-hidden": "true" }, "→"),
            h(
              "div",
              { class: "swap__port swap__port--target" },
              String(conflict.suggestion),
              h("span", null, "free"),
            ),
          )
        : null,
      h(
        "p",
        { class: "field__hint", style: { marginTop: "16px" } },
        conflict.suggestion !== null
          ? `For this run only. ${project.name} keeps port ${conflict.port} in its settings, and the new port reaches the command as PORT.`
          : "Every port Oracle tried nearby is also taken. Stop whatever is using this range, or give the project a different port in its settings.",
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

    const accept =
      conflict.suggestion === null
        ? null
        : button({
            label: `Start on ${conflict.suggestion}`,
            variant: "primary",
            onClick: () => {
              settle(conflict.suggestion);
              closeModal();
            },
          });

    if (accept) actions.push(accept);

    showModal({
      title: "Port already in use",
      body,
      actions,
      width: 440,
      // Covers Escape and the backdrop, neither of which routes through a button.
      onClose: () => settle(null),
    });

    // The dialog focuses its first button by default, which here would be Cancel — so
    // Enter, the key someone reaches for to accept an offer, would decline it instead.
    accept?.focus();
  });
}
