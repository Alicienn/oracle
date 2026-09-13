/**
 * The project rail.
 *
 * Icons only, with a status badge. Drag to reorder, using the native HTML drag events rather
 * than a pointer-move implementation: the browser already handles the drag image, the
 * autoscroll, and the cancel-on-escape, all of which would otherwise need writing.
 */

import type { ProjectView } from "../lib/api";
import { h, fill, icon } from "../lib/dom";
import { icons } from "../lib/icons";
import { reorder } from "../lib/actions";
import { get, patch } from "../lib/store";
import { iconTile, statusLabel } from "./common";

export function renderRail(
  host: HTMLElement,
  projects: ProjectView[],
  onAdd: () => void,
  onSettings: () => void,
): void {
  let draggedId: string | null = null;

  const items = projects.map((project) => {
    const item = h(
      "button",
      {
        class: "rail__item",
        type: "button",
        draggable: "true",
        title: `${project.name} — ${statusLabel(project.status)}`,
        "aria-label": project.name,
        dataset: {
          selected: String(project.id === get().selectedId),
          id: project.id,
        },
        onClick: () => patch({ selectedId: project.id, tab: "overview" }),
      },
      iconTile(project, 28),
      h("span", {
        class: "rail__badge",
        style: { background: badgeColour(project) },
      }),
    );

    item.addEventListener("dragstart", (event) => {
      draggedId = project.id;
      item.dataset.dragging = "true";
      event.dataTransfer?.setData("text/plain", project.id);
      if (event.dataTransfer) event.dataTransfer.effectAllowed = "move";
    });

    item.addEventListener("dragend", () => {
      draggedId = null;
      delete item.dataset.dragging;
    });

    item.addEventListener("dragover", (event) => {
      if (!draggedId || draggedId === project.id) return;
      event.preventDefault();
      if (event.dataTransfer) event.dataTransfer.dropEffect = "move";
    });

    item.addEventListener("drop", (event) => {
      event.preventDefault();
      if (!draggedId || draggedId === project.id) return;

      const ordered = projects.map((item) => item.id);
      const from = ordered.indexOf(draggedId);
      const to = ordered.indexOf(project.id);
      if (from < 0 || to < 0) return;

      ordered.splice(to, 0, ...ordered.splice(from, 1));
      void reorder(ordered);
    });

    return item;
  });

  fill(
    host,
    ...items,
    h("div", { class: "rail__spacer" }),
    h(
      "button",
      {
        class: "rail__item",
        type: "button",
        title: "Add a project",
        "aria-label": "Add a project",
        onClick: onAdd,
      },
      icon(icons.plus),
    ),
    h(
      "button",
      {
        class: "rail__item",
        type: "button",
        title: "Settings",
        "aria-label": "Settings",
        onClick: onSettings,
      },
      icon(icons.settings),
    ),
  );
}

/**
 * Which signal the rail badge shows.
 *
 * A local status wins when there is one, because that is the thing the user can act on from
 * here; remote health is what a project without a local target has to offer.
 */
function badgeColour(project: ProjectView): string {
  if (project.local) {
    switch (project.status) {
      case "running":
        return "var(--accent)";
      case "starting":
      case "unhealthy":
        return "var(--warn)";
      case "crashed":
        return "var(--danger)";
      default:
        return "var(--idle)";
    }
  }

  switch (project.remoteStatus.state) {
    case "up":
      return "var(--ok)";
    case "degraded":
      return "var(--warn)";
    case "down":
      return "var(--danger)";
    default:
      return "var(--idle)";
  }
}
