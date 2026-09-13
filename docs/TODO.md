# Oracle — Implementation plan

Legend: `[ ]` todo · `[~]` in progress · `[x]` done · `[!]` blocked

Last updated: 2026-09-13 — complete. Installer built and verified running.

---

## Progress

| Epic | Title | Stories | State |
|---|---|---|---|
| E0 | Foundations and tooling | 6 | `[x]` |
| E1 | Brand and assets | 4 | `[x]` |
| E2 | Config and data model | 6 | `[x]` |
| E3 | Project discovery | 4 | `[x]` |
| E4 | Process supervision | 8 | `[x]` |
| E5 | Resource monitoring | 5 | `[x]` |
| E6 | Remote status (VPS) | 4 | `[x]` |
| E7 | Git integration | 3 | `[x]` |
| E8 | IPC surface | 3 | `[x]` |
| E9 | Liquid glass design system | 7 | `[x]` |
| E10 | Main window | 8 | `[x]` |
| E11 | Tray panel | 5 | `[x]` |
| E12 | Settings and autostart | 5 | `[x]` |
| E13 | Robustness and errors | 5 | `[x]` |
| E14 | Tests | 4 | `[x]` |
| E15 | Installer build | 5 | `[x]` |

---

## E0 — Foundations and tooling

- [x] **E0.S1** Repository layout, `.gitignore`, `README.md`
- [x] **E0.S2** `package.json` with frontend dependencies installed
- [x] **E0.S3** `vite.config.ts` with two entry points (`index.html`, `panel.html`)
- [x] **E0.S4** `tsconfig.json` in strict mode
- [x] **E0.S5** `src-tauri/Cargo.toml` with every dependency resolved, `build.rs`
- [x] **E0.S6** `tauri.conf.json`: two windows, transparency, capabilities

**Exit criterion**: `cargo check` passes, `npm run build` passes.

---

## E1 — Brand and assets

- [x] **E1.S1** Copy the source logo to `assets/logo.png`
- [x] **E1.S2** Generate the Tauri icon set (`.ico`, multi-size `.png`) via `tauri icon`
- [x] **E1.S3** Icon set rebuilt from a transparent, edge-to-edge mark: no white tile in the taskbar or tray, and dedicated tray sizes
- [x] **E1.S4** Reproduce the mark as inline SVG for the UI (splash, header, about)

**Exit criterion**: icons present in `src-tauri/icons/`, mark rendered in the app.

---

## E2 — Config and data model

- [x] **E2.S1** `config::model` — `Project`, `LocalTarget`, `RemoteTarget`, `RepoRef`, `Settings`, `Config`
- [x] **E2.S2** Serde derives, defaults, `version` field for migration
- [x] **E2.S3** Path resolution (`%APPDATA%\Oracle\config.json`), directory creation
- [x] **E2.S4** Atomic write (temp + rename), read with fallback
- [x] **E2.S5** Corrupt-config recovery (rename + defaults + surfaced warning)
- [x] **E2.S6** Project CRUD: add, update, delete, reorder

**Exit criterion**: round-trip and recovery unit tests green.

---

## E3 — Project discovery

- [x] **E3.S1** Bounded recursive scan (max depth 4, skipping `node_modules`/`target`/`.git`/`dist`)
- [x] **E3.S2** Kind detection: `package.json` (+ `next` dependency → NextJs), `Cargo.toml`, `pyproject.toml`/`requirements.txt`, `docker-compose.yml`, `index.html`
- [x] **E3.S3** Default command and port inferred per kind
- [x] **E3.S4** Git remote detection to pre-fill `RepoRef`

**Exit criterion**: scanning `Documents` proposes a correct, deduplicated candidate list.

---

## E4 — Process supervision

- [x] **E4.S1** `ProcessManager`: `HashMap<ProjectId, ProcHandle>` behind an `RwLock`
- [x] **E4.S2** Command line parsing (quote handling), spawn through `cmd /C` on Windows
- [x] **E4.S3** Environment injection, `current_dir`, detached process group
- [x] **E4.S4** stdout/stderr capture in Tokio tasks, 1000-line ring buffer
- [x] **E4.S5** Graceful stop, then `taskkill /T /F` on the tree after 5 s
- [x] **E4.S6** Exit detection, distinguishing a requested stop from a crash
- [x] **E4.S7** Port probe: `Running` only once the port accepts a connection
- [x] **E4.S8** Adoption of processes started outside Oracle (port → PID resolution)

**Exit criterion**: a full start/stop cycle on a real Next.js project, leaving no orphans.

---

## E5 — Resource monitoring

- [x] **E5.S1** `sysinfo` sampler at 1 Hz in a dedicated Tokio task
- [x] **E5.S2** Process tree reconstruction from the root PID
- [x] **E5.S3** CPU % and RSS aggregation across all descendants
- [x] **E5.S4** 60-sample ring buffer per project
- [x] **E5.S5** Single `metrics:tick` event covering every project

**Exit criterion**: displayed values match Task Manager within 10%.

---

## E6 — Remote status (VPS)

- [x] **E6.S1** `reqwest` client (rustls), 5 s timeout, redirects not followed by default
- [x] **E6.S2** Concurrent health check loop on a configurable interval
- [x] **E6.S3** Status classification: `Up` / `Degraded` / `Down`, with latency
- [x] **E6.S4** `remote:tick` event, in-memory latency history

**Exit criterion**: a real VPS URL reports `Up` with a plausible latency.

---

## E7 — Git integration

- [x] **E7.S1** Detect `git` on `PATH`, degrade cleanly when absent
- [x] **E7.S2** Parse `git status --porcelain=v2 --branch`: branch, ahead/behind, cleanliness
- [x] **E7.S3** Last commit (`git log -1 --format=…`): short hash, subject, relative date

**Exit criterion**: correct status on a clean repo, a dirty repo, and one ahead of upstream.

---

## E8 — IPC surface

- [x] **E8.S1** Shared `AppState`, injected into commands
- [x] **E8.S2** Every `#[tauri::command]` handler (projects, launching, logs, scan, settings)
- [x] **E8.S3** Typed `api.ts` on the frontend, mirroring the Rust types exactly

**Exit criterion**: every command callable from the frontend with correct types.

---

## E9 — Liquid glass design system

- [x] **E9.S1** `tokens.css`: colours, radii, shadows, durations, springs, light and dark themes
- [x] **E9.S2** SVG refraction filters (`feTurbulence` + `feDisplacementMap`), three intensities
- [x] **E9.S3** `.glass` class: all three layers (refraction, illumination, highlight)
- [x] **E9.S4** Cursor-tracking specular highlight (CSS variables driven by throttled JS)
- [x] **E9.S5** Primitives: button, field, card, pill, toggle, modal, toast
- [x] **E9.S6** Performance fallback: frame rate probe, degraded mode, disable during drag
- [x] **E9.S7** `prefers-reduced-motion` and opaque glass mode

**Exit criterion**: a component gallery page holding 60 fps in light and dark.

---

## E10 — Main window

- [x] **E10.S1** Shell: custom title bar (drag, minimise, close), rail, column, panel
- [x] **E10.S2** Reactive store and render layer
- [x] **E10.S3** Project rail: icons, status dots, drag to reorder
- [x] **E10.S4** Project cards in list and grid, with status, kind, sparkline
- [x] **E10.S5** Detail panel: Overview / Logs / Metrics / Git / Settings tabs
- [x] **E10.S6** Log viewer (virtualised, auto-scroll, filter, copy)
- [x] **E10.S7** Add and edit project form with icon picker
- [x] **E10.S8** Global search and filters

**Exit criterion**: a full add → launch → observe → stop journey without touching disk by hand.

---

## E11 — Tray panel

- [x] **E11.S1** Tray icon, minimal menu (Open / Quit), left click toggles the panel
- [x] **E11.S2** Panel positioning near the tray, multi-monitor and DPI aware
- [x] **E11.S3** Contents: global gauges, search, compact list with play/stop
- [x] **E11.S4** Hide on blur, spring entrance animation
- [x] **E11.S5** Global shortcut `Ctrl+Shift+Space`

**Exit criterion**: the panel opens in the right place on both primary and secondary monitors.

---

## E12 — Settings and autostart

- [x] **E12.S1** Full settings screen
- [x] **E12.S2** Windows autostart through the `Run` registry key
- [x] **E12.S3** Start hidden in the tray
- [x] **E12.S4** Auto-launch flagged projects
- [x] **E12.S5** Manage the list of root folders to scan

**Exit criterion**: autostart verified after a real reboot.

---

## E13 — Robustness and errors

- [x] **E13.S1** Typed `OracleError`, serialised to the frontend
- [x] **E13.S2** Toast system with expandable detail
- [x] **E13.S3** Failure isolation: one failing project does not affect the others
- [x] **E13.S4** Background tasks protected against panics
- [x] **E13.S5** Single instance, clean shutdown of every child on exit

**Exit criterion**: injecting an invalid command, a dead URL, and a corrupt config — the app
survives all three.

---

## E14 — Tests

- [x] **E14.S1** Unit tests: detection, command parsing, ring buffer, git parsing
- [x] **E14.S2** Config tests: round-trip, defaults, corruption recovery
- [x] **E14.S3** Process lifecycle integration test
- [x] **E14.S4** `cargo clippy` and `tsc --noEmit` clean

**Exit criterion**: `cargo test` green, clippy clean.

---

## E15 — Installer build

- [x] **E15.S1** Bundle metadata: identifier, publisher, version, copyright
- [x] **E15.S2** Release profile: LTO, `codegen-units=1`, `panic=abort`, `strip`
- [x] **E15.S3** NSIS configuration: language, install mode, shortcuts
- [x] **E15.S4** `cargo tauri build` produces the installer
- [x] **E15.S5** Verification: install, launch, uninstall

**Exit criterion**: a working `Oracle_0.1.0_x64-setup.exe`.
