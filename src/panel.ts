/**
 * The tray panel.
 *
 * Shares the store, the API layer, and the glass system with the main window, so a project
 * started here shows as running there without either window asking the other. What differs
 * is only the layout and the density.
 */

import { api } from "./lib/api";
import type { ProjectView } from "./lib/api";
import { h, fill, icon, qs } from "./lib/dom";
import { bytes, percent } from "./lib/format";
import { brandMark, icons } from "./lib/icons";
import { installGlass } from "./lib/glass";
import { connect, get, subscribe, visibleProjects, patch } from "./lib/store";
import { open, toggle } from "./lib/actions";
import { clearToasts } from "./lib/toast";
import { button, iconTile, remoteDot, remoteLabel, sparkline, statusDot, statusLabel } from "./components/common";

const head = qs("#panel-head");
const meters = qs("#panel-meters");
const list = qs("#panel-list");
const foot = qs("#panel-foot");

function renderHead(): void {
  const state = get();
  const running = state.projects.filter((project) => project.status === "running").length;

  fill(
    head,
    h(
      "div",
      { class: "panel__brand" },
      h("span", { html: brandMark(16), style: { display: "flex" } }),
      h("span", null, "Oracle"),
    ),
    h(
      "span",
      { style: { color: "var(--muted)", fontSize: "var(--text-xs)" } },
      running === 0 ? "Nothing running" : `${running} running`,
    ),
  );
}

function renderMeters(): void {
  const { system } = get();
  const memoryPercent = system.memoryTotal
    ? (system.memoryUsed / system.memoryTotal) * 100
    : 0;

  const meter = (label: string, value: string, fillPercent: number) =>
    h(
      "div",
      { class: "meter glass glass--sunken" },
      h(
        "div",
        { class: "meter__row" },
        h("span", { class: "meter__value" }, value),
        h("span", { class: "meter__label" }, label),
      ),
      h(
        "div",
        { class: "meter__bar" },
        h("span", { style: { width: `${Math.min(100, Math.max(0, fillPercent))}%` } }),
      ),
    );

  fill(
    meters,
    meter("CPU", percent(get().system.cpu), system.cpu),
    meter("Memory", bytes(system.memoryUsed), memoryPercent),
  );
}

function renderSearch(): HTMLElement {
  return h(
    "div",
    { class: "panel__search" },
    icon(icons.search),
    h("input", {
      type: "search",
      placeholder: "Search",
      value: get().query,
      "aria-label": "Search projects",
      onInput: (event: Event) => patch({ query: (event.target as HTMLInputElement).value }),
    }),
  );
}

function row(project: ProjectView): HTMLElement {
  const live = project.status === "running" || project.status === "starting";
  const usage = get().usage[project.id];

  const meta: (HTMLElement | string)[] = [];

  if (project.local) {
    meta.push(statusDot(project.status));
    meta.push(
      live && usage
        ? `${percent(usage.current.cpu)} · ${bytes(usage.current.memory)}`
        : statusLabel(project.status),
    );
  } else if (project.remote) {
    meta.push(remoteDot(project.remoteStatus));
    meta.push(remoteLabel(project.remoteStatus));
  }

  return h(
    "button",
    {
      class: "prow",
      type: "button",
      title: project.name,
      // Selecting from the panel opens the full app on that project.
      onClick: () => {
        patch({ selectedId: project.id });
        void api.showMainWindow();
      },
    },
    iconTile(project, 26),
    h(
      "div",
      { class: "prow__body" },
      h("div", { class: "prow__name" }, project.name),
      h("div", { class: "prow__meta" }, ...meta),
    ),
    live && usage ? sparkline(usage.cpuHistory, 40, 16) : h("span"),
    project.local
      ? button({
          iconName: live ? "stop" : "play",
          variant: "ghost",
          title: live ? `Stop ${project.name}` : `Start ${project.name}`,
          onClick: () => void toggle(project),
        })
      : project.resolvedUrl
        ? button({
            iconName: "external",
            variant: "ghost",
            title: `Open ${project.resolvedUrl}`,
            onClick: () => void open(project),
          })
        : h("span"),
  );
}

function renderList(): void {
  const projects = visibleProjects();

  if (get().projects.length === 0) {
    fill(
      list,
      h(
        "div",
        { class: "panel__empty" },
        h("span", { html: brandMark(34), style: { display: "flex", color: "var(--accent)" } }),
        h("p", null, "No projects yet. Open Oracle to add one."),
      ),
    );
    return;
  }

  if (projects.length === 0) {
    fill(list, h("div", { class: "panel__empty" }, h("p", null, "Nothing matches.")));
    return;
  }

  fill(list, ...projects.map(row));
}

function renderFoot(): void {
  fill(
    foot,
    button({
      label: "Open Oracle",
      iconName: "external",
      variant: "primary",
      onClick: () => void api.showMainWindow(),
    }),
    h("div", { style: { flex: "1" } }),
    button({
      iconName: "power",
      variant: "ghost",
      title: "Quit Oracle",
      onClick: () => void api.quit(),
    }),
  );
}

function render(): void {
  renderHead();
  renderMeters();
  renderList();
  renderFoot();
}

async function main(): Promise<void> {
  installGlass();

  // The search field lives between the meters and the list; it is rendered once because it
  // holds focus and re-creating it on every tick would fight the user's typing.
  meters.insertAdjacentElement("afterend", renderSearch());

  subscribe(render);
  render();

  await connect();

  // Toasts belong to the window the user is looking at; a panel that hides should not keep
  // a stack of them waiting for the next time it opens.
  window.addEventListener("blur", clearToasts);

  // Escape dismisses the panel, matching how every other popover on the system behaves.
  document.addEventListener("keydown", (event) => {
    if (event.key === "Escape") void api.hidePanel();
  });
}

void main();
