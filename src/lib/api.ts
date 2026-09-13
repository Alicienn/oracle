/**
 * The typed edge of the IPC boundary.
 *
 * Every type here mirrors a Rust type in `src-tauri/src`. Keeping them hand-written rather
 * than generated is a deliberate trade: the surface is small enough to maintain by hand, and
 * a mismatch shows up immediately as a type error at the call site.
 */

import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

// ---------------------------------------------------------------------------
// Model
// ---------------------------------------------------------------------------

export type ProjectKind =
  | "nextJs"
  | "node"
  | "rust"
  | "python"
  | "docker"
  | "static"
  | "other";

export type IconSource =
  | { type: "auto" }
  | { type: "builtin"; value: string }
  | { type: "file"; value: string }
  | { type: "favicon"; value: string };

export interface LocalTarget {
  root: string;
  command: string;
  env: Record<string, string>;
  port?: number | null;
  openUrl?: string | null;
  autostartWithOracle: boolean;
}

export interface RemoteTarget {
  url: string;
  check: { type: "http"; method: "GET" | "HEAD"; expectStatus: number[] };
  intervalSecs: number;
}

export interface RepoRef {
  provider: "gitHub" | "gitLab" | "other";
  slug?: string | null;
  remoteUrl?: string | null;
}

export interface Project {
  id: string;
  name: string;
  kind: ProjectKind;
  icon: IconSource;
  accent?: string | null;
  local?: LocalTarget | null;
  remote?: RemoteTarget | null;
  repo?: RepoRef | null;
  tags: string[];
  favorite: boolean;
  order: number;
  notes?: string | null;
}

export type ProjectStatus =
  | "stopped"
  | "starting"
  | "running"
  | "unhealthy"
  | "crashed";

export type RemoteStatus =
  | { state: "up"; ms: number; code: number }
  | { state: "degraded"; ms: number; code: number }
  | { state: "down"; reason: string }
  | { state: "unchecked" };

/** A project plus everything the UI needs, resolved by the backend. */
export interface ProjectView extends Project {
  status: ProjectStatus;
  remoteStatus: RemoteStatus;
  pid?: number | null;
  resolvedAccent: string;
  resolvedUrl?: string | null;
  kindLabel: string;
}

export type Theme = "light" | "dark" | "system";
export type GlassLevel = "full" | "reduced" | "opaque";
export type ViewMode = "list" | "grid";

export interface Settings {
  startWithWindows: boolean;
  startHidden: boolean;
  autostartProjects: boolean;
  theme: Theme;
  glass: GlassLevel;
  healthIntervalSecs: number;
  scanRoots: string[];
  panelShortcut: string;
  minimiseToTray: boolean;
  view: ViewMode;
}

export interface SystemUsage {
  cpu: number;
  memoryUsed: number;
  memoryTotal: number;
}

export interface Snapshot {
  projects: ProjectView[];
  settings: Settings;
  system: SystemUsage;
  warnings: string[];
  gitAvailable: boolean;
  version: string;
}

export interface Sample {
  cpu: number;
  memory: number;
  processes: number;
}

export interface Usage {
  projectId: string;
  current: Sample;
  cpuHistory: number[];
  memoryHistory: number[];
}

export interface MetricsTick {
  projects: Usage[];
  system: SystemUsage;
}

export interface RemoteReport {
  projectId: string;
  status: RemoteStatus;
  checkedAt: number;
}

export interface LogLine {
  seq: number;
  stream: "stdout" | "stderr" | "system";
  text: string;
  at: number;
}

export interface Candidate {
  name: string;
  root: string;
  kind: ProjectKind;
  suggestedCommand: string;
  suggestedPort?: number | null;
  repo?: RepoRef | null;
  alreadyKnown: boolean;
}

export interface Commit {
  hash: string;
  subject: string;
  author: string;
  at: number;
}

export interface GitStatus {
  branch?: string | null;
  upstream?: string | null;
  ahead: number;
  behind: number;
  dirtyFiles: number;
  lastCommit?: Commit | null;
}

/** The shape an `OracleError` takes once it crosses the boundary. */
export interface ApiError {
  code: string;
  message: string;
  detail?: string | null;
  /** Present only on the errors the UI can act on, such as a port conflict. */
  data?: unknown;
}

/** The payload carried by a `port_in_use` error. */
export interface PortConflict {
  port: number;
  /** Another Oracle project by name, or a description of the foreign process. */
  holder: string;
  /** The next free port, or null when the search found none. */
  suggestion: number | null;
}

/**
 * Reads the conflict payload off an error, if that is what it is.
 *
 * The message is written for a human; the parts have to arrive separately for the UI to
 * offer a port rather than only report a collision.
 */
export function portConflict(error: unknown): PortConflict | null {
  if (!isApiError(error) || error.code !== "port_in_use") return null;

  const data = error.data;
  if (typeof data !== "object" || data === null) return null;

  const { port, holder, suggestion } = data as Record<string, unknown>;
  if (typeof port !== "number" || typeof holder !== "string") return null;

  return {
    port,
    holder,
    suggestion: typeof suggestion === "number" ? suggestion : null,
  };
}

/** Type guard, because a rejected invoke can also throw a plain string. */
export function isApiError(value: unknown): value is ApiError {
  return (
    typeof value === "object" &&
    value !== null &&
    "code" in value &&
    "message" in value
  );
}

/** Turns anything thrown by `invoke` into something safe to display. */
export function toApiError(value: unknown): ApiError {
  if (isApiError(value)) return value;
  return { code: "unknown", message: String(value) };
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

export const api = {
  snapshot: () => invoke<Snapshot>("get_snapshot"),

  logs: (projectId: string, since?: number) =>
    invoke<LogLine[]>("get_logs", { projectId, since }),
  clearLogs: (projectId: string) => invoke<void>("clear_logs", { projectId }),
  gitStatus: (projectId: string) =>
    invoke<GitStatus | null>("git_status", { projectId }),

  /** `port` overrides the project's declared port for this run only. */
  start: (projectId: string, port?: number) =>
    invoke<number>("start_project", { projectId, port }),
  stop: (projectId: string) => invoke<void>("stop_project", { projectId }),
  restart: (projectId: string, port?: number) =>
    invoke<number>("restart_project", { projectId, port }),

  addProject: (project: Project) => invoke<ProjectView>("add_project", { project }),
  updateProject: (project: Project) =>
    invoke<ProjectView>("update_project", { project }),
  deleteProject: (projectId: string) =>
    invoke<void>("delete_project", { projectId }),
  reorder: (orderedIds: string[]) => invoke<void>("reorder_projects", { orderedIds }),
  toggleFavorite: (projectId: string) =>
    invoke<boolean>("toggle_favorite", { projectId }),

  scan: () => invoke<Candidate[]>("scan_projects"),
  importCandidates: (candidates: Candidate[]) =>
    invoke<ProjectView[]>("import_candidates", { candidates }),

  updateSettings: (settings: Settings) =>
    invoke<Settings>("update_settings", { settings }),
  autostartState: () => invoke<boolean>("get_autostart_state"),
  importIcon: (source: string) => invoke<string>("import_icon", { source }),

  revealFolder: (path: string) => invoke<void>("reveal_folder", { path }),
  showMainWindow: () => invoke<void>("show_main_window"),
  hidePanel: () => invoke<void>("hide_panel"),
  quit: () => invoke<void>("quit_app"),
};

// ---------------------------------------------------------------------------
// Events
// ---------------------------------------------------------------------------

export const events = {
  metrics: (handler: (tick: MetricsTick) => void): Promise<UnlistenFn> =>
    listen<MetricsTick>("metrics:tick", (event) => handler(event.payload)),

  remote: (handler: (reports: RemoteReport[]) => void): Promise<UnlistenFn> =>
    listen<{ reports: RemoteReport[] }>("remote:tick", (event) =>
      handler(event.payload.reports),
    ),

  log: (handler: (projectId: string, line: LogLine) => void): Promise<UnlistenFn> =>
    listen<{ projectId: string; line: LogLine }>("log:line", (event) =>
      handler(event.payload.projectId, event.payload.line),
    ),

  status: (
    handler: (projectId: string, status: ProjectStatus) => void,
  ): Promise<UnlistenFn> =>
    listen<{ projectId: string; status: ProjectStatus }>("status:change", (event) =>
      handler(event.payload.projectId, event.payload.status),
    ),
};

/** A blank project, ready for the create form. */
export function emptyProject(): Project {
  return {
    id: "",
    name: "",
    kind: "other",
    icon: { type: "auto" },
    accent: null,
    local: null,
    remote: null,
    repo: null,
    tags: [],
    favorite: false,
    order: 0,
    notes: null,
  };
}
