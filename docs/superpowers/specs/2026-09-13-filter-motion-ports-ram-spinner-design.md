# Filter motion, port safety, RAM, and start/stop feedback

Date: 2026-09-13
Branch: rust-ui
Status: approved

Four independent changes to the Tauri build of Oracle (`src/` + `src-tauri/`). They share no
state and can land in any order; they are specified together because they were requested
together.

## 1. Fluid-glass transition on filter change

### Problem

Changing the filter in the toolbar swaps the list contents instantly. `renderList` calls
`fill()`, which replaces every child node, so there is nothing to animate and no continuity
for a project that appears under both the old and the new filter.

### Constraint that shapes the design

`render()` runs on every state change, including the metrics tick once a second. Animating
on every render would be both visually wrong and a permanent CPU cost. Motion must therefore
be driven by an explicit signal, not inferred from the render itself.

### Design

`main.ts` remembers the last `filter` and `query` it rendered. When either differs, it passes
`animate: true` to `renderList`; every other render passes `false`.

A new `src/lib/motion.ts` owns the animation and nothing else:

- `fluidList(host, render)` — captures the position of each existing card by project id,
  runs `render()`, then:
  - **FLIP** for cards whose id survives: animate the measured delta to zero, so a card that
    stays visible slides to its new row instead of jumping.
  - **Exit** for cards that disappeared: opacity 1 to 0, scale 1 to .96, blur 0 to 6px.
    Exiting nodes are cloned into an absolutely positioned overlay so they do not affect
    layout while they fade.
  - **Enter** for new cards: the inverse, staggered 18 ms per card, capped at 8 steps so a
    long list does not take a second to settle.
- `slidingPill(host)` — the segmented control gets an absolutely positioned indicator that
  translates to the pressed option, easing on `--spring`, tinted with the glass gradient.

All durations come from `--t-page` / `--spring` in `tokens.css`, which
`prefers-reduced-motion` already zeroes. `motion.ts` additionally returns early when
`data-glass` is `reduced` or `opaque`, so the blur component is dropped on machines that
already failed the frame-rate probe.

### Rejected alternative

The View Transitions API is available in this WebView2 and would give the morph for free, but
it snapshots the whole document. With around forty `backdrop-filter` surfaces the snapshot is
both expensive and visually incorrect — glass sampled from a static image of itself. WAAPI on
the handful of elements that actually move costs far less.

## 2. No two projects on the same port

### Problem

Nothing stops a second project starting on a port that is already serving. The first symptom
is a dev server that exits with `EADDRINUSE` a second after Oracle reports it as starting.

### Design

The invariant is enforced in the backend. Enforcing it in the UI would leave the tray panel,
the autostart sweep, and the IPC surface free to violate it.

**`runner/probe.rs`**

- `find_free_port(from: u16) -> Option<u16>`: scans upward from `from + 1` for up to 64
  candidates, confirming each by binding a `TcpListener` on loopback and dropping it. A bind
  is authoritative where a failed connect is not — a port can refuse connections and still be
  unbindable.

**`runner/mod.rs`**

- `ProcHandle` gains `port: Option<u16>`, the port the project declared when it started.
- `ProcessManager::start` takes `port_override: Option<u16>` and, before spawning, rejects:
  1. another running Oracle project holding the same effective port, named in the message;
  2. a foreign process listening on it, identified by PID via the existing `pid_on_port`.
- `apply_port_override(command: &str, port: u16) -> String`: sets `PORT` in the child's
  environment for this start only, and appends `--port <n>` when the command matches a known
  pattern (`vite`, `serve`) and does not already carry a `--port`. Pure function, unit-tested
  against the command strings Oracle actually produces.

Adoption is untouched. A project whose own port is already served is deliberately adopted
rather than refused.

**`error.rs`**

- `OracleError::PortInUse { port, holder, suggestion }`, code `port_in_use`.
- `WireError` gains `data: Option<serde_json::Value>` so a structured error can carry the
  suggestion to the frontend. Today an error is only `code` / `message` / `detail`, which is
  not enough to offer an action.

**Frontend**

- `api.start` accepts an optional port.
- `actions.start` catches `port_in_use` and opens a modal: "Port 3000 is taken by Promethee.
  Start on 3001 instead?" with `[Start on 3001]`, `[Settings]`, `[Cancel]`. Accepting calls
  `api.start(id, 3001)`; the project's stored configuration is never rewritten.

## 3. Resident memory

### Measured starting point

316 MB across seven WebView2 processes: ~197 MB GPU, ~72 MB in two renderers, the rest
plumbing. The renderer count is the interesting part — the tray panel is a second, full
webview built during `setup()` and kept alive, hidden, for the whole session.

Per the decision taken during design, nothing here changes what the app looks like. The
GPU-side cost of `backdrop-filter` and the SVG displacement filters is left alone.

### Design

- **The panel is built on demand.** `build_panel_window` moves out of `setup()`. `tray.rs`
  creates the window on first open (applying the same vibrancy) and destroys it after 60 s
  hidden. The delay keeps repeated toggles instant; the destruction returns the renderer.
  `commands::hide_panel` and the focus-loss handler arm the timer rather than only hiding.
- **Frontend log buffers become an LRU.** `state.logs` currently retains 1000 lines for every
  project ever selected, for the life of the process. It becomes the selected project plus
  the two before it, 500 lines each.
- **Backend rings are released.** A crashed project keeps its handle so its logs stay
  readable; the ring is dropped once the project is deselected and its crash acknowledged.

The result is measured per PID before and after rather than asserted.

## 4. Start/stop spinner

### Problem

`start` flips the project to `starting` optimistically, which is enough for the status dot but
leaves the button unchanged. `stop` shows nothing at all while `api.stop` waits out
`GRACEFUL_STOP`, up to five seconds. Both accept repeated clicks while in flight.

### Design

`State` gains `pending: Record<string, "start" | "stop">`, owned by the UI and distinct from
the backend's `status`.

- `actions.start` / `actions.stop` set it; the `events.status` handler clears it on
  `running`, `unhealthy`, `crashed`, or `stopped`; a 90 s guard clears it if no event arrives.
- `toggle` is a no-op while an operation is in flight, and the button renders `disabled`.
- `common.ts` gains `spinner()` — an SVG ring in `currentColor` with an animated
  `stroke-dasharray` — and `button()` accepts `pending`. Cards, the detail pane, and the tray
  panel all route through `button()`, so they inherit the behaviour without their own changes.
- The spinner and the icon share a box of the same size, so the swap is a cross-fade with no
  layout shift.

## Testing

- Rust unit tests: `apply_port_override` across the command shapes Oracle generates;
  `find_free_port` against a listener it binds itself; `ProcessManager::start` rejecting both
  conflict cases with code `port_in_use`.
- `tsc --noEmit` for the frontend, which is the only static gate the project has.
- Manual verification in the running app for the two visual features, plus a before/after
  memory reading by PID for section 3.
