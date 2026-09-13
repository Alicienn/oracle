/** The project list. */

import type { ProjectView, Usage } from "../lib/api";
import { h, fill } from "../lib/dom";
import { bytes, percent, shortPath } from "../lib/format";
import { brandMark } from "../lib/icons";
import { open, reorder, toggle } from "../lib/actions";
import { launch } from "./embedView";
import { get, patch } from "../lib/store";
import {
  activatable,
  button,
  emptyState,
  iconTile,
  isLive,
  pendingLabel,
  remoteDot,
  remoteLabel,
  sparkline,
  statusDot,
} from "./common";

export function renderList(
  host: HTMLElement,
  projects: ProjectView[],
  usage: Record<string, Usage>,
  onAdd: () => void,
  onScan: () => void,
): void {
  host.dataset.view = get().settings.view;
  host.dataset.empty = String(projects.length === 0);

  if (projects.length === 0) {
    const empty = emptyState(
      brandMark(52),
      get().projects.length === 0 ? "No projects yet" : "Nothing matches",
      get().projects.length === 0
        ? "Add a project by hand, or let Oracle scan your folders and suggest what it finds."
        : "Try a different search or filter.",
      get().projects.length === 0
        ? h(
            "div",
            { style: { display: "flex", gap: "8px" } },
            button({ label: "Scan folders", iconName: "scan", onClick: onScan }),
            button({
              label: "Add a project",
              iconName: "plus",
              variant: "primary",
              onClick: onAdd,
            }),
          )
        : undefined,
    );

    // Keyed like a card so the filter transition plays it in rather than snapping it into
    // place while the cards it replaces are still fading out.
    empty.dataset.key = "__empty";
    fill(host, empty);
    return;
  }

  fill(host, ...projects.map((project) => card(project, usage[project.id])));
}

function card(project: ProjectView, usage: Usage | undefined): HTMLElement {
  const selected = project.id === get().selectedId;
  const pending = get().pending[project.id];
  const live = isLive(project.status, pending);

  const meta: (HTMLElement | string)[] = [h("span", { class: "tag" }, project.kindLabel)];

  if (project.local) {
    meta.push(statusDot(project.status));
    if (live && usage) {
      meta.push(`${percent(usage.current.cpu)} · ${bytes(usage.current.memory)}`);
    } else if (project.local.root) {
      meta.push(shortPath(project.local.root));
    }
  }

  if (project.remote) {
    meta.push(remoteDot(project.remoteStatus));
    meta.push(remoteLabel(project.remoteStatus));
  }

  const chart = live && usage ? sparkline(usage.cpuHistory) : null;

  // A div, not a button: the card carries buttons of its own. See `activatable`.
  const node = h(
    "div",
    {
      class: "card glass glass--live",
      "aria-label": project.name,
      draggable: "true",
      // `key` is how the filter transition recognises a card across a rebuild, so it can
      // slide to its new row instead of being destroyed and recreated in place.
      dataset: { selected: String(selected), key: project.id },
    },
    // The accent edge that makes a running project readable without looking at its dot.
    live ? h("span", { class: "card__running", "aria-hidden": "true" }) : null,
    iconTile(project, 34),
    h(
      "div",
      { class: "card__body" },
      h(
        "div",
        { class: "card__name" },
        project.name,
        project.favorite ? h("span", { style: { color: "var(--accent)" } }, "★") : null,
      ),
      h("div", { class: "card__meta" }, ...meta),
    ),
    h(
      "div",
      { class: "card__actions" },
      chart,
      project.resolvedUrl
        ? button({
            iconName: "external",
            variant: "ghost",
            title: `Open ${project.resolvedUrl}`,
            onClick: () => void open(project),
          })
        : null,
      project.local
        ? button({
            iconName: live ? "stop" : "play",
            variant: live ? undefined : "primary",
            title: pending
              ? pendingLabel(pending.kind, project.name)
              : live
                ? `Stop ${project.name}`
                : `Start ${project.name}`,
            pending: pending?.since,
            disabled: pending?.kind === "stop",
            // Stopping is always just stopping; starting a project that serves a page opens
            // it, which is the whole point of the central panel.
            onClick: () => void (live ? toggle(project) : launch(project)),
          })
        : null,
    ),
  );

  draggable(node, project.id);

  return activatable(node, () => patch({ selectedId: project.id, tab: "overview" }), ".card__actions");
}

/**
 * Which card is being dragged, shared across every card in the list.
 *
 * Module scope rather than a closure: the drop handler that needs it belongs to a different
 * card than the one the drag started on.
 */
let dragging: string | null = null;

/**
 * Makes a card a drag handle for reordering.
 *
 * Uses the native drag events, as the rail does: the browser already provides the drag
 * image, the autoscroll and cancel-on-escape, all of which a pointer-move implementation
 * would have to rebuild.
 */
function draggable(node: HTMLElement, id: string): void {
  node.addEventListener("dragstart", (event) => {
    dragging = id;
    node.dataset.dragging = "true";
    event.dataTransfer?.setData("text/plain", id);
    if (event.dataTransfer) event.dataTransfer.effectAllowed = "move";
  });

  node.addEventListener("dragend", () => {
    dragging = null;
    delete node.dataset.dragging;
  });

  node.addEventListener("dragover", (event) => {
    if (!dragging || dragging === id) return;
    event.preventDefault();
    if (event.dataTransfer) event.dataTransfer.dropEffect = "move";
  });

  node.addEventListener("drop", (event) => {
    event.preventDefault();
    if (!dragging || dragging === id) return;

    moveBefore(dragging, id);
  });
}

/**
 * Moves one project to another's position, across the whole project list.
 *
 * Deliberately not computed from what is on screen. A filter hides projects, and a search
 * hides more; reordering only the visible ones would renumber them from zero and drag every
 * hidden project along behind them. So the move is applied to the full list in its stored
 * order, and the filtered view is only how the user pointed at the two ends of it.
 */
function moveBefore(id: string, targetId: string): void {
  const ordered = [...get().projects]
    .sort((a, b) => a.order - b.order)
    .map((project) => project.id);

  const from = ordered.indexOf(id);
  const to = ordered.indexOf(targetId);
  if (from < 0 || to < 0) return;

  ordered.splice(to, 0, ...ordered.splice(from, 1));
  void reorder(ordered);
}
