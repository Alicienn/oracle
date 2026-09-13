/**
 * The main window.
 *
 * One subscriber re-renders each region from the current state. Regions are cheap to rebuild
 * — the largest is a few dozen nodes — so this stays well inside a frame without any
 * diffing machinery.
 */

import { getCurrentWindow } from "@tauri-apps/api/window";
import type { ViewMode } from "./lib/api";
import { h, fill, icon, qs, debounce } from "./lib/dom";
import { brandMark, icons } from "./lib/icons";
import { installGlass, probePerformance } from "./lib/glass";
import {
  connect,
  get,
  patch,
  saveSettings,
  selected,
  subscribe,
  visibleProjects,
  type Filter,
} from "./lib/store";
import { startAll, stopAll } from "./lib/actions";
import { button, segmented } from "./components/common";
import { renderRail } from "./components/rail";
import { renderList } from "./components/cards";
import { renderDetail } from "./components/detail";
import { showProjectForm } from "./components/projectForm";
import { showSettings } from "./components/settings";
import { showScan } from "./components/scan";
import { isModalOpen } from "./components/modal";

const appWindow = getCurrentWindow();

const brand = qs("#brand");
const controls = qs("#window-controls");
const shell = qs("#shell");
const rail = qs("#rail");
const search = qs("#search");
const toolbarControls = qs("#toolbar-controls");
const list = qs("#list");
const detail = qs("#detail");

function renderChrome(): void {
  fill(
    brand,
    h("span", { html: brandMark(17), style: { display: "flex", color: "var(--accent)" } }),
    h("span", null, "Oracle"),
  );

  fill(
    controls,
    h(
      "button",
      { type: "button", "aria-label": "Minimise", onClick: () => void appWindow.minimize() },
      icon(icons.minimise),
    ),
    h(
      "button",
      {
        type: "button",
        "aria-label": "Maximise",
        onClick: () => void appWindow.toggleMaximize(),
      },
      icon(icons.maximise),
    ),
    h(
      "button",
      {
        type: "button",
        "aria-label": "Close",
        dataset: { action: "close" },
        // The window manager decides whether this hides to tray or exits, based on the
        // `minimiseToTray` setting handled in Rust.
        onClick: () => void appWindow.close(),
      },
      icon(icons.close),
    ),
  );
}

const onSearch = debounce((value: string) => patch({ query: value }), 120);

/**
 * Built once, on purpose.
 *
 * The search field holds focus and a caret position. Rebuilding it on every state change
 * would drop both mid-keystroke, and the store updates once a second from the metrics tick.
 */
function renderSearch(): void {
  fill(
    search,
    icon(icons.search),
    h("input", {
      type: "search",
      placeholder: "Search projects",
      "aria-label": "Search projects",
      onInput: (event: Event) => onSearch((event.target as HTMLInputElement).value),
    }),
  );
}

function renderToolbar(): void {
  const state = get();
  const anyRunning = state.projects.some(
    (project) => project.status === "running" || project.status === "starting",
  );

  fill(
    toolbarControls,
    segmented<Filter>(
      [
        { value: "all", label: "All" },
        { value: "local", label: "Local" },
        { value: "remote", label: "Remote" },
        { value: "running", label: "Running" },
      ],
      state.filter,
      (filter) => patch({ filter }),
    ),
    segmented<ViewMode>(
      [
        { value: "list", label: "", iconName: "list", title: "List view" },
        { value: "grid", label: "", iconName: "grid", title: "Grid view" },
      ],
      state.settings.view,
      (view) => void saveSettings({ view }),
    ),
    button({
      iconName: anyRunning ? "stop" : "play",
      title: anyRunning ? "Stop everything" : "Start every local project",
      onClick: () => void (anyRunning ? stopAll() : startAll()),
    }),
    button({ iconName: "scan", title: "Find projects", onClick: showScan }),
    button({
      iconName: "plus",
      variant: "primary",
      title: "Add a project",
      onClick: () => showProjectForm(),
    }),
  );
}

function render(): void {
  const state = get();
  const current = selected();

  shell.dataset.detail = current ? "open" : "closed";

  renderToolbar();
  renderRail(rail, [...state.projects].sort((a, b) => a.order - b.order), () => showProjectForm(), showSettings);
  renderList(list, visibleProjects(), state.usage, () => showProjectForm(), showScan);
  renderDetail(detail, current);
}

function bindShortcuts(): void {
  document.addEventListener("keydown", (event) => {
    if (isModalOpen()) return;

    const typing =
      event.target instanceof HTMLElement &&
      ["INPUT", "TEXTAREA", "SELECT"].includes(event.target.tagName);

    // Ctrl+F and "/" both focus search, the two conventions users arrive with.
    if ((event.ctrlKey && event.key === "f") || (!typing && event.key === "/")) {
      event.preventDefault();
      qs<HTMLInputElement>("#search input").focus();
      return;
    }

    if (event.ctrlKey && event.key === "n") {
      event.preventDefault();
      showProjectForm();
      return;
    }

    if (event.ctrlKey && event.key === ",") {
      event.preventDefault();
      showSettings();
      return;
    }

    if (event.key === "Escape" && !typing && get().selectedId) {
      patch({ selectedId: null });
    }
  });
}

async function main(): Promise<void> {
  installGlass();
  renderChrome();
  renderSearch();
  bindShortcuts();

  subscribe(render);
  render();

  await connect();

  // Measured after the first real render, so the number reflects the actual UI rather than
  // an empty page.
  await probePerformance(get().settings.glass);
}

void main();
