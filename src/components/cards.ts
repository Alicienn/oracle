/** The project list. */

import type { ProjectView, Usage } from "../lib/api";
import { h, fill } from "../lib/dom";
import { bytes, percent, shortPath } from "../lib/format";
import { brandMark } from "../lib/icons";
import { open, toggle } from "../lib/actions";
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
            onClick: () => void toggle(project),
          })
        : null,
    ),
  );

  return activatable(node, () => patch({ selectedId: project.id, tab: "overview" }), ".card__actions");
}
