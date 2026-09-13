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
import { installGlass, probePerformance, restoreHighlight } from "./lib/glass";
import { fluidList, slidingPill } from "./lib/motion";
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
import {
  button,
  isLive,
  segmented,
  setButtonIcon,
  setSegmentedValue,
} from "./components/common";
import { renderRail } from "./components/rail";
import { renderList } from "./components/cards";
import { renderDetail } from "./components/detail";
import { showProjectForm } from "./components/projectForm";
import { showSettings } from "./components/settings";
import { showScan } from "./components/scan";
import { isModalOpen } from "./components/modal";
import { showUpdatePrompt } from "./components/updatePrompt";
import { findUpdate } from "./lib/updates";

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

/**
 * The toolbar, built once.
 *
 * It used to be rebuilt on every state change, which is once a second from the metrics
 * tick. That cost nothing visible but it also made the selection highlight impossible to
 * animate: a control replaced between renders has no previous position to travel from.
 * Everything that varies is now updated in place by `syncToolbar`.
 */
let filterControl: HTMLElement;
let viewControl: HTMLElement;
let powerButton: HTMLButtonElement;

function buildToolbar(): void {
  const state = get();

  filterControl = segmented<Filter>(
    [
      { value: "all", label: "All" },
      { value: "local", label: "Local" },
      { value: "remote", label: "Remote" },
      { value: "running", label: "Running" },
    ],
    state.filter,
    (filter) => patch({ filter }),
    true,
  );

  viewControl = segmented<ViewMode>(
    [
      { value: "list", label: "", iconName: "list", title: "List view" },
      { value: "grid", label: "", iconName: "grid", title: "Grid view" },
    ],
    state.settings.view,
    (view) => void saveSettings({ view }),
    true,
  );

  powerButton = button({
    iconName: "play",
    title: "Start every local project",
    onClick: () => void (anyRunning() ? stopAll() : startAll()),
  });

  fill(
    toolbarControls,
    filterControl,
    viewControl,
    powerButton,
    button({ iconName: "scan", title: "Find projects", onClick: showScan }),
    button({
      iconName: "plus",
      variant: "primary",
      title: "Add a project",
      onClick: () => showProjectForm(),
    }),
  );
}

function anyRunning(): boolean {
  const state = get();
  return state.projects.some((project) => isLive(project.status, state.pending[project.id]));
}

function syncToolbar(): void {
  const state = get();

  setSegmentedValue(filterControl, state.filter);
  setSegmentedValue(viewControl, state.settings.view);
  slidingPill(filterControl);
  slidingPill(viewControl);

  const live = anyRunning();
  setButtonIcon(
    powerButton,
    live ? "stop" : "play",
    live ? "Stop everything" : "Start every local project",
  );
}

/**
 * What the list looked like the last time it was rendered.
 *
 * The transition has to be driven by an explicit change, not inferred from the render: the
 * metrics tick re-renders once a second, and animating the list every time would be both
 * wrong and a permanent cost. Only a new filter or a new query is a reason to move.
 */
let shown = { filter: "" as string, query: "" as string };

function render(): void {
  const state = get();
  const current = selected();

  shell.dataset.detail = current ? "open" : "closed";

  syncToolbar();
  renderRail(rail, [...state.projects].sort((a, b) => a.order - b.order), () => showProjectForm(), showSettings);

  const changed = state.filter !== shown.filter || state.query.trim() !== shown.query;
  shown = { filter: state.filter, query: state.query.trim() };

  const paint = () =>
    renderList(list, visibleProjects(), state.usage, () => showProjectForm(), showScan);

  if (changed) {
    fluidList(list, paint);
  } else {
    paint();
  }

  renderDetail(detail, current);

  // Regions above were rebuilt; the pointer highlight has to be put back on whatever now
  // sits under the cursor.
  restoreHighlight();
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
  buildToolbar();
  bindShortcuts();

  subscribe(render);
  render();

  await connect();

  // Measured after the first real render, so the number reflects the actual UI rather than
  // an empty page.
  await probePerformance(get().settings.glass);

  void offerUpdate();
}

void main();

/**
 * Looks for a new release once, at launch.
 *
 * After the first render and the performance probe, so a slow or unreachable release feed
 * cannot delay the window appearing. Silent when the app is current or the check fails —
 * Settings is where someone who wants an answer goes to ask.
 */
async function offerUpdate(): Promise<void> {
  if (!get().settings.checkUpdates) return;

  const update = await findUpdate();
  if (update) showUpdatePrompt(update, get().version);
}
