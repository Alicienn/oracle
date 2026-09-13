/**
 * A project's web app, shown in the central panel.
 *
 * The webview itself is native and knows nothing about this layout: it is positioned from
 * Rust, in the coordinates this module reports. Two jobs follow from that.
 *
 * The first is keeping it aligned. The panel changes shape when the window resizes, when
 * the rail expands, when the detail pane opens — so the host element is observed rather
 * than measured once, and every change is sent on.
 *
 * The second is the launch sequence. A project that has just been asked to start is not
 * ready to be looked at: its port accepting a connection means the server is listening, not
 * that it has finished compiling. So the panel shows the project booting, waits a beat past
 * the moment it reports ready, and only then reveals the page. The wait is what stops the
 * user landing on a blank screen and concluding it is broken.
 */

import type { ProjectView } from "../lib/api";
import { api } from "../lib/api";
import { h, fill } from "../lib/dom";
import { bytes } from "../lib/format";
import { open as openExternally, start } from "../lib/actions";
import { get, patch, reportError, subscribe } from "../lib/store";
import { button, iconTile, isLive } from "./common";
import { progressRing } from "./progressRing";

/** How long the tick is left on screen before the page takes over. */
const SETTLE_MS = 1400;

/** The host the webview is positioned over, watched for every change of shape. */
let host: HTMLElement | null = null;
let observer: ResizeObserver | null = null;
let unsubscribe: (() => void) | null = null;
let settleTimer: number | undefined;

/**
 * What the play button does for a project that serves a page.
 *
 * Starting a web project and then not showing it is a step nobody wanted; a project with no
 * address has nothing to show, and keeps the plain start.
 */
export function launch(project: ProjectView): Promise<void> {
  return project.resolvedUrl ? openWebApp(project) : start(project);
}

/**
 * Opens a project's web app, starting the project first if it is not up.
 *
 * Only meaningful for a project with an address; the caller checks that, because the button
 * that offers this should not exist otherwise.
 */
export async function openWebApp(project: ProjectView): Promise<void> {
  // Nothing to start, and nothing to wait for: a project that only exists as a URL is
  // served by someone else's machine. Asking to start it would earn "no local setup".
  const remoteOnly = !project.local;

  // `running` is not the only state the runner already holds a project in: a project it
  // adopted at startup, or one still booting, is live too, and asking it to start again
  // earns an "already running" error and a toast nobody asked for.
  const held = remoteOnly || isLive(project.status, get().pending[project.id]);

  // A project whose port never answered is alive and unreachable. Opening its address would
  // hand the user WebView2's own "cannot reach this page", which says nothing about why —
  // so the panel says it instead, and names the usual cause.
  if (project.status === "unhealthy") {
    patch({ embed: { projectId: project.id, phase: "unreachable" }, selectedId: null });
    return;
  }

  patch({
    embed: {
      projectId: project.id,
      phase: remoteOnly || project.status === "running" ? "open" : "launching",
    },
    selectedId: null,
  });

  if (held) return;

  await start(project);

  // A start that was refused leaves nothing to wait for, so the panel says so rather than
  // showing a ring for ever. `start` has already reported why.
  const after = get().projects.find((item) => item.id === project.id);
  if (after && !isLive(after.status, get().pending[project.id])) {
    patch({ embed: { projectId: project.id, phase: "failed" } });
  }
}

/** Leaves the web app on screen behind, keeping it and its page state alive. */
export async function leaveWebApp(): Promise<void> {
  patch({ embed: null, embedMemory: null });

  try {
    await api.hideEmbed();
  } catch (error) {
    reportError("Could not put the web app away", error);
  }
}

/** Discards a project's webview entirely, releasing the memory it held. */
export async function discardWebApp(projectId: string): Promise<void> {
  patch({ embed: null, embedMemory: null });

  try {
    await api.closeEmbed(projectId);
  } catch (error) {
    reportError("Could not close the web app", error);
  }
}

/**
 * Renders the panel for the current embed state.
 *
 * Called from the render loop, but only rebuilds when the phase or the project changes:
 * the host element must survive a metrics tick, or the observer watching it — and the
 * webview positioned over it — would be thrown away once a second.
 */
export function renderEmbed(container: HTMLElement): void {
  const state = get();
  const embed = state.embed;

  if (!embed) {
    teardown();
    fill(container);
    return;
  }

  const project = state.projects.find((item) => item.id === embed.projectId);
  if (!project) {
    void leaveWebApp();
    return;
  }

  const signature = `${embed.projectId}:${embed.phase}`;
  if (container.dataset.signature === signature) {
    // Same state: only the figures under it can have changed.
    refreshMemory(container);
    return;
  }
  container.dataset.signature = signature;

  fill(container, header(project), body(project, embed.phase));

  if (embed.phase === "open") {
    // The host exists only in this phase, so the observer is attached here.
    watch(container, project);
  } else {
    unwatch();
  }
}

// ---------------------------------------------------------------------------
// Chrome
// ---------------------------------------------------------------------------

function header(project: ProjectView): HTMLElement {
  const memory = h("span", { class: "webapp__memory" });

  return h(
    "div",
    { class: "webapp__bar" },
    button({
      iconName: "chevron",
      variant: "ghost",
      title: "Back to projects",
      onClick: () => void leaveWebApp(),
    }),
    iconTile(project, 20),
    h("span", { class: "webapp__name" }, project.name),
    h("span", { class: "webapp__url" }, project.resolvedUrl ?? ""),
    h("div", { style: { flex: "1" } }),
    memory,
    button({
      iconName: "restart",
      variant: "ghost",
      title: "Reload the page",
      onClick: () => void api.reloadEmbed(),
    }),
    button({
      iconName: "external",
      variant: "ghost",
      title: "Open in the browser",
      onClick: () => void openExternally(project),
    }),
    button({
      iconName: "close",
      variant: "ghost",
      // Named for what it costs, not just what it does: this is the way to get the
      // renderer's memory back, and it is why the figure beside it is shown at all.
      title: "Close this view and free its memory",
      onClick: () => void discardWebApp(project.id),
    }),
  );
}

function body(project: ProjectView, phase: string): HTMLElement {
  if (phase === "open") {
    // Deliberately empty: the native webview is placed over this element, and anything
    // drawn here would be hidden behind it.
    return h("div", { class: "webapp__host", id: "webapp-host" });
  }

  if (phase === "failed" || phase === "unreachable") {
    const unreachable = phase === "unreachable";

    return h(
      "div",
      { class: "webapp__stage" },
      iconTile(project, 56),
      h(
        "strong",
        null,
        unreachable
          ? `${project.name} is running but not answering`
          : `${project.name} did not come up`,
      ),
      h(
        "p",
        null,
        unreachable
          ? `Nothing is listening on ${project.resolvedUrl}. The usual cause is the project serving a different port than the one in its settings.`
          : "Its logs say why.",
      ),
      h(
        "div",
        { class: "row" },
        button({
          label: "Project settings",
          onClick: () => {
            void leaveWebApp();
            patch({ selectedId: project.id, tab: "overview" });
          },
        }),
        button({
          label: "Logs",
          onClick: () => {
            void leaveWebApp();
            patch({ selectedId: project.id, tab: "logs" });
          },
        }),
        button({
          label: "Back to projects",
          variant: "ghost",
          onClick: () => void leaveWebApp(),
        }),
      ),
    );
  }

  const ring = progressRing(44);
  const stage = h(
    "div",
    { class: "webapp__stage" },
    iconTile(project, 56),
    h("strong", null, project.name),
    ring.element,
    h(
      "p",
      null,
      phase === "ready" ? "Ready — opening the page…" : "Starting…",
    ),
  );

  if (phase === "ready") {
    // Attached first, then completed: re-parenting an element restarts its animations.
    requestAnimationFrame(() => ring.succeed());
  }

  return stage;
}

function refreshMemory(container: HTMLElement): void {
  const node = container.querySelector<HTMLElement>(".webapp__memory");
  if (!node) return;

  const memory = get().embedMemory;
  node.textContent = memory ? `${bytes(memory)} in this view` : "";
}

// ---------------------------------------------------------------------------
// Keeping the webview where it belongs
// ---------------------------------------------------------------------------

function watch(container: HTMLElement, project: ProjectView): void {
  const element = container.querySelector<HTMLElement>("#webapp-host");
  if (!element || host === element) return;

  host = element;

  const send = () => {
    const box = element.getBoundingClientRect();
    if (box.width < 1 || box.height < 1) return;

    void api
      .setEmbedBounds({ x: box.x, y: box.y, width: box.width, height: box.height })
      .catch(() => {
        // A bounds update that misses is not worth a toast; the next one will land.
      });
  };

  observer?.disconnect();
  observer = new ResizeObserver(send);
  observer.observe(element);

  // The grid transition that opens the panel runs for `--t-panel`, and the observer fires
  // throughout it, so the webview follows the animation rather than jumping at the end.
  const box = element.getBoundingClientRect();
  void api
    .openEmbed(project.id, { x: box.x, y: box.y, width: box.width, height: box.height })
    .catch((error) => {
      reportError(`Could not open ${project.name} here`, error);
      void leaveWebApp();
    });
}

function unwatch(): void {
  observer?.disconnect();
  observer = null;
  host = null;
}

function teardown(): void {
  unwatch();
  if (settleTimer !== undefined) {
    clearTimeout(settleTimer);
    settleTimer = undefined;
  }
}

// ---------------------------------------------------------------------------
// The launch sequence
// ---------------------------------------------------------------------------

/**
 * Advances the phase as the project's status changes.
 *
 * Subscribed once, for the life of the window: the sequence is driven by events that arrive
 * whether or not anything is rendering, and a subscription set up per render would be torn
 * down mid-launch.
 */
export function watchLaunches(): void {
  if (unsubscribe) return;

  unsubscribe = subscribe((state) => {
    const embed = state.embed;
    if (!embed || embed.phase !== "launching") return;

    const project = state.projects.find((item) => item.id === embed.projectId);
    if (!project) return;

    if (project.status === "running") {
      patch({ embed: { ...embed, phase: "ready" } });

      // The beat between "the port answered" and "here is the page".
      clearTimeout(settleTimer);
      settleTimer = window.setTimeout(() => {
        const current = get().embed;
        if (current?.projectId === embed.projectId && current.phase === "ready") {
          patch({ embed: { ...current, phase: "open" } });
        }
      }, SETTLE_MS);
      return;
    }

    if (project.status === "crashed") {
      patch({ embed: { ...embed, phase: "failed" } });
      return;
    }

    // Alive, but its port never opened: a different failure, and a different answer.
    if (project.status === "unhealthy") {
      patch({ embed: { ...embed, phase: "unreachable" } });
    }
  });
}
