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

export interface State {
  projects: ProjectView[];
  settings: Settings;
  system: SystemUsage;
  /** Latest sample per project id. */
  usage: Record<string, Usage>;
  /** Buffered log lines per project id. */
  logs: Record<string, LogLine[]>;
  git: Record<string, GitStatus | null>;
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
  },
  system: { cpu: 0, memoryUsed: 0, memoryTotal: 0 },
  usage: {},
  logs: {},
  git: {},
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

  if (scheduled) return;
  scheduled = true;

  requestAnimationFrame(() => {
    scheduled = false;
    for (const listener of listeners) listener(state);
  });
}

/** Replaces one project in place, leaving the rest of the array identity alone. */
export function patchProject(id: string, changes: Partial<ProjectView>): void {
  patch({
    projects: state.projects.map((project) =>
      project.id === id ? { ...project, ...changes } : project,
    ),
  });
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

  await events.status((projectId: string, status: ProjectStatus) => {
    patchProject(projectId, { status });
  });

  await events.log((projectId: string, line: LogLine) => {
    const existing = state.logs[projectId] ?? [];
    // Mirror the backend's cap so a long-lived session cannot grow without bound.
    const next = existing.length >= 1000 ? [...existing.slice(1), line] : [...existing, line];
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
