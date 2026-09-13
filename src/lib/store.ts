/**
 * The application store.
 *
 * A plain object behind a subscriber list. Mutations go through `patch`, which notifies
 * subscribers once, on the next frame — so ten status events arriving together cause one
 * render rather than ten.
 */

import type {
  GitStatus,
  LogLine,
  ProjectStatus,
  ProjectView,
  RemoteReport,
  Settings,
  SystemUsage,
  Usage,
} from "./api";
import { api, events, toApiError } from "./api";
import { setGlassLevel, setTheme } from "./glass";
import { toast } from "./toast";

export type Filter = "all" | "local" | "remote" | "running";
export type Tab = "overview" | "logs" | "metrics" | "git";

/** An operation the user asked for that the backend has not yet answered. */
export type Pending = "start" | "stop";

/**
 * A pending operation and when it began.
 *
 * The timestamp is not decoration. Every region is rebuilt from scratch on each state
 * change — including the metrics tick, once a second — so a spinner node only ever lives
 * for a frame or two. Its CSS animation would restart with each new node and visibly
 * stutter; knowing when the operation started lets the spinner offset its animation and
 * appear continuous across every rebuild.
 */
export interface PendingOp {
  kind: Pending;
  /** `performance.now()` at the moment the request was made. */
  since: number;
}

/**
 * How long a pending operation is allowed to sit unanswered.
 *
 * The backend resolves a start within `READY_TIMEOUT` (60s) and a stop within
 * `GRACEFUL_STOP` (5s), so reaching this guard means an event was lost rather than slow. A
 * spinner that never stops is worse than one that gives up.
 */
const PENDING_GUARD_MS = 90_000;

/**
 * Log lines held per project, and how many projects keep a buffer at all.
 *
 * The backend's ring is the archive — `api.logs` re-reads it whenever a project is selected
 * — so the frontend only needs enough to render the pane and follow the tail. Keeping 1000
 * lines for every project ever opened, for the life of the window, cost tens of megabytes
 * of strings that nothing was going to read again.
 */
const LOG_LIMIT = 500;
const LOG_PROJECTS = 3;

export interface State {
  projects: ProjectView[];
  settings: Settings;
  system: SystemUsage;
  /** Latest sample per project id. */
  usage: Record<string, Usage>;
  /** Buffered log lines per project id. */
  logs: Record<string, LogLine[]>;
  git: Record<string, GitStatus | null>;
  /**
   * In-flight start/stop requests, per project id.
   *
   * Owned by the UI rather than derived from `status`: the backend's `starting` covers a
   * project that is genuinely booting, which is not the same question as whether the button
   * the user just pressed has been answered.
   */
  pending: Record<string, PendingOp>;
  selectedId: string | null;
  tab: Tab;
  filter: Filter;
  query: string;
  ready: boolean;
  version: string;
  gitAvailable: boolean;
}

type Listener = (state: State) => void;

const initial: State = {
  projects: [],
  settings: {
    startWithWindows: false,
    startHidden: false,
    autostartProjects: true,
    theme: "system",
    glass: "full",
    healthIntervalSecs: 30,
    scanRoots: [],
    panelShortcut: "CmdOrCtrl+Shift+Space",
    minimiseToTray: true,
    view: "list",
    checkUpdates: true,
  },
  system: { cpu: 0, memoryUsed: 0, memoryTotal: 0 },
  usage: {},
  logs: {},
  git: {},
  pending: {},
  selectedId: null,
  tab: "overview",
  filter: "all",
  query: "",
  ready: false,
  version: "",
  gitAvailable: false,
};

let state: State = initial;
const listeners = new Set<Listener>();
let scheduled = false;

export function get(): State {
  return state;
}

export function subscribe(listener: Listener): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

/** Merges a partial state and schedules one notification for the next frame. */
export function patch(changes: Partial<State>): void {
  state = { ...state, ...changes };

  // Log eviction hangs off selection because selection is what decides which buffers are
  // still worth holding. Doing it here rather than in each component means no caller can
  // change the selection and skip it.
  if ("selectedId" in changes) state = evictStaleLogs(state);

  if (scheduled) return;
  scheduled = true;

  requestAnimationFrame(() => {
    scheduled = false;
    for (const listener of listeners) listener(state);
  });
}

/**
 * The projects whose logs are worth keeping, most recently selected first.
 *
 * Held outside the state because it is bookkeeping, not something any component renders.
 */
const recentlyViewed: string[] = [];

/** Drops log buffers for every project outside the recently-viewed window. */
function evictStaleLogs(next: State): State {
  const current = next.selectedId;

  if (current) {
    const existing = recentlyViewed.indexOf(current);
    if (existing !== -1) recentlyViewed.splice(existing, 1);
    recentlyViewed.unshift(current);
    recentlyViewed.length = Math.min(recentlyViewed.length, LOG_PROJECTS);
  }

  const stale = Object.keys(next.logs).filter((id) => !recentlyViewed.includes(id));
  if (stale.length === 0) return next;

  const logs = { ...next.logs };
  for (const id of stale) delete logs[id];

  return { ...next, logs };
}

/** Replaces one project in place, leaving the rest of the array identity alone. */
export function patchProject(id: string, changes: Partial<ProjectView>): void {
  patch({
    projects: state.projects.map((project) =>
      project.id === id ? { ...project, ...changes } : project,
    ),
  });
}

/**
 * Marks an operation as in flight and arms the guard that clears it.
 *
 * The timer is cancelled by `clearPending`, so a normal completion never leaves one
 * pending: the only path that reaches the guard is a status event that never arrived.
 */
export function setPending(projectId: string, kind: Pending): void {
  clearGuard(projectId);
  guards.set(
    projectId,
    window.setTimeout(() => {
      guards.delete(projectId);
      clearPending(projectId);
    }, PENDING_GUARD_MS),
  );

  patch({
    pending: { ...state.pending, [projectId]: { kind, since: performance.now() } },
  });
}

export function clearPending(projectId: string): void {
  clearGuard(projectId);
  if (!(projectId in state.pending)) return;

  const next = { ...state.pending };
  delete next[projectId];
  patch({ pending: next });
}

export function pendingFor(projectId: string): PendingOp | undefined {
  return state.pending[projectId];
}

const guards = new Map<string, number>();

function clearGuard(projectId: string): void {
  const timer = guards.get(projectId);
  if (timer !== undefined) {
    clearTimeout(timer);
    guards.delete(projectId);
  }
}

export function selected(): ProjectView | null {
  return state.projects.find((project) => project.id === state.selectedId) ?? null;
}

/** The projects the current filter and query admit, in display order. */
export function visibleProjects(): ProjectView[] {
  const query = state.query.trim().toLowerCase();

  return state.projects
    .filter((project) => {
      switch (state.filter) {
        case "local":
          if (!project.local) return false;
          break;
        case "remote":
          if (!project.remote) return false;
          break;
        case "running":
          if (project.status !== "running" && project.status !== "starting") return false;
          break;
      }

      if (!query) return true;

      return (
        project.name.toLowerCase().includes(query) ||
        project.kindLabel.toLowerCase().includes(query) ||
        project.tags.some((tag) => tag.toLowerCase().includes(query)) ||
        (project.local?.root ?? "").toLowerCase().includes(query)
      );
    })
    .sort((a, b) => {
      // Favourites float, then the stored order.
      if (a.favorite !== b.favorite) return a.favorite ? -1 : 1;
      return a.order - b.order;
    });
}

// ---------------------------------------------------------------------------
// Wiring
// ---------------------------------------------------------------------------

/** Loads the initial snapshot and subscribes to the backend's event stream. */
export async function connect(): Promise<void> {
  try {
    const snapshot = await api.snapshot();

    setTheme(snapshot.settings.theme);
    setGlassLevel(snapshot.settings.glass);

    patch({
      projects: snapshot.projects,
      settings: snapshot.settings,
      system: snapshot.system,
      version: snapshot.version,
      gitAvailable: snapshot.gitAvailable,
      ready: true,
    });

    for (const warning of snapshot.warnings) {
      toast({ tone: "error", title: "Configuration", message: warning, sticky: true });
    }
  } catch (error) {
    const failure = toApiError(error);
    toast({
      tone: "error",
      title: "Oracle could not start cleanly",
      message: failure.message,
      detail: failure.detail,
      sticky: true,
    });
    patch({ ready: true });
  }

  await events.metrics((tick) => {
    const usage: Record<string, Usage> = {};
    for (const item of tick.projects) usage[item.projectId] = item;
    patch({ usage, system: tick.system });
  });

  await events.remote((reports: RemoteReport[]) => {
    const byId = new Map(reports.map((report) => [report.projectId, report.status]));
    patch({
      projects: state.projects.map((project) => {
        const next = byId.get(project.id);
        return next ? { ...project, remoteStatus: next } : project;
      }),
    });
  });

  // Icons arrive after startup, from hosts that may be slow; re-reading the list is what
  // swaps initials for the real thing.
  await events.icons(() => void refreshProjects());

  await events.status((projectId: string, status: ProjectStatus) => {
    patchProject(projectId, { status });

    // Every status the backend emits is an answer to whatever was asked of the project, so
    // any one of them ends the wait. `starting` is the exception: it is the backend echoing
    // the request, not resolving it.
    if (status !== "starting") clearPending(projectId);
  });

  await events.log((projectId: string, line: LogLine) => {
    // A project nobody has looked at recently gets no buffer. Its output is not lost — the
    // backend ring holds it, and `loadLogs` reads the whole thing back the moment the
    // project is selected — so buffering it here would only be paying to duplicate it.
    if (!recentlyViewed.includes(projectId)) return;

    const existing = state.logs[projectId] ?? [];
    const next =
      existing.length >= LOG_LIMIT ? [...existing.slice(1), line] : [...existing, line];
    patch({ logs: { ...state.logs, [projectId]: next } });
  });
}

/** Pulls the whole project list again, after a change the events do not cover. */
export async function refreshProjects(): Promise<void> {
  try {
    const snapshot = await api.snapshot();
    patch({ projects: snapshot.projects, settings: snapshot.settings });
  } catch (error) {
    reportError("Could not refresh the project list", error);
  }
}

export async function loadLogs(projectId: string): Promise<void> {
  try {
    const lines = await api.logs(projectId);
    patch({ logs: { ...state.logs, [projectId]: lines } });
  } catch (error) {
    reportError("Could not read the logs", error);
  }
}

export async function loadGit(projectId: string): Promise<void> {
  try {
    const status = await api.gitStatus(projectId);
    patch({ git: { ...state.git, [projectId]: status } });
  } catch (error) {
    // A project without git is a normal state, not something worth a toast.
    patch({ git: { ...state.git, [projectId]: null } });
    void error;
  }
}

export async function saveSettings(changes: Partial<Settings>): Promise<void> {
  const next = { ...state.settings, ...changes };

  // Apply the visual settings immediately: waiting for the round trip would make the theme
  // toggle feel broken.
  if (changes.theme) setTheme(changes.theme);
  if (changes.glass) setGlassLevel(changes.glass);

  patch({ settings: next });

  try {
    const saved = await api.updateSettings(next);
    patch({ settings: saved });
  } catch (error) {
    reportError("Could not save the settings", error);
    // Put back what the backend actually holds.
    await refreshProjects();
  }
}

/** Turns any thrown value into a toast, without ever throwing again. */
export function reportError(title: string, error: unknown): void {
  const failure = toApiError(error);
  toast({
    tone: "error",
    title,
    message: failure.message,
    detail: failure.detail,
  });
}
