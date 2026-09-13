# Oracle — Specification

> A single command center for every project I run: see them, launch them, watch them.

Document version: 1.0 — 2026-09-13
Status: approved, implementation in progress

---

## 1. Problem

I have a lot of small side projects. They are wildly heterogeneous:

- some run **locally** (Next.js under `npm run dev`, Python scripts, Rust servers)
- some are **deployed to my VPS** (GitHub repo with CI/CD)
- some have a **GitHub repo**, some don't
- each has its own launch command, port, and environment variables

The result: I never know what is running, where, or in what state. Starting a project means
finding the folder, opening a terminal, and remembering the command.

**Oracle** collapses all of that into one surface.

---

## 2. Goals

| # | Goal | Success criterion |
|---|---|---|
| G1 | See every project's state at a glance | The tray panel shows N projects without scrolling for N ≤ 8 |
| G2 | Start and stop a local project in one click | Click to live process: under 500 ms of Oracle-side latency |
| G3 | Know whether a VPS app is up | Periodic HTTP health check with latency displayed |
| G4 | Watch what running projects consume | CPU and memory aggregated over the process tree, sampled at 1 Hz |
| G5 | Add and customise a project without editing files | Full form in the UI, uploadable icon |
| G6 | A UI worth looking at | Coherent liquid glass, 60 fps, no jank while dragging |
| G7 | Always within reach | Tray icon plus floating panel, optional start with Windows |

### Non-goals for v1

- Deploying or rolling back from Oracle
- Editing files or embedding a full terminal
- Managing more than one local machine
- Multi-user or cloud sync
- macOS and Linux support (the code stays portable, but is not tested there)

---

## 3. Technical decisions

### D1 — Stack: Tauri 2 (Rust) with a Vite/TypeScript frontend

**Why.** The part of Oracle that needs to be fast is the backend: disk scanning, process
supervision, `sysinfo` sampling, concurrent health checks. All of that is native Rust. The
UI, by contrast, needs fast iteration and sophisticated glass effects — territory where CSS
plus SVG filters beats a hand-written shader by a wide margin.

**Rejected alternative.** Pure Rust GPU (`iced` / `wgpu`) would give physically exact
refraction and roughly 25 MB of RAM, but costs three to four times the UI development effort
for a gain nobody can see in a management tool. WebView2 is already resident in memory on
Windows 11, so the real overhead is small.

**No frontend framework.** No React, Vue, or Solid. Components are functions that return DOM
nodes, backed by a small hand-written reactive store (around 80 lines). Target bundle: under
40 KB. Oracle's DOM is small and its updates are targeted; a framework would be weight
without benefit.

### D2 — Persistence: JSON on disk, no database

`%APPDATA%\Oracle\config.json`, written atomically (temp file, then rename).

Metrics are deliberately **not** persisted: they live in an in-memory ring buffer of 60
samples. Nobody wants to inspect the CPU usage of two days ago. That removes SQLite, a C
dependency to compile, and a schema migration story.

### D3 — Remote status: HTTP health checks

Oracle issues a periodic `GET` (or `HEAD`) against one URL per remote project and records
the HTTP status and latency. Nothing to install on the VPS, and it works regardless of how
the app is deployed — Docker, systemd, or behind a reverse proxy.

**Rejected for v1.** SSH (`docker ps`, `systemctl status`) is richer but drags in key
management, a connection pool, and brittle output parsing. A Rust agent deployed on the VPS
is the cleanest design but is a second project in its own right. The data model leaves room
for both: `RemoteCheck` is an enum.

### D4 — Git: shell out to `git`, not `libgit2`

A single `git status --porcelain=v2 --branch` yields branch, ahead/behind, and worktree
cleanliness. `git2` would compile C for no gain here. If `git` is absent from `PATH`, the
project is simply marked as having no VCS.

### D5 — Monitoring: `sysinfo`, aggregated per process tree

`next dev` spawns several children; measuring only the root PID badly underestimates real
usage. Oracle rebuilds the tree from the root PID and sums CPU and RSS across every
descendant.

### D6 — Two windows, one codebase

- `main` — the full application, 1240×820, resizable
- `panel` — the floating tray panel, 400×620, undecorated, always on top, hidden on blur

Two Vite entry points (`index.html`, `panel.html`) sharing CSS tokens, the store, and the
API layer.

---

## 4. Architecture

```
┌──────────────────────── WebView2 ────────────────────────┐
│  index.html (app)              panel.html (tray)         │
│       │                              │                   │
│       └──── store.ts ── api.ts ──────┘                   │
└──────────────────────────│───────────────────────────────┘
                           │  invoke / events
┌──────────────────────────▼─── Rust ──────────────────────┐
│  commands.rs        — IPC surface                        │
│  ┌────────────┬─────────────┬──────────┬──────────────┐  │
│  │ config     │ discovery   │ runner   │ monitor      │  │
│  │ (JSON)     │ (scan+kind) │ (procs)  │ (sysinfo)    │  │
│  ├────────────┼─────────────┼──────────┼──────────────┤  │
│  │ vcs (git)  │ remote      │ tray     │ autostart    │  │
│  │            │ (HTTP)      │          │              │  │
│  └────────────┴─────────────┴──────────┴──────────────┘  │
│  AppState: RwLock<Config> + ProcessManager + Monitor     │
└──────────────────────────────────────────────────────────┘
```

### Data flow

**Supervision loop.** One Tokio task runs at 1 Hz: it refreshes `sysinfo`, aggregates per
tree for every live project, pushes into the ring buffer, and emits a single `metrics:tick`
event carrying the state of all projects. The frontend never *asks* for metrics — it
listens. One event per second, regardless of project count.

**Health check loop.** A separate task on a configurable period (30 s by default), issuing
concurrent requests with a 5 s timeout, emitting `remote:tick`.

**Logs.** Each spawned process gets two reader tasks (stdout, stderr) that push lines into a
1000-line ring buffer and emit `log:line`. The frontend only subscribes while the log pane is
open.

### Rust modules

| Module | Responsibility | Depends on |
|---|---|---|
| `error` | `OracleError` (thiserror), serialisable to the frontend | — |
| `config::model` | `Project`, `Settings`, `Config` types | — |
| `config` | Load, atomic write, defaults, version migration | `model` |
| `discovery` | Bounded recursive scan, project kind detection | `model` |
| `runner` | Spawn, tree kill, log capture, adoption by port | `model`, `error` |
| `monitor` | Sampling, tree aggregation, history | `runner` |
| `remote` | Concurrent HTTP health checks | `model` |
| `vcs` | Git status | — |
| `tray` | Tray icon, panel positioning | `config` |
| `autostart` | Windows `Run` registry key | — |
| `commands` | `#[tauri::command]` handlers | all |

Each module exposes a narrow API and knows nothing about the others beyond what it imports.
`runner` has no concept of a health check; `monitor` has no concept of a remote project.

---

## 5. Data model

```rust
struct Project {
    id: Uuid,
    name: String,
    kind: ProjectKind,          // NextJs | Node | Rust | Python | Docker | Static | Other
    icon: IconSource,           // Builtin(name) | File(path) | Favicon(url) | Auto
    accent: Option<String>,     // hex colour, otherwise derived from kind
    local: Option<LocalTarget>,
    remote: Option<RemoteTarget>,
    repo: Option<RepoRef>,
    tags: Vec<String>,
    favorite: bool,
    order: i32,
}

struct LocalTarget {
    root: PathBuf,
    command: String,            // e.g. "npm run dev"
    env: BTreeMap<String, String>,
    port: Option<u16>,          // expected; detected otherwise
    open_url: Option<String>,   // otherwise http://localhost:{port}
    autostart_with_oracle: bool,
}

struct RemoteTarget {
    url: String,
    check: RemoteCheck,         // Http { method, expect_status } — extensible
    interval_secs: u32,
}

struct RepoRef {
    provider: RepoProvider,     // GitHub | Other | None
    slug: Option<String>,       // "owner/name"
    branch_hint: Option<String>,
}
```

### States

```
ProjectStatus = Stopped | Starting | Running | Unhealthy | Crashed | Unknown
RemoteStatus  = Up { ms } | Down { reason } | Degraded { code } | Unchecked
```

A local project moves `Starting → Running` once the process is alive **and** its port
accepts a connection (or immediately, if no port is declared). It moves to `Crashed` if the
process exits non-zero without having been asked to stop.

---

## 6. Error handling

The principle: **no error is swallowed, no error takes the app down.**

- A typed `OracleError` crosses the IPC boundary as `{ code, message, detail }`. The frontend
  shows a toast with the short message and an expandable section for the detail.
- Project failures are isolated: an invalid launch command marks that one project `Crashed`
  with its captured stderr, and leaves every other project untouched.
- An unreadable or corrupt config makes Oracle start from defaults, rename the offending file
  to `config.corrupt-{timestamp}.json`, and surface the fact in the UI. Never a silent loss.
- Timeouts everywhere: 5 s on health checks, 5 s of graceful shutdown before a forced tree
  kill.
- Panics: background tasks are wrapped so the loop restarts rather than dying.

---

## 7. UI design

### Palette

| Token | Value | Use |
|---|---|---|
| `--accent` | `#C15F3C` | Actions, active status, brand |
| `--paper` | `#F4F3EE` | Light-theme background |
| `--ink` | `#1A1815` | Light-theme text |
| `--muted` | `#B1ADA1` | Secondary text, borders |
| `--white` | `#FFFFFF` | Negative space, surfaces |

The dark theme derives from these: background `#141310`, darker glass surfaces, unchanged
accent.

Status colours: `running` = accent, `stopped` = `--muted`, `crashed` = `#B3442E`,
`unhealthy` = `#C99A2E`, `remote up` = `#5B8C6E`.

### Liquid glass

Three stacked layers, following Apple's model (highlight / shadow / illumination):

1. **Refraction** — an SVG `feTurbulence` + `feDisplacementMap` filter applied to the
   backdrop via `backdrop-filter: url(#lg-refract)`. This is the part plain `blur()` cannot
   do: displace pixels. Low intensity (scale 8–14) on large surfaces, stronger (20+) on small
   floating elements.
2. **Illumination** — `backdrop-filter: blur(24px) saturate(180%)` plus an
   `rgba(255,255,255,0.18)` veil.
3. **Specular highlight** — a conic-gradient border via `border-image` or a masked `::before`
   pseudo-element, brightest at the top-left, tracking the cursor on interactive elements.

Shadows come in two tiers (near and tight, far and diffuse) to separate planes.

Motion uses springs, never linear `ease-in-out`. Default `cubic-bezier(0.32, 0.72, 0, 1)`
over 420 ms for panel entrances, 180 ms for hover states. `prefers-reduced-motion` drops
transitions to 0 ms and turns refraction into a static blur.

**Performance fallback.** The refraction filter is expensive during resize. Oracle measures
frame time at startup; below 50 fps it falls back to `blur()` alone and says so in settings.
During a drag or window resize, refraction is disabled and restored when the gesture ends.

### Main window

A left icon rail of projects (reorderable) · a centre column (list or grid of project cards)
· a contextual right panel (the selected project's logs, metrics, git, and settings).

The header holds global search, filters (all / local / remote / running), a theme toggle, and
settings.

### Tray panel

400 × 620, anchored above the tray icon, springing up from below with a slight blur.

Contents: global CPU and memory gauges at the top · a search field · the project list with a
status dot, a mini sparkline, and a play/stop button · a footer with "Open Oracle" and
settings.

It is a real interface, not a context menu: the same components, the same glass, the same
interactions as the app.

---

## 8. Settings

- Start with Windows (`Run` registry key, no elevation required)
- Start hidden in the tray
- Auto-launch projects flagged for it when Oracle starts
- Theme: light / dark / system
- Glass intensity: full / reduced / opaque
- Health check interval
- Root folders to scan
- Global shortcut to open the panel (default `Ctrl+Shift+Space`)

---

## 9. Testing

- **Unit** (`cargo test`): project kind detection, command parsing, process tree aggregation,
  ring buffer, `git status --porcelain=v2` parsing, config serialisation and migration.
- **Integration**: a full launch → supervise → stop cycle against a toy binary; config
  corruption and recovery; health checks against a local HTTP server.
- **Manual**: glass rendering in light and dark, tray behaviour, autostart after a reboot,
  the installer on a clean machine.

---

## 10. Deliverable

A Windows NSIS installer (`Oracle_0.1.0_x64-setup.exe`) produced by `cargo tauri build`, with
icons generated from the logo, a Start menu shortcut, and an uninstaller.
