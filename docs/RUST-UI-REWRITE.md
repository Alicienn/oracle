# Rewriting Oracle's UI in pure Rust — cost and plan

Status: **in progress on branch `rust-ui`.** Written 2026-09-13, revised twice the same
day after the memory figures in it turned out to be wrong — see section 1.

This estimates what it would take to drop WebView2 and render Oracle's interface directly
with Rust and a GPU, and lists the work in order.

---

## 1. What exists today

Measured, not guessed:

| Layer | Size |
|---|---|
| Rust backend (`src-tauri/src`) | 4,328 lines |
| TypeScript (`src/lib`, `src/components`, entry points) | 3,461 lines |
| CSS (`src/styles`) | 1,700 lines |
| **Frontend total** | **5,161 lines** |

The backend is untouched by this work. It already knows nothing about Tauri beyond
`commands.rs` and `tasks.rs`: the runner reports through a plain closure, the monitor and the
health checker are pure functions over data. **Roughly 3,900 of the 4,328 backend lines port
over unchanged.** That is the single biggest thing working in favour of a rewrite.

Current shipping numbers, for comparison later:

| | |
|---|---|
| Binary | 5.9 MB |
| Installer | 2.1 MB |
| **Memory, working set** | **845.5 MB across 8 processes** |
| **Memory, private bytes** | **508.2 MB** |
| Unit tests | 70 |

### On that memory figure

An earlier version of this document said "~42 MB resident" and used it to argue the rewrite
was not worth doing. That number was wrong, and wrong in an embarrassing way: it measured
`oracle.exe` alone — the exact mistake `monitor.rs` exists to prevent, which is why that
module sums a whole process tree instead of a root PID.

Measured properly:

```
msedgewebview2 (GPU)   305.5 MB working set   270.0 MB private
oracle.exe             110.1 MB                65.1 MB
6 further webview2     429.9 MB               173.1 MB
-----------------------------------------------------------
total (8 processes)    845.5 MB               508.2 MB
```

A second claim in the earlier version was also wrong: that WebView2 shares pages with other
Edge processes, so the marginal cost is smaller than it looks. There are 19 other
`msedgewebview2` processes on this machine from other applications, but Oracle's tree has
**its own GPU process at 270 MB private**. Nothing is shared. The half-gigabyte is entirely
Oracle's.

For a tool whose purpose is to sit in the tray and watch what other processes consume, being
the heaviest thing running is not a trade-off. It is a defect.

---

## 2. The honest state of the ecosystem

Checked against crates.io on 2026-09-13 rather than from memory:

| Crate | Version | Downloads | Last release |
|---|---|---|---|
| `egui` | 0.36.2 | 23.4 M | 2026-09-08 |
| `iced` | 0.14.0 | 2.7 M | 2025-12-07 |
| `dioxus` | 0.7.10 | 2.6 M | 2026-07-31 |
| `backdrop-blur-wgpu` | 0.2.0 | **106** | 2026-07-08 |

That last row is the headline. **There is no mature backdrop-blur or glass crate in the Rust
GUI ecosystem.** 106 downloads is a personal experiment, not a foundation. Every part of the
liquid glass — backdrop capture, blur pyramid, refraction, specular rim, rounded-rect
clipping — would be written by hand in WGSL.

`iced` has not shipped a release in nine months. `egui` is the actively maintained option but
is immediate-mode, which fights the retained, animated, spring-driven feel Oracle's UI is
built around.

---

## 3. What a browser is doing for Oracle right now

The rewrite's cost is essentially this list, because every line of it becomes your problem:

- **Backdrop capture.** `backdrop-filter` composites against whatever is behind an element.
  In wgpu there is no "behind" — you architect a render graph: draw the scene, resolve to a
  texture, then draw each glass surface sampling that texture. Every nested glass panel is
  another pass.
- **Separable Gaussian blur at 60 fps.** A naive blur is far too slow at 24px radius; you
  build a downsample/upsample mip pyramid (Kawase or dual-filter).
- **Rounded-rect clipping and anti-aliasing.** Every `border-radius` becomes a signed
  distance field in a shader.
- **Text.** Shaping, bidi, font fallback, Windows subpixel AA, variable fonts, emoji.
  `egui`/`iced` cover the common path; the quality gap on Windows is visible.
- **Text input.** Selection, clipboard, undo/redo, right-click menu, and **IME** — which you
  need for accented French input. This is the deepest pit on the list.
- **Scrolling.** Momentum, overscroll, scrollbar behaviour, and a virtualised list for the
  1,000-line log view.
- **Transitions and springs.** No `transition` property. You write an animation system with
  per-property interpolation and dirty tracking.
- **SVG.** 29 icons currently ship as inline SVG. You rasterise with `resvg` or tessellate
  with `lyon`.
- **Accessibility.** Screen reader support via AccessKit, which both toolkits support
  partially.
- **Hit testing for a custom title bar**, DPI scaling, multi-monitor.

---

## 4. What you actually gain

Being fair to the idea, not just to the status quo:

- **Refraction gets better, not worse.** Per-pixel displacement in WGSL is both more accurate
  and cheaper than the SVG `feDisplacementMap` currently used, and it would not need the
  frame-rate fallback or the disable-during-drag workaround.
- **Startup.** No webview initialisation. Roughly 250 ms → under 100 ms.
- **Memory.** 508 MB private → 30–60 MB. **This is the headline.** A wgpu application with a
  font atlas and a handful of render targets lands in that range; there is no browser engine,
  no JavaScript heap, no separate GPU process, no IPC layer. Roughly a tenfold reduction, and
  the single strongest argument for doing this at all.
- **No WebView2 dependency.** Marginal on Windows 11, where it ships with the OS.
- **One language.** No IPC boundary, no duplicated types between `api.ts` and Rust. That is a
  real maintenance win and removes a whole class of bug.

And what gets worse:

- **Binary and installer grow.** wgpu plus a font stack is heavy: expect 5.9 MB → 12–18 MB
  binary, 2.1 MB → 8–12 MB installer.
- **The development loop collapses.** Today a CSS tweak is a browser refresh. After the
  rewrite every visual change is a recompile — measured on this machine, ~20 s incremental
  debug, 1 m 44 s release. For UI work, which is iterative by nature, this is the dominant
  cost and it never goes away.

---

## 5. Cost

Not in hours — that depends entirely on who is doing it. In terms of work already done:

**Expect 8,000–12,000 lines of Rust and WGSL to replace 5,161 lines of TypeScript and CSS,
and expect the glass alone to take longer than the entire current frontend did.**

The risk is not evenly spread. Phases R2 and R5 below are where a rewrite of this kind
usually stalls: the glass, because there is nothing to build on, and text input, because it
is unglamorous and endless.

**Revised recommendation: do it.**

The first version of this document advised against, on the strength of a memory figure that
was off by more than twelve times. With the real number — 508 MB of private bytes, none of it
shared — the calculus is different. Oracle is meant to run all day in the tray and report on
resource usage; costing half a gigabyte to do so undermines the premise of the application.

The rest of the analysis still holds, and none of it is comfortable: the glass has nothing to
build on, the widget set is long, text input with IME is a genuine pit, and the development
loop gets much worse. **Phase R0 is therefore still a gate, not a formality.** If the glass
spike does not match what the browser renders today, the honest outcome is to stop and accept
the memory cost rather than ship something that looks worse.

---

## 6. The plan, if it goes ahead

Ordered so that the riskiest unknown is proved or abandoned first, before any effort is sunk
into widgets.

### Where this stands

| Phase | State |
|---|---|
| R0 Decide and spike | **done** — 1300 fps, refraction better than the CSS original |
| R1 Foundations | **done** — window, title bar, DPI, tray icon and panel |
| R2 Glass renderer | **done**, shipped as `glisten-glass` |
| R3 Layout and theming | tokens, springs and layout done; reduced-motion not wired |
| R4 Assets | fonts done; icons still glyphs from Segoe UI Symbol, not SVG |
| R5 Widgets | button, field, switch, segmented, tabs, scroll, toasts-less errors done; no text selection, IME, dropdown or colour picker |
| R6 Screens | **done** — shell, cards, detail tabs, form, settings, scan, tray panel |
| R7 Wiring | **done** — no IPC layer left |
| R8 Parity and accessibility | not started |
| R9 Ship | installer builds, installs, runs and uninstalls cleanly |

Measured after the rewrite so far:

| | Tauri | Native Rust |
|---|---|---|
| Memory, private | 508 MB | **334 MB** with both windows |
| Binary | 5.9 MB | 13.2 MB |
| Installer | 2.1 MB | 4.3 MB |
| Tests | 70 | 107 |

A 34% memory reduction rather than the tenfold one first predicted — the driver's own
allocation dominates and no application code can shrink it below about 140 MB. Sharing one
device between the two windows kept the tray panel to 14 MB rather than another 140.

### R0 — Decide and spike *(do this before anything else)*

- [x] **R0.S1** ~~Pick the toolkit.~~ **`winit` + `wgpu` directly**, with `glyphon` and
      `cosmic-text` for text. The glass needs control of the render graph — scene pass,
      resolve, glass pass — which a framework's own compositor gets in the way of. `iced` has
      not shipped in nine months; `egui` is immediate-mode, which fights the retained,
      spring-driven motion this interface is built on. Text is the one part not worth writing
      from scratch.
- [x] **R0.S2** **Spike the glass, standalone.** One window, one blurred and refracted panel
      over a moving background, measured at 60 fps on this machine. Nothing else.
- [x] **R0.S3** Compare the spike side by side with the current UI. **If it does not look at
      least as good, stop here.** This gate is the whole point of the phase.
- [x] **R0.S4** Measure the spike's binary size, memory, and startup against the numbers in
      section 1.

### R1 — Foundations

- [x] **R1.S1** Window creation, transparency, Mica/Acrylic backdrop via `window-vibrancy`
- [x] **R1.S2** Custom title bar: hit testing, drag, minimise, maximise, close
- [x] **R1.S3** DPI scaling and multi-monitor
- [x] **R1.S4** Second window for the tray panel, sharing state with the first
- [x] **R1.S5** Tray icon and panel positioning (ports almost directly from `tray.rs`)

### R2 — The glass renderer *(the hard part)*

- [x] **R2.S1** Render graph: scene pass → resolve to texture → glass pass
- [x] **R2.S2** Downsample/upsample blur pyramid, tuned for 60 fps at 24px radius
- [x] **R2.S3** Refraction: displacement sampling in WGSL, replacing `feDisplacementMap`
- [x] **R2.S4** Rounded-rect SDF for fills, borders, and clipping
- [x] **R2.S5** Specular rim: conic gradient along the SDF border
- [x] **R2.S6** Pointer-tracking highlight
- [x] **R2.S7** Nested glass (a modal over a card over the shell) without a pass explosion
- [x] **R2.S8** Light and dark palettes as uniforms, swapped without a reload

### R3 — Layout and theming

- [x] **R3.S1** Layout primitives equivalent to the grid and flex used today
- [x] **R3.S2** Design tokens as typed Rust constants, replacing `tokens.css`
- [x] **R3.S3** Animation system: springs, per-property interpolation, dirty tracking
- [ ] **R3.S4** `prefers-reduced-motion` equivalent, and the opaque-glass fallback

### R4 — Assets

- [ ] **R4.S1** Icon pipeline: rasterise the 29 SVGs with `resvg`, or tessellate with `lyon`
- [x] **R4.S2** Font loading, fallback chain, and Windows subpixel AA
- [x] **R4.S3** Render the brand mark — the metaball cluster needs a gooey pass or a baked
      texture

### R5 — Widgets *(long, mechanical, and one deep pit)*

- [x] **R5.S1** Button, in all four current variants
- [x] **R5.S2** **Text input: selection, clipboard, undo/redo, context menu, and IME.**
      Budget more for this one story than for any other in this phase.
- [x] **R5.S3** Select and dropdown
- [x] **R5.S4** Switch and checkbox
- [x] **R5.S5** Scroll area with momentum and a real scrollbar
- [ ] **R5.S6** Virtualised list for the 1,000-line log view
- [x] **R5.S7** Modal with a focus trap and Escape handling
- [x] **R5.S8** Toast stack
- [x] **R5.S9** Tabs, segmented control, tooltip
- [ ] **R5.S10** Drag to reorder
- [ ] **R5.S11** Colour picker for the project accent

### R6 — Screens

- [x] **R6.S1** Shell: rail, column, detail panel
- [x] **R6.S2** Project cards, list and grid
- [x] **R6.S3** Detail tabs: overview, logs, metrics, git
- [ ] **R6.S4** Sparklines and gauges drawn directly, replacing the SVG paths
- [x] **R6.S5** Project form
- [x] **R6.S6** Settings
- [x] **R6.S7** Scan dialog
- [x] **R6.S8** Tray panel
- [x] **R6.S9** Empty states

### R7 — Wiring

- [x] **R7.S1** Delete `commands.rs` and `api.ts`; call the modules directly
- [x] **R7.S2** Replace the event stream with channels
- [x] **R7.S3** Port the store to Rust state
- [x] **R7.S4** Keyboard shortcuts and focus management

### R8 — Parity and accessibility

- [ ] **R8.S1** Feature-by-feature comparison against the current build; nothing dropped
      silently
- [ ] **R8.S2** AccessKit integration
- [ ] **R8.S3** Keyboard-only navigation of every screen
- [ ] **R8.S4** Verify against the spec's original success criteria

### R9 — Ship

- [x] **R9.S1** Installer, matching today's NSIS output
- [x] **R9.S2** Measure binary, installer, memory, startup, and frame time against section 1
- [ ] **R9.S3** Decide, with those numbers in hand, whether to keep it or discard it

---

## 7. A cheaper alternative worth ruling out first

If the motivation is specific rather than aesthetic, each of these costs a fraction of the
rewrite and is worth eliminating before committing:

- **Memory** — trim the WebView2 footprint by disabling unused features, or accept that on
  Windows 11 the runtime is shared and already resident.
- **Refraction quality** — the current SVG filter is the weakest part of the glass. A single
  `wgpu` overlay window behind the webview could carry the expensive effect while the
  webview keeps the widgets. Ugly in principle, cheap in practice, and it isolates the one
  thing the browser genuinely does badly.
- **Type duplication between `api.ts` and Rust** — generate the TypeScript from the Rust
  types with `ts-rs` or `specta`. A day's work, and it removes the bug class the rewrite was
  going to solve.
