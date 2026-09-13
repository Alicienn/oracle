//! The application: state, event loop, and the layout of every screen.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use glisten_glass::Blur;
use glisten_motion::{Animated, Motion};
use winit::application::ApplicationHandler;
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow};
use winit::keyboard::{Key as WinitKey, NamedKey};
use winit::window::{Window, WindowId};
#[cfg(windows)]
use winit::platform::windows::WindowAttributesExtWindows;

use crate::diag;
use crate::core::config::{self, model::*};
use crate::core::monitor::{Monitor, Sample};
use crate::core::runner::{ProcessManager, ProjectStatus, RunnerEvent};
use crate::ui::input::{id, Input, Key};
use crate::ui::paint::{Frame, Rect};
use crate::ui::renderer::{Gpu, Target};
use crate::ui::text::{Align, Run};
use crate::ui::theme::{self, gap, radius, text, Colour, GlassStyle, Palette};
use crate::ui::widgets::{Ui, Weight};

/// Height of the custom title bar, in logical pixels.
const TITLEBAR: f32 = 42.0;
/// Width of the project rail.
const RAIL: f32 = 64.0;

pub struct Oracle {
    window: Option<Arc<Window>>,
    gpu: Option<Gpu>,
    target: Option<Target>,

    /// The tray panel: a second window on the same device, hidden until the tray is
    /// clicked. Kept alive rather than created on demand, because building a surface and a
    /// glyph atlas takes long enough to be visible.
    panel_window: Option<Arc<Window>>,
    panel_target: Option<Target>,
    panel_frame: Frame,
    panel_visible: bool,
    /// Set by the tray thread; acted on from the event loop.
    tray_events: std::sync::mpsc::Receiver<TrayCommand>,
    tray_sender: std::sync::mpsc::Sender<TrayCommand>,
    tray: Option<tray_icon::TrayIcon>,

    config: Config,
    palette: Palette,
    scale: f32,

    input: Input,
    frame: Frame,

    selected: Option<String>,
    query: String,
    filter: Filter,
    tab: Tab,
    /// Which full-window screen is in front of the project list, if any.
    screen: Screen,

    /// Git status per project, read on demand rather than polled: it shells out, and a
    /// repository does not change while nobody is looking at it.
    git: HashMap<String, Option<crate::core::vcs::GitStatus>>,
    /// Remote health per project, refreshed on its own interval.
    remote: HashMap<String, crate::core::remote::RemoteStatus>,
    health: Arc<parking_lot::Mutex<Vec<(String, crate::core::remote::RemoteStatus)>>>,
    last_health: Instant,
    http: reqwest::Client,

    /// The project being added or edited, if the form is open.
    draft: Option<Project>,
    /// True when the draft is new rather than an edit of an existing project.
    draft_is_new: bool,

    /// Text widths measured at the end of a frame and reused by the next one.
    ///
    /// Only the text engine knows how wide a string renders, and it is not reachable from
    /// the layout pass. One frame of lag on a caret is invisible; guessing from a character
    /// count is not.
    measured: HashMap<String, f32>,
    to_measure: Vec<String>,

    /// Candidates from the last folder scan, and which are ticked.
    candidates: Vec<crate::core::discovery::Candidate>,
    chosen: std::collections::HashSet<String>,
    scanning: bool,
    scan_result: Arc<parking_lot::Mutex<Option<Vec<crate::core::discovery::Candidate>>>>,

    /// Width of the detail pane, animated so opening it slides rather than snaps.
    detail: Animated<f32>,
    /// How far the project list is scrolled, in logical pixels.
    scroll: f32,

    /// Owns the async runtime the supervisor spawns onto. Dropping it would kill every
    /// child, so it lives exactly as long as the application does.
    runtime: tokio::runtime::Runtime,
    runner: Arc<ProcessManager>,
    /// Status per project, kept in step by draining `events` each frame.
    statuses: HashMap<String, ProjectStatus>,
    events: std::sync::mpsc::Receiver<RunnerEvent>,

    monitor: Monitor,
    usage: HashMap<String, Sample>,
    last_sample: Instant,

    started: Instant,
    last_frame: Instant,
    /// Set while anything is still moving, so the loop can idle when nothing is.
    needs_frame: bool,
    /// Set by the close button; acted on once the frame it was clicked in has finished.
    closing: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Filter {
    All,
    Local,
    Remote,
    Running,
}

/// A pane of the project detail.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tab {
    Overview,
    Logs,
    Metrics,
    Git,
}

impl Tab {
    const ALL: [Tab; 4] = [Tab::Overview, Tab::Logs, Tab::Metrics, Tab::Git];
    const LABELS: [&'static str; 4] = ["Overview", "Logs", "Metrics", "Git"];
}

/// What the tray asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayCommand {
    TogglePanel,
    ShowWindow,
    Quit,
}

/// What occupies the window. Settings and the scanner take it over entirely rather than
/// floating: at this size a modal over a modal is worse than a screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Screen {
    Projects,
    Settings,
    Scan,
    Form,
}

impl Filter {
    const ALL: [Filter; 4] = [Filter::All, Filter::Local, Filter::Remote, Filter::Running];

    fn label(self) -> &'static str {
        match self {
            Self::All => "All",
            Self::Local => "Local",
            Self::Remote => "Remote",
            Self::Running => "Running",
        }
    }
}

impl Default for Oracle {
    fn default() -> Self {
        let loaded = config::load();
        probe("after config load");

        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .thread_name("oracle")
            .build()
            .expect("Oracle could not start its async runtime");

        // The supervisor reports through a plain closure. Sending into a channel keeps every
        // status change on the interface thread, where the frame that draws it lives.
        let (sender, events) = std::sync::mpsc::channel();
        let runner = Arc::new(ProcessManager::new(
            Arc::new(move |event| {
                let _ = sender.send(event);
            }),
            runtime.handle().clone(),
        ));

        let (tray_sender, tray_events) = std::sync::mpsc::channel();

        Self {
            window: None,
            gpu: None,
            target: None,
            panel_window: None,
            panel_target: None,
            panel_frame: Frame::new(1.0),
            panel_visible: false,
            tray_events,
            tray_sender,
            tray: None,
            palette: match loaded.config.settings.theme {
                Theme::Light => Palette::LIGHT,
                _ => Palette::DARK,
            },
            config: loaded.config,
            scale: 1.0,
            input: Input::default(),
            frame: Frame::new(1.0),
            selected: None,
            query: String::new(),
            filter: Filter::All,
            tab: Tab::Overview,
            screen: Screen::Projects,
            git: HashMap::new(),
            remote: HashMap::new(),
            health: Arc::new(parking_lot::Mutex::new(Vec::new())),
            last_health: Instant::now() - Duration::from_secs(3600),
            http: crate::core::remote::client(),
            draft: None,
            draft_is_new: false,
            measured: HashMap::new(),
            to_measure: Vec::new(),
            candidates: Vec::new(),
            chosen: std::collections::HashSet::new(),
            scanning: false,
            scan_result: Arc::new(parking_lot::Mutex::new(None)),
            detail: Animated::new(0.0)
                .with_motion(Motion::Spring(theme::motion::PANEL))
                .with_epsilon(0.4),
            scroll: 0.0,
            runtime,
            runner,
            statuses: HashMap::new(),
            events,
            monitor: Monitor::new(),
            usage: HashMap::new(),
            last_sample: Instant::now(),
            started: Instant::now(),
            last_frame: Instant::now(),
            needs_frame: true,
            closing: false,
        }
    }
}

impl Oracle {
    /// Projects the current filter and search admit, in display order.
    fn visible(&self) -> Vec<&Project> {
        let query = self.query.trim().to_lowercase();

        let mut projects: Vec<&Project> = self
            .config
            .projects
            .iter()
            .filter(|p| match self.filter {
                Filter::All => true,
                Filter::Local => p.local.is_some(),
                Filter::Remote => p.remote.is_some(),
                Filter::Running => matches!(
                    self.statuses.get(&p.id),
                    Some(ProjectStatus::Running | ProjectStatus::Starting)
                ),
            })
            .filter(|p| {
                query.is_empty()
                    || p.name.to_lowercase().contains(&query)
                    || p.kind.label().to_lowercase().contains(&query)
                    || p.tags.iter().any(|t| t.to_lowercase().contains(&query))
            })
            .collect();

        projects.sort_by(|a, b| match (a.favorite, b.favorite) {
            (true, false) => std::cmp::Ordering::Less,
            (false, true) => std::cmp::Ordering::Greater,
            _ => a.order.cmp(&b.order),
        });

        projects
    }

    fn accent_of(&self, project: &Project) -> Colour {
        project
            .accent
            .as_deref()
            .and_then(parse_hex)
            .unwrap_or_else(|| parse_hex(project.kind.accent()).unwrap_or(self.palette.accent))
    }

    /// Drains status changes and resamples resource usage.
    ///
    /// Both run from the frame rather than on their own timers: the interface is the only
    /// consumer, so there is nothing to gain from updating state nobody is about to draw.
    fn poll(&mut self) {
        while let Ok(event) = self.events.try_recv() {
            if let RunnerEvent::Status { project_id, status } = event {
                self.statuses.insert(project_id, status);
            }
            // Log lines stay in the supervisor's ring buffer; the log pane reads them there.
        }

        // Sampling walks every process on the machine, so it keeps its own once-a-second
        // cadence regardless of how often the window redraws.
        if self.last_sample.elapsed() >= Duration::from_secs(1) {
            self.last_sample = Instant::now();

            let roots: Vec<(String, u32)> = self
                .runner
                .running()
                .into_iter()
                .map(|info| (info.project_id, info.pid))
                .collect();

            self.usage = self
                .monitor
                .sample(&roots)
                .into_iter()
                .map(|usage| (usage.project_id, usage.current))
                .collect();
        }
    }

    fn status_of(&self, project_id: &str) -> ProjectStatus {
        self.statuses
            .get(project_id)
            .copied()
            .unwrap_or(ProjectStatus::Stopped)
    }

    /// Starts a project, or stops it if it is already up.
    fn toggle(&mut self, project_id: &str) {
        let live = matches!(
            self.status_of(project_id),
            ProjectStatus::Running | ProjectStatus::Starting
        );

        if live {
            // Stopping waits on a process tree unwinding, which must not block the frame.
            let runner = self.runner.clone();
            let id = project_id.to_string();
            diag!("stopping {id}, running: {:?}", runner.running());
            self.runtime.spawn(async move {
                match runner.stop(&id).await {
                    Ok(()) => probe("stop returned Ok"),
                    Err(err) => diag!("stop failed: {err}"),
                }
            });
            self.statuses
                .insert(project_id.to_string(), ProjectStatus::Stopped);
            return;
        }

        let Some(project) = self.config.project(project_id).cloned() else {
            return;
        };

        match self.runner.start(&project) {
            Ok(_) => {
                self.statuses
                    .insert(project_id.to_string(), ProjectStatus::Starting);
            }
            Err(err) => {
                self.statuses
                    .insert(project_id.to_string(), ProjectStatus::Crashed);
                eprintln!("Oracle could not start {}: {err}", project.name);
            }
        }
    }

    fn build(&mut self, width: f32, height: f32, dt: f32) {
        self.frame.clear();
        self.poll();
        self.poll_health();
        self.poll_scan();

        self.titlebar(width, dt);

        match self.screen {
            Screen::Settings => {
                self.settings_screen(width, height, dt);
                self.frame
                    .rule(0.0, TITLEBAR, width, false, self.palette.line);
                return;
            }
            Screen::Scan => {
                self.scan_screen(width, height, dt);
                self.frame
                    .rule(0.0, TITLEBAR, width, false, self.palette.line);
                return;
            }
            Screen::Form => {
                self.form_screen(width, height, dt);
                self.frame
                    .rule(0.0, TITLEBAR, width, false, self.palette.line);
                return;
            }
            Screen::Projects => {}
        }

        let palette = self.palette;
        self.detail
            .set_target(if self.selected.is_some() { 380.0 } else { 0.0 });
        self.detail.tick(dt);
        let detail_width = self.detail.get();

        let body_top = TITLEBAR;
        let body_height = height - TITLEBAR;

        self.rail(body_top, body_height, dt);

        let column_x = RAIL;
        let column_width = width - RAIL - detail_width;

        self.toolbar(column_x, body_top, column_width, dt);
        self.list(
            column_x,
            body_top + 52.0,
            column_width,
            body_height - 52.0,
            dt,
        );

        if detail_width > 1.0 {
            self.detail_pane(width - detail_width, body_top, detail_width, body_height, dt);
        }

        // Separators last, so they sit above the panels they divide.
        self.frame
            .rule(0.0, TITLEBAR, width, false, palette.line);
        self.frame
            .rule(RAIL, body_top, body_height, true, palette.line);
    }

    fn titlebar(&mut self, width: f32, dt: f32) {
        let palette = self.palette;

        // The mark, drawn from the same three circles the logo was built from.
        let cx = gap::LG + 9.0;
        let cy = TITLEBAR * 0.5;
        self.frame.dot([cx - 5.0, cy - 5.0], 7.0, palette.accent);
        self.frame.dot([cx + 6.0, cy - 4.0], 5.0, palette.accent);
        self.frame.dot([cx, cy + 4.0], 6.0, palette.accent);

        self.frame.text(
            Run::new("Oracle", gap::LG + 26.0, cy - 9.0, text::BASE, palette.ink).weight(620),
        );

        // Window controls, in the Windows order and at the Windows size.
        let window = self.window.clone();
        let controls: [(&str, &str); 3] = [
            ("minimise", "\u{2500}"),
            ("maximise", "\u{25A1}"),
            ("close", "\u{2715}"),
        ];

        for (index, (key, glyph)) in controls.iter().enumerate() {
            let rect: Rect = [
                width - 46.0 * (3 - index) as f32,
                0.0,
                46.0,
                TITLEBAR - 1.0,
            ];
            let response = self.input.interact(id("window", key), rect, dt);

            if response.hover > 0.01 {
                let tint = if *key == "close" {
                    alpha_of(palette.danger, response.hover)
                } else {
                    alpha_of(palette.line, response.hover)
                };
                self.frame.fill(rect, tint, 0.0);
            }

            let ink = if *key == "close" && response.hover > 0.5 {
                [1.0, 1.0, 1.0, 1.0]
            } else {
                palette.muted
            };
            self.frame
                .centred(*glyph, rect[0] + 23.0, rect[1] + 12.0, text::SM, ink, 400);

            if response.clicked {
                if let Some(window) = window.as_ref() {
                    match *key {
                        "minimise" => window.set_minimized(true),
                        "maximise" => window.set_maximized(!window.is_maximized()),
                        _ => self.closing = true,
                    }
                }
            }
        }

        // Anywhere else on the bar drags the window. Started from the press rather than
        // from a click, so the drag begins on the same frame the button goes down.
        let drag: Rect = [0.0, 0.0, width - 138.0, TITLEBAR];
        if self.input.interact(id("window", "drag"), drag, dt).pressed
            && self.input.just_pressed
        {
            if let Some(window) = window.as_ref() {
                let _ = window.drag_window();
            }
        }
    }

    fn rail(&mut self, top: f32, height: f32, dt: f32) {
        let palette = self.palette;
        let projects: Vec<(String, String, Colour, ProjectStatus)> = self
            .visible()
            .iter()
            .map(|p| {
                (
                    p.id.clone(),
                    initials(&p.name),
                    self.accent_of(p),
                    self.status_of(&p.id),
                )
            })
            .collect();

        let mut y = top + gap::MD;
        for (project_id, mark, accent, status) in projects {
            let rect: Rect = [(RAIL - 40.0) * 0.5, y, 40.0, 40.0];
            let response = self
                .input
                .interact(id("rail", &project_id), rect, dt);

            let selected = self.selected.as_deref() == Some(project_id.as_str());
            if selected {
                self.frame.fill(rect, palette.accent_wash, radius::MD);
            } else if response.hover > 0.01 {
                self.frame
                    .fill(rect, alpha_of(palette.line, response.hover), radius::MD);
            }

            let tile: Rect = [rect[0] + 6.0, rect[1] + 6.0, 28.0, 28.0];
            self.frame.fill(tile, accent, radius::SM);
            self.frame.centred(
                mark,
                tile[0] + 14.0,
                tile[1] + 6.0,
                text::SM,
                [1.0, 1.0, 1.0, 1.0],
                700,
            );

            self.frame.dot(
                [rect[0] + 33.0, rect[1] + 33.0],
                9.0,
                status_colour(status, &palette),
            );

            if response.clicked {
                self.selected = if selected { None } else { Some(project_id.clone()) };
            }

            y += 48.0;
        }

        // Scan and settings sit at the bottom of the rail.
        let bottom = top + height - gap::MD - 88.0;
        let mut go = None;

        for (index, (key, glyph, screen)) in [
            ("rail-scan", "\u{2315}", Screen::Scan),
            ("rail-settings", "\u{2699}", Screen::Settings),
        ]
        .iter()
        .enumerate()
        {
            let rect: Rect = [(RAIL - 40.0) * 0.5, bottom + index as f32 * 44.0, 40.0, 40.0];
            if self.ui(dt).icon_button(key, rect, glyph, false) {
                go = Some(*screen);
            }
        }

        if let Some(screen) = go {
            self.screen = screen;
            self.scroll = 0.0;
            if screen == Screen::Scan && self.candidates.is_empty() {
                self.start_scan();
            }
        }
    }

    fn toolbar(&mut self, x: f32, y: f32, width: f32, dt: f32) {
        let palette = self.palette;
        let inner = x + gap::LG;
        let row_y = y + gap::MD;

        // Search field.
        let filters_width = 240.0;
        let actions_width = 188.0;
        let search: Rect = [
            inner,
            row_y,
            (width - gap::LG * 2.0 - filters_width - actions_width - gap::SM * 2.0).max(120.0),
            34.0,
        ];

        let mut query = self.query.clone();
        let query_width = self.width_of(&query);
        if self
            .ui(dt)
            .field("search", search, &mut query, "Search projects", query_width)
        {
            self.query = query;
            self.scroll = 0.0;
        }

        // Filter segments.
        let mut fx = search[0] + search[2] + gap::SM;
        let seg_rect: Rect = [fx, row_y, filters_width, 34.0];
        self.frame
            .panel(seg_rect, GlassStyle::sunken(&palette), 1.0);

        let seg_width = filters_width / Filter::ALL.len() as f32;
        for filter in Filter::ALL {
            let rect: Rect = [fx + 3.0, row_y + 3.0, seg_width - 6.0, 28.0];
            let response = self.input.interact(id("filter", filter.label()), rect, dt);

            if self.filter == filter {
                self.frame
                    .fill(rect, palette.glass_tint_strong, radius::SM);
            } else if response.hover > 0.01 {
                self.frame
                    .fill(rect, alpha_of(palette.line, response.hover * 0.8), radius::SM);
            }

            self.frame.centred(
                filter.label(),
                rect[0] + rect[2] * 0.5,
                rect[1] + 6.0,
                text::SM,
                if self.filter == filter {
                    palette.ink
                } else {
                    palette.muted
                },
                560,
            );

            if response.clicked {
                self.filter = filter;
            }

            fx += seg_width;
        }

        // Run-everything, scan, and add.
        let ax = seg_rect[0] + seg_rect[2] + gap::SM;
        let anyone_running = self
            .statuses
            .values()
            .any(|s| matches!(s, ProjectStatus::Running | ProjectStatus::Starting));

        let (toggle_all, scan, _add) = {
            let mut ui = self.ui(dt);
            let toggle_all = ui.button(
                "toggle-all",
                [ax, row_y, 56.0, 34.0],
                if anyone_running { "\u{25A0}" } else { "\u{25B6}" },
                Weight::Secondary,
            );
            let scan = ui.button(
                "toolbar-scan",
                [ax + 64.0, row_y, 56.0, 34.0],
                "\u{2315}",
                Weight::Secondary,
            );
            let add = ui.button(
                "toolbar-add",
                [ax + 128.0, row_y, 56.0, 34.0],
                "+",
                Weight::Primary,
            );
            (toggle_all, scan, add)
        };

        if _add {
            self.open_form(None);
        }

        if toggle_all {
            // Start everything local that is idle, or stop everything that is not.
            let ids: Vec<String> = self
                .config
                .projects
                .iter()
                .filter(|p| p.local.is_some())
                .map(|p| p.id.clone())
                .collect();

            for id in ids {
                let live = matches!(
                    self.status_of(&id),
                    ProjectStatus::Running | ProjectStatus::Starting
                );
                if live == anyone_running {
                    self.toggle(&id);
                }
            }
        }
        if scan {
            self.screen = Screen::Scan;
            self.scroll = 0.0;
            if self.candidates.is_empty() {
                self.start_scan();
            }
        }
    }

    fn list(&mut self, x: f32, y: f32, width: f32, height: f32, dt: f32) {
        let palette = self.palette;
        let projects: Vec<Project> = self.visible().into_iter().cloned().collect();

        if projects.is_empty() {
            let cx = x + width * 0.5;
            let cy = y + height * 0.42;

            self.frame.dot([cx - 11.0, cy - 10.0], 15.0, alpha_of(palette.accent, 0.35));
            self.frame.dot([cx + 12.0, cy - 8.0], 11.0, alpha_of(palette.accent, 0.35));
            self.frame.dot([cx, cy + 9.0], 13.0, alpha_of(palette.accent, 0.35));

            self.frame.centred(
                if self.config.projects.is_empty() {
                    "No projects yet"
                } else {
                    "Nothing matches"
                },
                cx,
                cy + 40.0,
                text::MD,
                palette.ink,
                620,
            );
            self.frame.text(
                Run::new(
                    if self.config.projects.is_empty() {
                        "Add a project by hand, or let Oracle scan your folders."
                    } else {
                        "Try a different search or filter."
                    },
                    cx,
                    cy + 66.0,
                    text::BASE,
                    palette.muted,
                )
                .align(Align::Centre),
            );
            return;
        }

        let inner = x + gap::LG;
        let card_width = width - gap::LG * 2.0;

        // Scroll, clamped so the list cannot be flung past its own contents.
        let content = projects.len() as f32 * 70.0;
        let overflow = (content - height + gap::LG).max(0.0);
        self.scroll = (self.scroll - self.input.scroll).clamp(0.0, overflow);

        let mut cy = y - self.scroll;

        for project in &projects {
            // Rows outside the viewport are skipped entirely, so a hundred projects cost
            // the same as the dozen actually on screen.
            if cy + 62.0 < y || cy > y + height {
                cy += 70.0;
                continue;
            }

            let rect: Rect = [inner, cy, card_width, 62.0];
            let response = self.input.interact(id("card", &project.id), rect, dt);
            let selected = self.selected.as_deref() == Some(project.id.as_str());

            let opacity = 1.0 - response.press * 0.25;
            self.frame.panel(rect, GlassStyle::card(&palette), opacity);

            if selected {
                self.frame.fill(
                    [rect[0], rect[1], 3.0, rect[3]],
                    palette.accent,
                    1.5,
                );
            }

            let accent = self.accent_of(project);
            let tile: Rect = [rect[0] + gap::MD, rect[1] + 14.0, 34.0, 34.0];
            self.frame.fill(tile, accent, radius::SM);
            self.frame.centred(
                initials(&project.name),
                tile[0] + 17.0,
                tile[1] + 8.0,
                text::BASE,
                [1.0, 1.0, 1.0, 1.0],
                700,
            );

            let tx = tile[0] + 34.0 + gap::MD;
            self.frame.text(
                Run::new(&project.name, tx, rect[1] + 13.0, text::BASE, palette.ink).weight(600),
            );

            let status = self.status_of(&project.id);
            let live = matches!(status, ProjectStatus::Running | ProjectStatus::Starting);

            // A running project reports what it costs; a stopped one reports what it is.
            let meta = match (live, self.usage.get(&project.id)) {
                (true, Some(sample)) => format!(
                    "{} · {} · {}",
                    status_label(status),
                    percent(sample.cpu),
                    bytes(sample.memory)
                ),
                (true, None) => status_label(status).to_string(),
                _ => match (&project.local, &project.remote) {
                    (Some(local), _) => format!(
                        "{} · {}",
                        project.kind.label(),
                        if local.command.is_empty() {
                            "no command"
                        } else {
                            &local.command
                        }
                    ),
                    (None, Some(remote)) => format!("{} · {}", project.kind.label(), remote.url),
                    _ => project.kind.label().to_string(),
                },
            };
            self.frame
                .label(meta, tx, rect[1] + 33.0, text::SM, palette.muted);

            self.frame.dot(
                [rect[0] + rect[2] - 62.0, rect[1] + 31.0],
                8.0,
                status_colour(status, &palette),
            );

            // Play and stop share one control, as they did on the web.
            let play: Rect = [rect[0] + rect[2] - 42.0, rect[1] + 16.0, 30.0, 30.0];
            let play_response = self.input.interact(id("play", &project.id), play, dt);
            let lit = play_response.hover > 0.01 || live;

            self.frame.fill(
                play,
                if lit {
                    if live {
                        palette.accent
                    } else {
                        palette.accent_bright
                    }
                } else {
                    alpha_of(palette.accent, 0.18)
                },
                radius::SM,
            );
            self.frame.centred(
                if live { "\u{25A0}" } else { "\u{25B6}" },
                play[0] + 15.0,
                play[1] + 6.0,
                text::SM,
                if lit { [1.0, 1.0, 1.0, 1.0] } else { palette.accent },
                500,
            );

            if play_response.clicked {
                let project_id = project.id.clone();
                diag!("play clicked on {}", project.name);
                self.toggle(&project_id);
            }

            if response.clicked {
                self.selected = if selected { None } else { Some(project.id.clone()) };
            }

            cy += 70.0;
        }
    }

    fn detail_pane(&mut self, x: f32, y: f32, width: f32, height: f32, dt: f32) {
        let palette = self.palette;
        let Some(selected) = self.selected.clone() else {
            return;
        };
        let Some(project) = self.config.project(&selected).cloned() else {
            return;
        };

        self.frame.rule(x, y, height, true, palette.line);

        let inner = x + gap::LG;
        let content_width = width - gap::LG * 2.0;
        let accent = self.accent_of(&project);
        let status = self.status_of(&project.id);

        // Header.
        let tile: Rect = [inner, y + gap::LG, 34.0, 34.0];
        self.frame.fill(tile, accent, radius::SM);
        self.frame.centred(
            initials(&project.name),
            tile[0] + 17.0,
            tile[1] + 8.0,
            text::BASE,
            [1.0, 1.0, 1.0, 1.0],
            700,
        );

        self.frame.text(
            Run::new(&project.name, inner + 46.0, y + gap::LG + 1.0, text::MD, palette.ink)
                .weight(640)
                .width(content_width - 100.0),
        );
        self.frame.label(
            status_label(status),
            inner + 46.0,
            y + gap::LG + 22.0,
            text::SM,
            palette.muted,
        );

        let close: Rect = [x + width - 44.0, y + gap::LG, 30.0, 30.0];
        let tabs_y = y + 68.0;
        let selected_tab = Tab::ALL.iter().position(|t| *t == self.tab).unwrap_or(0);

        let edit: Rect = [close[0] - 34.0, close[1], 30.0, 30.0];
        let (closed, edit_clicked, picked) = {
            let mut ui = self.ui(dt);
            let closed = ui.icon_button("detail-close", close, "\u{2715}", false);
            let edit_clicked = ui.icon_button("detail-edit", edit, "\u{270E}", false);
            let picked = ui.tabs("tab", inner, tabs_y, &Tab::LABELS, selected_tab);
            (closed, edit_clicked, picked)
        };

        if edit_clicked {
            self.open_form(Some(&project));
            return;
        }

        if closed {
            self.selected = None;
            return;
        }
        if let Some(index) = picked {
            self.tab = Tab::ALL[index];
            // Git shells out, so it is read when the pane is opened rather than polled.
            if self.tab == Tab::Git {
                self.read_git(&project);
            }
        }

        self.frame
            .rule(x, tabs_y + 36.0, width, false, palette.line);

        let body_y = tabs_y + 36.0 + gap::LG;
        let body_height = height - (body_y - y) - gap::LG;

        match self.tab {
            Tab::Overview => self.overview_tab(&project, inner, body_y, content_width, dt),
            Tab::Logs => self.logs_tab(&project, inner, body_y, content_width, body_height),
            Tab::Metrics => self.metrics_tab(&project, inner, body_y, content_width),
            Tab::Git => self.git_tab(&project, inner, body_y, content_width),
        }
    }

    fn overview_tab(&mut self, project: &Project, x: f32, y: f32, width: f32, dt: f32) {
        let status = self.status_of(&project.id);
        let live = matches!(status, ProjectStatus::Running | ProjectStatus::Starting);
        let url = open_url(project);

        // Actions.
        let mut actions: Vec<(&str, String)> = Vec::new();
        if project.local.is_some() {
            actions.push(("toggle", if live { "Stop" } else { "Start" }.to_string()));
        }
        if url.is_some() {
            actions.push(("open", "Open".to_string()));
        }
        if project.local.is_some() {
            actions.push(("folder", "Folder".to_string()));
        }

        let mut clicked = None;
        {
            let mut ui = self.ui(dt);
            let mut cursor = x;
            for (key, label) in &actions {
                let rect: Rect = [cursor, y, 86.0, 30.0];
                let weight = if *key == "toggle" && !live {
                    Weight::Primary
                } else {
                    Weight::Secondary
                };
                if ui.button(key, rect, label, weight) {
                    clicked = Some(*key);
                }
                cursor += 94.0;
            }
        }

        match clicked {
            Some("toggle") => {
                let id = project.id.clone();
                self.toggle(&id);
            }
            Some("open") => {
                if let Some(url) = url.clone() {
                    open_in_browser(&url);
                }
            }
            Some("folder") => {
                if let Some(local) = &project.local {
                    reveal(&local.root);
                }
            }
            _ => {}
        }

        // Facts.
        let mut row_y = y + 46.0;
        let mut rows: Vec<(String, String)> = vec![("Type".into(), project.kind.label().into())];

        if let Some(local) = &project.local {
            rows.push(("Folder".into(), short_path(&local.root.to_string_lossy())));
            rows.push((
                "Command".into(),
                if local.command.is_empty() {
                    "not set".into()
                } else {
                    local.command.clone()
                },
            ));
            if let Some(port) = local.port {
                rows.push(("Port".into(), port.to_string()));
            }
            if !local.env.is_empty() {
                rows.push((
                    "Environment".into(),
                    format!("{} variables", local.env.len()),
                ));
            }
        }
        if let Some(remote) = &project.remote {
            rows.push(("Health check".into(), remote.url.clone()));
            rows.push((
                "Remote".into(),
                describe_remote(self.remote.get(&project.id)),
            ));
        }
        if let Some(repo) = &project.repo {
            if let Some(slug) = &repo.slug {
                rows.push(("Repository".into(), slug.clone()));
            }
        }
        if let Some(pid) = self.runner.pid(&project.id) {
            rows.push(("Process id".into(), pid.to_string()));
        }

        let mut ui = self.ui(dt);
        for (label, value) in &rows {
            ui.row(x, row_y, width, label, value);
            row_y += 26.0;
        }
    }

    fn logs_tab(&mut self, project: &Project, x: f32, y: f32, width: f32, height: f32) {
        let palette = self.palette;
        let lines = self.runner.logs(&project.id, None);

        if lines.is_empty() {
            let message = if matches!(self.status_of(&project.id), ProjectStatus::Stopped) {
                "Nothing yet. Start the project to see its output."
            } else {
                "Waiting for output…"
            };
            let mut ui = self.ui(0.0);
            ui.placeholder(x, y + 20.0, width, message);
            return;
        }

        // The last screenful, newest at the bottom. A log pane that does not follow the tail
        // is a log pane nobody reads.
        let row_height = 17.0;
        let visible = ((height / row_height).floor() as usize).max(1);
        let start = lines.len().saturating_sub(visible);

        for (index, line) in lines[start..].iter().enumerate() {
            let ly = y + index as f32 * row_height;
            let ink = match line.stream {
                crate::core::runner::logs::Stream::Stderr => palette.danger,
                crate::core::runner::logs::Stream::System => palette.accent,
                _ => palette.ink_soft,
            };

            self.frame.text(
                Run::new(format!("{:04}", line.seq), x, ly, text::SM, palette.muted).monospace(),
            );
            self.frame.text(
                Run::new(&line.text, x + 40.0, ly, text::SM, ink)
                    .monospace()
                    .clip([x, y, width, height]),
            );
        }
    }

    fn metrics_tab(&mut self, project: &Project, x: f32, y: f32, width: f32) {
        let palette = self.palette;
        let Some(sample) = self.usage.get(&project.id).copied() else {
            let mut ui = self.ui(0.0);
            ui.placeholder(
                x,
                y + 20.0,
                width,
                "No measurements. Metrics are collected while a project runs.",
            );
            return;
        };

        let gauge_width = (width - gap::MD) * 0.5;
        for (index, (label, value)) in [
            ("CPU across the process tree", percent(sample.cpu)),
            ("Memory", bytes(sample.memory)),
        ]
        .iter()
        .enumerate()
        {
            let rect: Rect = [x + (gauge_width + gap::MD) * index as f32, y, gauge_width, 84.0];
            self.frame
                .panel(rect, GlassStyle::sunken(&palette), 1.0);
            self.frame.text(
                Run::new(value, rect[0] + gap::MD, rect[1] + 14.0, text::XL, palette.ink)
                    .weight(640),
            );
            self.frame.text(
                Run::new(*label, rect[0] + gap::MD, rect[1] + 52.0, text::SM, palette.muted)
                    .width(gauge_width - gap::MD * 2.0),
            );
        }

        let mut ui = self.ui(0.0);
        ui.row(x, y + 104.0, width, "Processes", &sample.processes.to_string());
        ui.heading(x, y + 140.0, "How this is measured");
        ui.placeholder(
            x,
            y + 160.0,
            width,
            "CPU is percent of one core, summed over every descendant, so a project using four cores reads 400%.",
        );
    }

    fn git_tab(&mut self, project: &Project, x: f32, y: f32, width: f32) {
        let status = self.git.get(&project.id).cloned();

        let mut ui = self.ui(0.0);
        match status {
            None => ui.placeholder(x, y + 20.0, width, "Reading…"),
            Some(None) => ui.placeholder(
                x,
                y + 20.0,
                width,
                if project.local.is_some() {
                    "This folder is not a git repository."
                } else {
                    "This project has no local folder to inspect."
                },
            ),
            Some(Some(git)) => {
                let mut row_y = y;
                ui.row(
                    x,
                    row_y,
                    width,
                    "Branch",
                    git.branch.as_deref().unwrap_or("detached HEAD"),
                );
                row_y += 26.0;

                if let Some(upstream) = &git.upstream {
                    ui.row(x, row_y, width, "Upstream", upstream);
                    row_y += 26.0;
                }

                let sync = if git.ahead > 0 || git.behind > 0 {
                    format!("{} ahead, {} behind", git.ahead, git.behind)
                } else if git.upstream.is_some() {
                    "In sync".to_string()
                } else {
                    "No upstream".to_string()
                };
                ui.row(x, row_y, width, "Sync", &sync);
                row_y += 26.0;

                let worktree = if git.dirty_files == 0 {
                    "Clean".to_string()
                } else {
                    format!(
                        "{} changed file{}",
                        git.dirty_files,
                        if git.dirty_files == 1 { "" } else { "s" }
                    )
                };
                ui.row(x, row_y, width, "Worktree", &worktree);
                row_y += 26.0;

                if let Some(commit) = &git.last_commit {
                    ui.row(x, row_y, width, "Last commit", &commit.hash);
                    row_y += 26.0;
                    ui.row(x, row_y, width, "Message", &commit.subject);
                    row_y += 26.0;
                    ui.row(x, row_y, width, "Author", &commit.author);
                }
            }
        }
    }

    /// Reads git status for a project, once, when its pane is opened.
    fn read_git(&mut self, project: &Project) {
        let status = project
            .local
            .as_ref()
            .and_then(|local| crate::core::vcs::status(&local.root).ok().flatten());
        self.git.insert(project.id.clone(), status);
    }
}

impl ApplicationHandler for Oracle {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }

        // Fit the preferred size to the display rather than assuming it fits.
        //
        // 1240x820 logical is 1860x1230 physical on a 150% display, which is taller than a
        // 1080p screen — the window opened with its own controls off the bottom right. The
        // margin leaves room for the taskbar.
        let (preferred_width, preferred_height) = event_loop
            .primary_monitor()
            .map(|monitor| {
                let scale = monitor.scale_factor();
                let size = monitor.size();
                // The vertical margin is larger than the horizontal one because the taskbar
                // takes a strip off the bottom that `monitor.size()` still counts. Without
                // it the window's own footer — Save, Cancel — sits behind the taskbar and
                // cannot be clicked.
                (
                    (size.width as f64 / scale - 120.0).min(1240.0).max(900.0),
                    (size.height as f64 / scale - 200.0).min(820.0).max(560.0),
                )
            })
            .unwrap_or((1100.0, 740.0));

        let attributes = Window::default_attributes()
            .with_title("Oracle")
            .with_inner_size(winit::dpi::LogicalSize::new(
                preferred_width,
                preferred_height,
            ))
            .with_min_inner_size(winit::dpi::LogicalSize::new(860.0, 560.0))
            // Oracle draws its own title bar, so the system must not draw one too.
            .with_decorations(false)
            // The window is transparent so the rounded corners of the shell show the
            // desktop rather than a square of paper.
            .with_transparent(true);

        let window = match event_loop.create_window(attributes) {
            Ok(window) => Arc::new(window),
            Err(err) => {
                eprintln!("Oracle could not open a window: {err}");
                event_loop.exit();
                return;
            }
        };

        self.scale = window.scale_factor() as f32;
        self.frame.set_scale(self.scale);
        self.panel_frame.set_scale(self.scale);
        probe("after window");

        let (gpu, mut target) = match pollster::block_on(Gpu::new(window.clone(), self.scale)) {
            Ok(pair) => pair,
            Err(err) => {
                eprintln!("Oracle could not start the renderer: {err}");
                event_loop.exit();
                return;
            }
        };
        target.set_blur(&gpu, Blur::Standard);

        // The tray panel. Built now rather than on first use: a surface and a glyph atlas
        // take long enough that creating them on the click would be visible as a stall.
        let panel_attributes = Window::default_attributes()
            .with_title("Oracle")
            .with_inner_size(winit::dpi::LogicalSize::new(PANEL_WIDTH, PANEL_HEIGHT))
            .with_decorations(false)
            .with_transparent(true)
            .with_resizable(false)
            // It behaves like a popover, not a second application.
            .with_skip_taskbar(true)
            .with_window_level(winit::window::WindowLevel::AlwaysOnTop)
            .with_visible(false);

        match event_loop.create_window(panel_attributes) {
            Ok(panel) => {
                let panel = Arc::new(panel);
                match gpu.attach(panel.clone(), self.scale) {
                    Ok(mut panel_target) => {
                        panel_target.set_blur(&gpu, Blur::Standard);
                        self.panel_target = Some(panel_target);
                        self.panel_window = Some(panel);
                    }
                    Err(err) => eprintln!("Oracle could not prepare the tray panel: {err}"),
                }
            }
            Err(err) => eprintln!("Oracle could not open the tray panel: {err}"),
        }

        self.gpu = Some(gpu);
        self.target = Some(target);
        probe("after renderer");

        self.build_tray();
        self.window = Some(window);
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        // The panel is a separate window with its own frame and its own surface, so its
        // events must not be mistaken for the application's.
        if self.panel_window.as_ref().is_some_and(|w| w.id() == id) {
            self.panel_event(event);
            return;
        }

        match event {
            // Closing the window sends Oracle to the tray when the setting says so;
            // quitting is explicit, from the tray menu or the panel.
            WindowEvent::CloseRequested => {
                if self.config.settings.minimise_to_tray {
                    if let Some(window) = self.window.as_ref() {
                        window.set_visible(false);
                    }
                } else {
                    self.shutdown(event_loop);
                }
            }

            WindowEvent::Resized(size) => {
                if let (Some(gpu), Some(target)) = (self.gpu.as_ref(), self.target.as_mut()) {
                    target.resize(gpu, size.width, size.height);
                }
                self.needs_frame = true;
            }

            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                self.scale = scale_factor as f32;
                self.frame.set_scale(self.scale);
                if let Some(target) = self.target.as_mut() {
                    target.text.set_scale(self.scale);
                }
                if let Some(panel) = self.panel_target.as_mut() {
                    panel.text.set_scale(self.scale);
                }
                self.panel_frame.set_scale(self.scale);
                self.needs_frame = true;
            }

            WindowEvent::CursorMoved { position, .. } => {
                self.input.pointer = [
                    position.x as f32 / self.scale,
                    position.y as f32 / self.scale,
                ];
                self.input.pointer_in_window = true;
                self.needs_frame = true;
            }

            WindowEvent::CursorLeft { .. } => {
                self.input.pointer_in_window = false;
                self.needs_frame = true;
            }

            WindowEvent::MouseInput { state, button, .. } if button == MouseButton::Left => {
                diag!(
                    "mouse {state:?} at {:.0},{:.0} in_window={}",
                    self.input.pointer[0], self.input.pointer[1], self.input.pointer_in_window
                );
                match state {
                    ElementState::Pressed => {
                        self.input.down = true;
                        self.input.just_pressed = true;
                    }
                    ElementState::Released => {
                        self.input.down = false;
                        self.input.just_released = true;
                    }
                }
                self.needs_frame = true;
            }

            WindowEvent::MouseWheel { delta, .. } => {
                self.input.scroll += match delta {
                    MouseScrollDelta::LineDelta(_, y) => y * 40.0,
                    MouseScrollDelta::PixelDelta(p) => p.y as f32,
                };
                self.needs_frame = true;
            }

            WindowEvent::KeyboardInput { event, .. }
                if event.state == ElementState::Pressed =>
            {
                match &event.logical_key {
                    WinitKey::Named(NamedKey::Escape) => {
                        self.input.keys.push(Key::Escape);
                        // Back out one level at a time: focus, then screen, then selection.
                        if self.input.focus.is_some() {
                            self.input.blur();
                        } else if self.screen != Screen::Projects {
                            self.screen = Screen::Projects;
                            self.draft = None;
                            self.scroll = 0.0;
                        } else if self.selected.is_some() {
                            self.selected = None;
                        }
                    }
                    WinitKey::Named(NamedKey::Backspace) => {
                        self.input.keys.push(Key::Backspace);
                    }
                    WinitKey::Named(NamedKey::Enter) => self.input.keys.push(Key::Enter),
                    WinitKey::Named(NamedKey::Tab) => self.input.keys.push(Key::Tab),
                    WinitKey::Named(NamedKey::Space) => self.input.typed.push(' '),
                    WinitKey::Character(c) => self.input.typed.push_str(c.as_str()),
                    _ => {}
                }
                self.needs_frame = true;
            }

            WindowEvent::RedrawRequested => {
                let now = Instant::now();
                let dt = now.duration_since(self.last_frame).as_secs_f32().min(0.1);
                self.last_frame = now;

                let Some(target) = self.target.as_mut() else {
                    return;
                };

                let (pw, ph) = target.size();
                let (width, height) = (pw as f32 / self.scale, ph as f32 / self.scale);

                self.build(width, height, dt);

                let elapsed = self.started.elapsed().as_secs_f32();
                let palette = self.palette;
                if let (Some(gpu), Some(target)) = (self.gpu.as_ref(), self.target.as_mut()) {
                    target.render(gpu, &self.frame, &palette, elapsed);

                    // Strings the layout pass asked about. Measured here because this is the
                    // only place the text engine is reachable, and cached because shaping is
                    // the expensive half of drawing text.
                    for value in self.to_measure.drain(..) {
                        // Shaped at the logical size, so the result is already in the units
                        // the layout works in. Dividing by the scale here would shrink the
                        // caret's travel by a third on a 150% display.
                        let width = target.text.measure(&value, text::BASE, 400, false);
                        self.measured.insert(value, width);
                    }
                    if self.measured.len() > 256 {
                        self.measured.clear();
                    }
                }

                // Another frame is needed only while something is still moving. The wash
                // drifts, so this is effectively always true — but the structure is here so
                // that switching the wash off makes the app genuinely idle.
                self.needs_frame = self.input.animating() || !self.detail.is_settled() || true;
                self.input.end_frame();

                if self.closing {
                    self.closing = false;
                    if self.config.settings.minimise_to_tray {
                        if let Some(window) = self.window.as_ref() {
                            window.set_visible(false);
                        }
                    } else {
                        self.shutdown(event_loop);
                    }
                }
            }

            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        self.poll_tray(event_loop);

        if let Some(window) = self.window.as_ref() {
            if self.needs_frame {
                window.request_redraw();
                event_loop.set_control_flow(ControlFlow::Poll);
            } else {
                event_loop.set_control_flow(ControlFlow::Wait);
            }
        }

        if self.panel_visible {
            if let Some(panel) = self.panel_window.as_ref() {
                panel.request_redraw();
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Small helpers
// ---------------------------------------------------------------------------

fn status_colour(status: ProjectStatus, palette: &Palette) -> Colour {
    match status {
        ProjectStatus::Running => palette.accent,
        ProjectStatus::Starting | ProjectStatus::Unhealthy => palette.warn,
        ProjectStatus::Crashed => palette.danger,
        ProjectStatus::Stopped => palette.idle,
    }
}

fn status_label(status: ProjectStatus) -> &'static str {
    match status {
        ProjectStatus::Running => "Running",
        ProjectStatus::Starting => "Starting",
        ProjectStatus::Unhealthy => "Not responding",
        ProjectStatus::Crashed => "Crashed",
        ProjectStatus::Stopped => "Stopped",
    }
}

/// CPU as a percentage of one core, so a project pinning four reads 400%.
fn percent(value: f32) -> String {
    if value < 10.0 {
        format!("{value:.1}%")
    } else {
        format!("{}%", value.round())
    }
}

/// Binary units, because that is what Task Manager reports and a mismatch reads as a bug.
fn bytes(value: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut scaled = value as f64;
    let mut unit = 0;

    while scaled >= 1024.0 && unit < UNITS.len() - 1 {
        scaled /= 1024.0;
        unit += 1;
    }

    if scaled >= 100.0 || unit <= 1 {
        format!("{scaled:.0} {}", UNITS[unit])
    } else {
        format!("{scaled:.1} {}", UNITS[unit])
    }
}

fn alpha_of(colour: Colour, a: f32) -> Colour {
    [colour[0], colour[1], colour[2], colour[3] * a]
}

fn initials(name: &str) -> String {
    let words: Vec<&str> = name
        .split(|c: char| c.is_whitespace() || c == '-' || c == '_' || c == '.')
        .filter(|w| !w.is_empty())
        .collect();

    match words.len() {
        0 => "?".to_string(),
        1 => words[0].chars().take(2).collect::<String>().to_uppercase(),
        _ => {
            let mut out = String::new();
            out.extend(words[0].chars().take(1));
            out.extend(words[1].chars().take(1));
            out.to_uppercase()
        }
    }
}

fn short_path(path: &str) -> String {
    let parts: Vec<&str> = path
        .replace('\\', "/")
        .split('/')
        .filter(|p| !p.is_empty())
        .map(|p| p.to_string())
        .collect::<Vec<String>>()
        .iter()
        .map(|s| Box::leak(s.clone().into_boxed_str()) as &str)
        .collect();

    if parts.len() <= 2 {
        path.to_string()
    } else {
        format!("…/{}", parts[parts.len() - 2..].join("/"))
    }
}

fn parse_hex(value: &str) -> Option<Colour> {
    let hex = value.trim_start_matches('#');
    if hex.len() != 6 {
        return None;
    }
    let n = u32::from_str_radix(hex, 16).ok()?;
    Some(theme::hex(n))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initials_use_the_first_two_words() {
        assert_eq!(initials("Junior Manage"), "JM");
        assert_eq!(initials("app-tiktok"), "AT");
        assert_eq!(initials("oracle"), "OR");
        assert_eq!(initials(""), "?");
    }

    #[test]
    fn hex_colours_from_the_config_are_parsed() {
        let accent = parse_hex("#C15F3C").expect("should parse");
        assert!((accent[0] - 193.0 / 255.0).abs() < 1e-5);

        assert!(parse_hex("nonsense").is_none());
        assert!(parse_hex("#FFF").is_none(), "short form is not supported");
    }

    #[test]
    fn a_long_path_is_shortened_to_its_last_two_segments() {
        assert_eq!(
            short_path("C:\\Users\\alici\\Documents\\AppTiktok"),
            "…/Documents/AppTiktok"
        );
        assert_eq!(short_path("C:/one"), "C:/one");
    }
}

/// Formats and prints a diagnostic only when `ORACLE_DIAG` is set, so the formatting cost
/// is not paid on every frame of a normal run.
#[macro_export]
macro_rules! diag {
    ($($arg:tt)*) => {
        if $crate::app::diagnostics() {
            $crate::app::probe(&format!($($arg)*));
        }
    };
}

/// Whether diagnostics are on. Read once: the environment does not change under us.
pub fn diagnostics() -> bool {
    use std::sync::OnceLock;
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("ORACLE_DIAG").is_some())
}

/// Prints a stage with this process's resident memory beside it.
///
/// Set `ORACLE_DIAG=1` to see it. Kept rather than deleted because it is what found the
/// 307 MB the graphics driver reserves at device creation — a number no amount of reading
/// the code would have revealed — and the next surprise will want the same tool.
pub fn probe(stage: &str) {
    if !diagnostics() {
        return;
    }

    use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};

    let mut system = System::new();
    let pid = Pid::from_u32(std::process::id());
    system.refresh_processes_specifics(
        ProcessesToUpdate::Some(&[pid]),
        true,
        ProcessRefreshKind::nothing().with_memory(),
    );

    if let Some(process) = system.process(pid) {
        eprintln!(
            "[diag] {:<20} {:>7.1} MB",
            stage,
            process.memory() as f64 / 1_048_576.0
        );
    }
}

impl Oracle {
    /// Borrows the frame and the input together, which is what a control needs.
    fn ui(&mut self, dt: f32) -> Ui<'_> {
        Ui {
            frame: &mut self.frame,
            input: &mut self.input,
            palette: self.palette,
            dt,
        }
    }

    /// Full-window settings.
    fn settings_screen(&mut self, width: f32, height: f32, dt: f32) {
        let palette = self.palette;
        let settings = self.config.settings.clone();

        let inner = gap::XL;
        let content = (width - gap::XL * 2.0).min(620.0);

        self.frame.text(
            Run::new("Settings", inner, TITLEBAR + gap::XL, text::LG, palette.ink).weight(640),
        );

        let back: Rect = [width - 44.0 - gap::XL, TITLEBAR + gap::XL, 30.0, 30.0];
        let mut changed: Option<Settings> = None;
        let leave;
        let theme_pick;
        let glass_pick;
        // Carried out of the borrow below rather than recomputed from a sum of constants,
        // which is how the version label ended up on top of the Add button.
        let mut cursor;

        {
            let mut ui = self.ui(dt);
            leave = ui.icon_button("settings-close", back, "\u{2715}", false);

            let mut y = TITLEBAR + 84.0;

            ui.heading(inner, y, "Appearance");
            y += 26.0;
            theme_pick = ui.segmented(
                "theme",
                [inner, y, 240.0, 32.0],
                &["Light", "Dark", "System"],
                match settings.theme {
                    Theme::Light => 0,
                    Theme::Dark => 1,
                    Theme::System => 2,
                },
            );
            y += 48.0;

            glass_pick = ui.segmented(
                "glass",
                [inner, y, 240.0, 32.0],
                &["Full", "Reduced", "Opaque"],
                match settings.glass {
                    GlassLevel::Full => 0,
                    GlassLevel::Reduced => 1,
                    GlassLevel::Opaque => 2,
                },
            );
            y += 56.0;

            ui.heading(inner, y, "Startup");
            y += 26.0;

            let mut next = settings.clone();
            let mut touched = false;

            if let Some(value) = ui.toggle(
                "start-windows",
                inner,
                y,
                content,
                "Start with Windows",
                "Registers Oracle in the startup list. No administrator rights needed.",
                settings.start_with_windows,
            ) {
                next.start_with_windows = value;
                touched = true;
            }
            y += 54.0;

            if let Some(value) = ui.toggle(
                "start-hidden",
                inner,
                y,
                content,
                "Start hidden in the tray",
                "Comes up as a tray icon only, without opening the window.",
                settings.start_hidden,
            ) {
                next.start_hidden = value;
                touched = true;
            }
            y += 54.0;

            if let Some(value) = ui.toggle(
                "autostart-projects",
                inner,
                y,
                content,
                "Launch flagged projects",
                "Starts every project marked to run when Oracle starts.",
                settings.autostart_projects,
            ) {
                next.autostart_projects = value;
                touched = true;
            }
            y += 54.0;

            if let Some(value) = ui.toggle(
                "minimise-tray",
                inner,
                y,
                content,
                "Close to tray",
                "Closing the window keeps Oracle running.",
                settings.minimise_to_tray,
            ) {
                next.minimise_to_tray = value;
                touched = true;
            }
            y += 70.0;

            ui.heading(inner, y, "Folders to scan");
            cursor = y + 26.0;

            if touched {
                changed = Some(next);
            }
        }

        // Scan roots are drawn outside the `Ui` borrow, because removing one mutates config.
        let mut remove = None;
        let mut y = cursor;
        let roots = settings.scan_roots.clone();

        if roots.is_empty() {
            self.ui(dt).placeholder(
                inner,
                y,
                content,
                "No folders yet. Add the directories your projects live in.",
            );
            y += 30.0;
        }

        for (index, root) in roots.iter().enumerate() {
            let label = root.to_string_lossy().to_string();
            self.frame
                .label(label, inner, y + 6.0, text::BASE, palette.ink_soft);

            let bin: Rect = [inner + content - 30.0, y, 30.0, 30.0];
            if self
                .ui(dt)
                .icon_button(&format!("root{index}"), bin, "\u{2715}", true)
            {
                remove = Some(index);
            }
            self.frame
                .rule(inner, y + 36.0, content, false, palette.line);
            y += 44.0;
        }

        let add: Rect = [inner, y + gap::SM, 160.0, 32.0];
        let add_clicked = self
            .ui(dt)
            .button("add-root", add, "Add Documents", Weight::Secondary);

        self.frame.label(
            format!("Oracle {}", env!("CARGO_PKG_VERSION")),
            inner,
            (add[1] + add[3] + gap::XL).max(height - 34.0),
            text::SM,
            palette.muted,
        );

        if let Some(index) = theme_pick {
            let mut next = self.config.settings.clone();
            next.theme = [Theme::Light, Theme::Dark, Theme::System][index];
            self.apply_settings(next);
        }
        if let Some(index) = glass_pick {
            let mut next = self.config.settings.clone();
            next.glass = [GlassLevel::Full, GlassLevel::Reduced, GlassLevel::Opaque][index];
            self.apply_settings(next);
        }
        if let Some(next) = changed {
            self.apply_settings(next);
        }
        if let Some(index) = remove {
            let mut next = self.config.settings.clone();
            next.scan_roots.remove(index);
            self.apply_settings(next);
        }
        if add_clicked {
            if let Some(home) = dirs::home_dir() {
                let documents = home.join("Documents");
                if documents.is_dir() {
                    let mut next = self.config.settings.clone();
                    if !next.scan_roots.contains(&documents) {
                        next.scan_roots.push(documents);
                        self.apply_settings(next);
                    }
                }
            }
        }
        if leave {
            self.screen = Screen::Projects;
            self.scroll = 0.0;
        }
    }

    /// Applies a settings change and writes it straight to disk.
    ///
    /// No save button: every control here takes effect the moment it is touched, and the
    /// config is small enough that persisting on each change costs nothing.
    fn apply_settings(&mut self, settings: Settings) {
        let theme_changed = settings.theme != self.config.settings.theme;
        let glass_changed = settings.glass != self.config.settings.glass;
        let autostart_changed = settings.start_with_windows
            != self.config.settings.start_with_windows
            || settings.start_hidden != self.config.settings.start_hidden;

        self.config.settings = settings;

        if theme_changed {
            self.palette = match self.config.settings.theme {
                Theme::Light => Palette::LIGHT,
                _ => Palette::DARK,
            };
        }
        if glass_changed {
            let blur = match self.config.settings.glass {
                GlassLevel::Full => Blur::Standard,
                GlassLevel::Reduced => Blur::Light,
                GlassLevel::Opaque => Blur::None,
            };
            if let Some(gpu) = self.gpu.as_ref() {
                if let Some(target) = self.target.as_mut() {
                    target.set_blur(gpu, blur);
                }
                if let Some(panel) = self.panel_target.as_mut() {
                    panel.set_blur(gpu, blur);
                }
            }
        }
        if autostart_changed {
            if let Err(err) = crate::core::autostart::set(
                self.config.settings.start_with_windows,
                self.config.settings.start_hidden,
            ) {
                eprintln!("Oracle could not change the startup entry: {err}");
            }
        }

        if let Err(err) = config::save(&self.config) {
            eprintln!("Oracle could not save the settings: {err}");
        }
    }

    /// Full-window folder scan.
    fn scan_screen(&mut self, width: f32, height: f32, dt: f32) {
        let palette = self.palette;
        let inner = gap::XL;
        let content = width - gap::XL * 2.0;

        self.frame.text(
            Run::new("Find projects", inner, TITLEBAR + gap::XL, text::LG, palette.ink)
                .weight(640),
        );

        let back: Rect = [width - 44.0 - gap::XL, TITLEBAR + gap::XL, 30.0, 30.0];
        let leave = self.ui(dt).icon_button("scan-close", back, "\u{2715}", false);

        let list_y = TITLEBAR + 84.0;

        if self.scanning {
            self.ui(dt).placeholder(inner, list_y, content, "Scanning…");
        } else if self.config.settings.scan_roots.is_empty() {
            self.ui(dt).placeholder(
                inner,
                list_y,
                content,
                "No folders are set to be scanned. Add one under Settings, then rescan.",
            );
        } else if self.candidates.is_empty() {
            self.ui(dt).placeholder(
                inner,
                list_y,
                content,
                "Nothing found in the folders Oracle scans.",
            );
        } else {
            let summary = format!(
                "Found {} project{}. Pick the ones to add.",
                self.candidates.len(),
                if self.candidates.len() == 1 { "" } else { "s" }
            );
            self.frame
                .label(summary, inner, list_y, text::SM, palette.muted);

            let rows: Vec<(String, String, String, bool)> = self
                .candidates
                .iter()
                .map(|c| {
                    (
                        c.root.to_string_lossy().to_string(),
                        c.name.clone(),
                        format!(
                            "{} · {}",
                            short_path(&c.root.to_string_lossy()),
                            if c.suggested_command.is_empty() {
                                "no command"
                            } else {
                                c.suggested_command.as_str()
                            }
                        ),
                        c.already_known,
                    )
                })
                .collect();

            let top = list_y + 28.0;
            let viewport = height - 90.0 - top;

            // Twenty-eight candidates do not fit on one screen; without this only the first
            // nine were reachable and the rest were silently unimportable.
            let overflow = (rows.len() as f32 * 50.0 - viewport).max(0.0);
            self.scroll = (self.scroll - self.input.scroll).clamp(0.0, overflow);

            let mut y = top - self.scroll;
            let mut toggled = None;

            for (key, name, detail, known) in rows {
                if y + 44.0 < top {
                    y += 50.0;
                    continue;
                }
                if y > top + viewport {
                    break;
                }

                let rect: Rect = [inner, y, content, 44.0];
                let ticked = self.chosen.contains(&key);
                let response = self.input.interact(id("cand", &key), rect, dt);

                if !known && response.hover > 0.01 {
                    self.frame.fill(
                        rect,
                        theme::fade(palette.line, response.hover * 0.6),
                        radius::SM,
                    );
                }

                // A tick box drawn rather than glyphed, so it cannot vanish with a font.
                let box_rect: Rect = [inner + 6.0, y + 13.0, 18.0, 18.0];
                self.frame.fill(
                    box_rect,
                    if ticked { palette.accent } else { palette.line_strong },
                    radius::XS,
                );
                if ticked {
                    self.frame.dot(
                        [box_rect[0] + 9.0, box_rect[1] + 9.0],
                        8.0,
                        [1.0, 1.0, 1.0, 1.0],
                    );
                }

                let ink = if known { palette.muted } else { palette.ink };
                self.frame
                    .text(Run::new(name, inner + 36.0, y + 5.0, text::BASE, ink).weight(560));
                self.frame
                    .label(detail, inner + 36.0, y + 24.0, text::SM, palette.muted);

                if known {
                    self.frame.label(
                        "Already added",
                        inner + content - 110.0,
                        y + 14.0,
                        text::XS,
                        palette.muted,
                    );
                } else if response.clicked {
                    toggled = Some(key.clone());
                }

                y += 50.0;
            }

            if let Some(key) = toggled {
                if !self.chosen.remove(&key) {
                    self.chosen.insert(key);
                }
            }
        }

        let footer = height - 54.0;
        self.frame
            .rule(0.0, footer - gap::MD, width, false, palette.line);

        let count = self.chosen.len();
        let add_label = if count == 0 {
            "Add selected".to_string()
        } else {
            format!("Add {count} project{}", if count == 1 { "" } else { "s" })
        };

        let (rescan, add) = {
            let mut ui = self.ui(dt);
            let rescan = ui.button(
                "rescan",
                [inner, footer, 100.0, 32.0],
                "Rescan",
                Weight::Secondary,
            );
            let add = ui.button(
                "import",
                [width - gap::XL - 170.0, footer, 170.0, 32.0],
                &add_label,
                Weight::Primary,
            );
            (rescan, add)
        };

        if rescan {
            self.start_scan();
        }
        if add && count > 0 {
            self.import_chosen();
        }
        if leave {
            self.screen = Screen::Projects;
            self.scroll = 0.0;
        }
    }

    /// Kicks off a folder scan on a worker thread.
    ///
    /// Walking a disk blocks for as long as it takes, so it must never run on the frame.
    fn start_scan(&mut self) {
        if self.scanning {
            return;
        }
        self.scanning = true;
        self.candidates.clear();
        self.chosen.clear();

        let roots = self.config.settings.scan_roots.clone();
        let known: Vec<std::path::PathBuf> = self
            .config
            .projects
            .iter()
            .filter_map(|p| p.local.as_ref().map(|l| l.root.clone()))
            .collect();
        let slot = self.scan_result.clone();

        self.runtime.spawn_blocking(move || {
            let found = crate::core::discovery::scan(&roots, &known);
            *slot.lock() = Some(found);
        });
    }

    /// Turns the ticked candidates into projects.
    fn import_chosen(&mut self) {
        let mut order = self.config.projects.len() as i32;

        for candidate in &self.candidates {
            let key = candidate.root.to_string_lossy().to_string();
            if !self.chosen.contains(&key) {
                continue;
            }

            let mut project = Project::new(candidate.name.clone());
            project.kind = candidate.kind;
            project.repo = candidate.repo.clone();
            project.order = order;

            let mut local =
                LocalTarget::new(candidate.root.clone(), candidate.suggested_command.clone());
            local.port = candidate.suggested_port;
            project.local = Some(local);

            self.config.projects.push(project);
            order += 1;
        }

        self.chosen.clear();
        if let Err(err) = config::save(&self.config) {
            eprintln!("Oracle could not save the imported projects: {err}");
        }
        self.screen = Screen::Projects;
    }

    /// Collects finished health checks and starts a round if one is due.
    fn poll_health(&mut self) {
        for (project_id, status) in self.health.lock().drain(..) {
            self.remote.insert(project_id, status);
        }

        let interval =
            Duration::from_secs(self.config.settings.health_interval_secs.max(5) as u64);
        if self.last_health.elapsed() < interval {
            return;
        }
        self.last_health = Instant::now();

        let targets: Vec<(String, RemoteTarget)> = self
            .config
            .projects
            .iter()
            .filter_map(|p| p.remote.as_ref().map(|r| (p.id.clone(), r.clone())))
            .collect();

        if targets.is_empty() {
            return;
        }

        let client = self.http.clone();
        let slot = self.health.clone();

        self.runtime.spawn(async move {
            // Concurrent, so ten dead hosts cost one timeout rather than ten in sequence.
            let checks: Vec<_> = targets
                .into_iter()
                .map(|(id, target)| {
                    let client = client.clone();
                    tokio::spawn(async move {
                        let status = crate::core::remote::check(&client, &target).await;
                        (id, status)
                    })
                })
                .collect();

            for handle in checks {
                if let Ok(result) = handle.await {
                    slot.lock().push(result);
                }
            }
        });
    }

    /// Collects a finished scan.
    fn poll_scan(&mut self) {
        if let Some(found) = self.scan_result.lock().take() {
            self.candidates = found;
            self.scanning = false;
        }
    }
}

/// Where a project's "open" button should point.
fn open_url(project: &Project) -> Option<String> {
    if let Some(local) = &project.local {
        if let Some(url) = &local.open_url {
            return Some(url.clone());
        }
        if let Some(port) = local.port {
            return Some(format!("http://localhost:{port}"));
        }
    }
    project.remote.as_ref().map(|r| r.url.clone())
}

fn describe_remote(status: Option<&crate::core::remote::RemoteStatus>) -> String {
    use crate::core::remote::RemoteStatus;
    match status {
        Some(RemoteStatus::Up { ms, .. }) => format!("Up · {ms} ms"),
        Some(RemoteStatus::Degraded { code, ms }) => format!("HTTP {code} · {ms} ms"),
        Some(RemoteStatus::Down { reason }) => reason.clone(),
        _ => "Not checked yet".to_string(),
    }
}

/// Opens a URL in the user's browser.
///
/// Restricted to http and https, the same rule the web build's opener scope enforced: never
/// a path, and never a custom scheme that could hand something to another program.
#[cfg(windows)]
fn open_in_browser(url: &str) {
    use std::os::windows::process::CommandExt;

    if !url.starts_with("http://") && !url.starts_with("https://") {
        eprintln!("Oracle refused to open {url}: only http and https are allowed");
        return;
    }

    let mut command = std::process::Command::new("cmd");
    command.raw_arg(format!("/C start \"\" \"{url}\""));
    command.creation_flags(0x0800_0000);
    let _ = command.spawn();
}

#[cfg(not(windows))]
fn open_in_browser(url: &str) {
    if !url.starts_with("http://") && !url.starts_with("https://") {
        return;
    }
    let _ = std::process::Command::new("xdg-open").arg(url).spawn();
}

/// Reveals a folder in the system file manager.
#[cfg(windows)]
fn reveal(path: &std::path::Path) {
    use std::os::windows::process::CommandExt;
    let mut command = std::process::Command::new("explorer");
    command.arg(path);
    command.creation_flags(0x0800_0000);
    let _ = command.spawn();
}

#[cfg(not(windows))]
fn reveal(path: &std::path::Path) {
    let _ = std::process::Command::new("xdg-open").arg(path).spawn();
}

impl Oracle {
    /// Opens the form on a copy of a project, or on a blank one.
    ///
    /// A copy, so abandoning the form leaves the stored project untouched.
    fn open_form(&mut self, project: Option<&Project>) {
        self.draft_is_new = project.is_none();
        self.draft = Some(match project {
            Some(existing) => existing.clone(),
            None => {
                let mut fresh = Project::new(String::new());
                fresh.local = Some(LocalTarget::new(std::path::PathBuf::new(), String::new()));
                fresh
            }
        });
        self.screen = Screen::Form;
        self.input.blur();
        self.scroll = 0.0;
    }

    /// The width a string renders at, from the previous frame's measurements.
    ///
    /// Queues the string for measuring if it has not been seen, so the caret lands correctly
    /// from the second frame onwards.
    fn width_of(&mut self, value: &str) -> f32 {
        match self.measured.get(value) {
            Some(width) => *width,
            None => {
                if !self.to_measure.iter().any(|q| q == value) {
                    self.to_measure.push(value.to_string());
                }
                0.0
            }
        }
    }

    /// The add and edit form.
    fn form_screen(&mut self, width: f32, height: f32, dt: f32) {
        let palette = self.palette;
        let Some(mut draft) = self.draft.clone() else {
            self.screen = Screen::Projects;
            return;
        };

        let inner = gap::XL;
        let content = (width - gap::XL * 2.0).min(640.0);
        let half = (content - gap::MD) * 0.5;

        let title = if self.draft_is_new {
            "Add a project".to_string()
        } else {
            format!("Edit {}", draft.name)
        };
        self.frame.text(
            Run::new(title, inner, TITLEBAR + gap::XL, text::LG, palette.ink).weight(640),
        );

        // Pull every editable value out of the draft, so the borrow checker is not fighting
        // a `Ui` that holds the frame while the draft is also borrowed.
        let mut name = draft.name.clone();
        let local = draft.local.clone().unwrap_or_else(|| {
            LocalTarget::new(std::path::PathBuf::new(), String::new())
        });
        let mut root = local.root.to_string_lossy().to_string();
        let mut command = local.command.clone();
        let mut port = local.port.map(|p| p.to_string()).unwrap_or_default();
        let mut open = local.open_url.clone().unwrap_or_default();
        let mut health = draft
            .remote
            .as_ref()
            .map(|r| r.url.clone())
            .unwrap_or_default();

        let widths = [
            self.width_of(&name),
            self.width_of(&root),
            self.width_of(&command),
            self.width_of(&port),
            self.width_of(&open),
            self.width_of(&health),
        ];

        let back: Rect = [width - 44.0 - gap::XL, TITLEBAR + gap::XL, 30.0, 30.0];
        let mut y = TITLEBAR + 84.0;
        let leave;
        let mut changed = false;

        {
            let mut ui = self.ui(dt);
            leave = ui.icon_button("form-close", back, "\u{2715}", false);

            changed |= ui.labelled_field(
                "name", inner, y, content, "Name", &mut name, "Project name", widths[0],
            );
            y += 62.0;

            ui.heading(inner, y, "Runs locally");
            y += 24.0;

            changed |= ui.labelled_field(
                "root",
                inner,
                y,
                content,
                "Folder",
                &mut root,
                "C:\\Users\\you\\projects\\app",
                widths[1],
            );
            y += 62.0;

            changed |= ui.labelled_field(
                "command",
                inner,
                y,
                content,
                "Command",
                &mut command,
                "npm run dev",
                widths[2],
            );
            y += 62.0;

            changed |= ui.labelled_field(
                "port", inner, y, half, "Port", &mut port, "3000", widths[3],
            );
            changed |= ui.labelled_field(
                "open",
                inner + half + gap::MD,
                y,
                half,
                "Open URL",
                &mut open,
                "defaults to localhost",
                widths[4],
            );
            y += 70.0;

            ui.heading(inner, y, "Deployed elsewhere");
            y += 24.0;

            changed |= ui.labelled_field(
                "health",
                inner,
                y,
                content,
                "Health check URL",
                &mut health,
                "https://app.example.com/health",
                widths[5],
            );
            y += 70.0;
        }

        // Kind, as a segmented control rather than free text.
        let kinds = [
            ProjectKind::NextJs,
            ProjectKind::Node,
            ProjectKind::Rust,
            ProjectKind::Python,
            ProjectKind::Docker,
            ProjectKind::Static,
            ProjectKind::Other,
        ];
        let labels: Vec<&str> = kinds.iter().map(|k| k.label()).collect();
        let current = kinds.iter().position(|k| *k == draft.kind).unwrap_or(6);

        let picked = {
            let mut ui = self.ui(dt);
            ui.heading(inner, y, "Type");
            ui.segmented("kind", [inner, y + 24.0, content, 32.0], &labels, current)
        };
        if let Some(index) = picked {
            draft.kind = kinds[index];
            changed = true;
        }

        // Footer.
        let footer = height - 54.0;
        self.frame
            .rule(0.0, footer - gap::MD, width, false, palette.line);

        let is_new = self.draft_is_new;
        let (delete, cancel, save) = {
            let mut ui = self.ui(dt);
            let delete = if is_new {
                false
            } else {
                ui.button("form-delete", [inner, footer, 90.0, 32.0], "Delete", Weight::Danger)
            };
            let cancel = ui.button(
                "form-cancel",
                [width - gap::XL - 200.0, footer, 90.0, 32.0],
                "Cancel",
                Weight::Secondary,
            );
            let save = ui.button(
                "form-save",
                [width - gap::XL - 100.0, footer, 100.0, 32.0],
                "Save",
                Weight::Primary,
            );
            (delete, cancel, save)
        };

        // Fold the edited strings back into the draft.
        //
        // Stored exactly as typed. Trimming here would delete the space the moment it is
        // typed, so the next frame reseeds the field without it and the following word runs
        // into the previous one — "cargo test" became "cargotest". Whitespace is cleaned up
        // once, on save.
        if changed {
            draft.name = name;

            draft.local = if root.is_empty() && command.is_empty() {
                None
            } else {
                Some(LocalTarget {
                    root: std::path::PathBuf::from(&root),
                    command: command.clone(),
                    env: local.env.clone(),
                    port: port.trim().parse::<u16>().ok(),
                    open_url: (!open.is_empty()).then(|| open.clone()),
                    autostart_with_oracle: local.autostart_with_oracle,
                })
            };

            draft.remote = (!health.is_empty()).then(|| RemoteTarget {
                url: health.clone(),
                check: RemoteCheck::default(),
                interval_secs: draft
                    .remote
                    .as_ref()
                    .map(|r| r.interval_secs)
                    .unwrap_or(30),
            });

            self.draft = Some(draft.clone());
        }

        if delete {
            let id = draft.id.clone();
            self.delete_project(&id);
            return;
        }
        if cancel || leave {
            self.draft = None;
            self.screen = Screen::Projects;
            self.input.blur();
            return;
        }
        if save {
            self.save_draft(draft);
        }
    }

    /// Commits the form.
    ///
    /// A project with no name is refused rather than saved as a blank row nobody can
    /// identify; everything else is allowed through, because a command that does not work
    /// yet is a normal state to save.
    fn save_draft(&mut self, mut draft: Project) {
        if draft.name.trim().is_empty() {
            return;
        }

        // The one place whitespace is cleaned up, so editing never fights the user.
        draft.name = draft.name.trim().to_string();

        if let Some(local) = draft.local.as_mut() {
            local.root = std::path::PathBuf::from(local.root.to_string_lossy().trim());
            local.command = local.command.trim().to_string();
            local.open_url = local
                .open_url
                .as_ref()
                .map(|url| url.trim().to_string())
                .filter(|url| !url.is_empty());
        }
        if let Some(remote) = draft.remote.as_mut() {
            remote.url = remote.url.trim().to_string();
        }
        draft.remote = draft.remote.filter(|r| !r.url.is_empty());
        draft.local = draft
            .local
            .filter(|l| !l.root.as_os_str().is_empty() || !l.command.is_empty());

        match self.config.project_mut(&draft.id) {
            Some(existing) => {
                let order = existing.order;
                *existing = draft.clone();
                existing.order = order;
            }
            None => {
                draft.order = self.config.projects.len() as i32;
                self.config.projects.push(draft.clone());
            }
        }

        if let Err(err) = config::save(&self.config) {
            eprintln!("Oracle could not save the project: {err}");
        }

        self.draft = None;
        self.screen = Screen::Projects;
        self.selected = Some(draft.id);
        self.input.blur();
    }

    /// Removes a project, stopping it first so nothing is orphaned.
    fn delete_project(&mut self, project_id: &str) {
        if matches!(
            self.status_of(project_id),
            ProjectStatus::Running | ProjectStatus::Starting
        ) {
            let runner = self.runner.clone();
            let id = project_id.to_string();
            self.runtime.spawn(async move {
                let _ = runner.stop(&id).await;
            });
        }

        self.config.projects.retain(|p| p.id != project_id);
        self.config.normalise_order();
        self.statuses.remove(project_id);
        self.usage.remove(project_id);
        self.git.remove(project_id);
        self.remote.remove(project_id);

        if self.selected.as_deref() == Some(project_id) {
            self.selected = None;
        }

        if let Err(err) = config::save(&self.config) {
            eprintln!("Oracle could not save after removing the project: {err}");
        }

        self.draft = None;
        self.screen = Screen::Projects;
        self.input.blur();
    }
}

/// Size of the tray panel, in logical pixels.
const PANEL_WIDTH: f64 = 380.0;
const PANEL_HEIGHT: f64 = 560.0;

impl Oracle {
    /// Builds the tray icon and wires its events into the event loop's channel.
    ///
    /// `tray-icon` delivers events through global handlers on its own thread, so both are
    /// forwarded into a channel and acted on from the event loop where the windows live.
    fn build_tray(&mut self) {
        use tray_icon::menu::{Menu, MenuEvent, MenuItem};
        use tray_icon::{TrayIconBuilder, TrayIconEvent};

        let icon = match tray_image() {
            Some(icon) => icon,
            None => {
                eprintln!("Oracle could not build its tray icon");
                return;
            }
        };

        let menu = Menu::new();
        let open = MenuItem::new("Open Oracle", true, None);
        let quit = MenuItem::new("Quit", true, None);
        let open_id = open.id().clone();
        let quit_id = quit.id().clone();

        if menu.append(&open).is_err() || menu.append(&quit).is_err() {
            eprintln!("Oracle could not build its tray menu");
            return;
        }

        let sender = self.tray_sender.clone();
        MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
            let command = if event.id == open_id {
                TrayCommand::ShowWindow
            } else if event.id == quit_id {
                TrayCommand::Quit
            } else {
                return;
            };
            let _ = sender.send(command);
        }));

        let sender = self.tray_sender.clone();
        TrayIconEvent::set_event_handler(Some(move |event: TrayIconEvent| {
            // Left click toggles the panel. The menu has the right button, which is why it
            // is not shown on the left one.
            if let TrayIconEvent::Click {
                button: tray_icon::MouseButton::Left,
                button_state: tray_icon::MouseButtonState::Up,
                ..
            } = event
            {
                let _ = sender.send(TrayCommand::TogglePanel);
            }
        }));

        match TrayIconBuilder::new()
            .with_menu(Box::new(menu))
            .with_icon(icon)
            .with_tooltip("Oracle")
            .with_menu_on_left_click(false)
            .build()
        {
            Ok(tray) => self.tray = Some(tray),
            Err(err) => eprintln!("Oracle could not create its tray icon: {err}"),
        }
    }

    /// Acts on whatever the tray asked for since the last frame.
    fn poll_tray(&mut self, event_loop: &ActiveEventLoop) {
        while let Ok(command) = self.tray_events.try_recv() {
            match command {
                TrayCommand::TogglePanel => self.toggle_panel(),
                TrayCommand::ShowWindow => {
                    self.hide_panel();
                    if let Some(window) = self.window.as_ref() {
                        window.set_visible(true);
                        window.set_minimized(false);
                        window.focus_window();
                    }
                }
                TrayCommand::Quit => {
                    self.shutdown(event_loop);
                    return;
                }
            }
        }
    }

    fn toggle_panel(&mut self) {
        if self.panel_visible {
            self.hide_panel();
        } else {
            self.show_panel();
        }
    }

    /// Places the panel above the tray and shows it.
    ///
    /// The tray sits at the bottom right on a default Windows setup, and the work area stops
    /// where the taskbar begins — which is what anchors the panel without having to ask the
    /// shell where the tray icon actually is.
    fn show_panel(&mut self) {
        let Some(panel) = self.panel_window.as_ref() else {
            return;
        };

        if let Some(monitor) = panel.current_monitor().or_else(|| panel.primary_monitor()) {
            let scale = monitor.scale_factor();
            let area = monitor.size();
            let origin = monitor.position();

            let width = PANEL_WIDTH * scale;
            let height = PANEL_HEIGHT * scale;
            let margin = 12.0 * scale;
            // Room for the taskbar. Monitor size counts it as usable; it is not.
            let taskbar = 56.0 * scale;

            let x = origin.x as f64 + area.width as f64 - width - margin;
            let y = origin.y as f64 + area.height as f64 - height - taskbar - margin;

            panel.set_outer_position(winit::dpi::PhysicalPosition::new(x.max(0.0), y.max(0.0)));
        }

        panel.set_visible(true);
        panel.focus_window();
        self.panel_visible = true;
        self.needs_frame = true;
    }

    fn hide_panel(&mut self) {
        if let Some(panel) = self.panel_window.as_ref() {
            panel.set_visible(false);
        }
        self.panel_visible = false;
    }

    /// Stops every child and exits.
    fn shutdown(&mut self, event_loop: &ActiveEventLoop) {
        let runner = self.runner.clone();
        self.runtime.block_on(async move { runner.stop_all().await });
        event_loop.exit();
    }

    /// Lays out the tray panel.
    ///
    /// A real interface, not a menu: the same glass, the same controls, the same status the
    /// application shows — just denser, and only what can be acted on without reading.
    fn build_panel(&mut self, width: f32, height: f32, dt: f32) {
        self.panel_frame.clear();
        let palette = self.palette;

        // Header.
        let cx = gap::MD + 8.0;
        let cy = 26.0;
        self.panel_frame.dot([cx - 4.0, cy - 4.0], 6.0, palette.accent);
        self.panel_frame.dot([cx + 5.0, cy - 3.5], 4.5, palette.accent);
        self.panel_frame.dot([cx, cy + 3.5], 5.0, palette.accent);

        self.panel_frame.text(
            Run::new("Oracle", gap::MD + 22.0, cy - 8.0, text::BASE, palette.ink).weight(640),
        );

        let running = self
            .statuses
            .values()
            .filter(|s| matches!(s, ProjectStatus::Running))
            .count();
        self.panel_frame.text(
            Run::new(
                if running == 0 {
                    "Nothing running".to_string()
                } else {
                    format!("{running} running")
                },
                width - gap::MD,
                cy - 6.0,
                text::XS,
                palette.muted,
            )
            .align(Align::Right),
        );

        self.panel_frame
            .rule(0.0, 48.0, width, false, palette.line);

        // Rows.
        let projects: Vec<Project> = self.config.projects.clone();
        let mut y = 58.0;

        if projects.is_empty() {
            self.panel_frame.text(
                Run::new(
                    "No projects yet. Open Oracle to add one.",
                    width * 0.5,
                    height * 0.4,
                    text::SM,
                    palette.muted,
                )
                .align(Align::Centre)
                .width(width - gap::XL),
            );
        }

        let mut toggled = None;
        for project in &projects {
            if y > height - 62.0 {
                break;
            }

            let status = self.status_of(&project.id);
            let live = matches!(status, ProjectStatus::Running | ProjectStatus::Starting);
            let accent = self.accent_of(project);
            let rect: Rect = [gap::SM, y, width - gap::SM * 2.0, 44.0];

            let response = self
                .input
                .interact(id("panelrow", &project.id), rect, dt);
            if response.hover > 0.01 {
                self.panel_frame.fill(
                    rect,
                    theme::fade(palette.line, response.hover * 0.8),
                    radius::SM,
                );
            }

            let tile: Rect = [rect[0] + gap::SM, y + 9.0, 26.0, 26.0];
            self.panel_frame.fill(tile, accent, radius::XS);
            self.panel_frame.centred(
                initials(&project.name),
                tile[0] + 13.0,
                tile[1] + 5.0,
                text::XS,
                [1.0, 1.0, 1.0, 1.0],
                700,
            );

            let tx = tile[0] + 34.0;
            self.panel_frame.text(
                Run::new(&project.name, tx, y + 7.0, text::SM, palette.ink)
                    .weight(570)
                    .width(width - tx - 70.0),
            );

            let meta = match (live, self.usage.get(&project.id)) {
                (true, Some(sample)) => {
                    format!("{} · {}", percent(sample.cpu), bytes(sample.memory))
                }
                _ => status_label(status).to_string(),
            };
            self.panel_frame
                .label(meta, tx, y + 24.0, text::XS, palette.muted);

            self.panel_frame.dot(
                [width - 54.0, y + 22.0],
                7.0,
                status_colour(status, &palette),
            );

            if project.local.is_some() {
                let play: Rect = [width - 42.0, y + 9.0, 26.0, 26.0];
                let play_response = self
                    .input
                    .interact(id("panelplay", &project.id), play, dt);
                let lit = play_response.hover > 0.01 || live;

                self.panel_frame.fill(
                    play,
                    if lit {
                        palette.accent
                    } else {
                        theme::fade(palette.accent, 0.18)
                    },
                    radius::XS,
                );
                self.panel_frame.centred(
                    if live { "\u{25A0}" } else { "\u{25B6}" },
                    play[0] + 13.0,
                    play[1] + 5.0,
                    text::XS,
                    if lit { [1.0, 1.0, 1.0, 1.0] } else { palette.accent },
                    500,
                );

                if play_response.clicked {
                    toggled = Some(project.id.clone());
                }
            }

            y += 50.0;
        }

        if let Some(project_id) = toggled {
            self.toggle(&project_id);
        }

        // Footer.
        let footer = height - 46.0;
        self.panel_frame
            .rule(0.0, footer - gap::SM, width, false, palette.line);

        let (open, quit) = {
            let palette = self.palette;
            let mut ui = Ui {
                frame: &mut self.panel_frame,
                input: &mut self.input,
                palette,
                dt,
            };
            let open = ui.button(
                "panel-open",
                [gap::MD, footer, 120.0, 30.0],
                "Open Oracle",
                Weight::Primary,
            );
            let quit = ui.icon_button(
                "panel-quit",
                [width - gap::MD - 30.0, footer, 30.0, 30.0],
                "\u{23FB}",
                false,
            );
            (open, quit)
        };

        if open {
            let _ = self.tray_sender.send(TrayCommand::ShowWindow);
        }
        if quit {
            let _ = self.tray_sender.send(TrayCommand::Quit);
        }
    }
}

/// The tray icon, built from the same three circles as the brand mark.
///
/// Drawn rather than loaded: a PNG beside the executable is one more thing to lose, and at
/// 32 pixels the mark is three discs and a hole.
fn tray_image() -> Option<tray_icon::Icon> {
    const SIZE: u32 = 32;
    let accent = [193u8, 95, 60];

    let mut pixels = vec![0u8; (SIZE * SIZE * 4) as usize];

    // Centre, radius, and whether the disc is solid or a ring.
    let discs: [(f32, f32, f32, bool); 4] = [
        (16.0, 17.5, 6.6, true),
        (8.5, 8.5, 4.0, false),
        (16.2, 27.0, 3.4, false),
        (25.5, 9.0, 3.2, false),
    ];

    for y in 0..SIZE {
        for x in 0..SIZE {
            let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
            let mut coverage = 0.0f32;

            for (cx, cy, radius, hollow) in discs {
                let distance = ((px - cx).powi(2) + (py - cy).powi(2)).sqrt();
                // One pixel of feather, so the mark is not jagged at this size.
                let inside = (1.0 - (distance - radius + 0.5)).clamp(0.0, 1.0);
                let mut value = inside;

                if hollow {
                    // The hole through the middle disc.
                    let hole = ((distance - 2.5) + 0.5).clamp(0.0, 1.0);
                    value = inside.min(hole);
                }

                coverage = coverage.max(value);
            }

            let index = ((y * SIZE + x) * 4) as usize;
            pixels[index] = accent[0];
            pixels[index + 1] = accent[1];
            pixels[index + 2] = accent[2];
            pixels[index + 3] = (coverage * 255.0) as u8;
        }
    }

    tray_icon::Icon::from_rgba(pixels, SIZE, SIZE).ok()
}

impl Oracle {
    /// Events belonging to the tray panel.
    fn panel_event(&mut self, event: WindowEvent) {
        match event {
            // The panel behaves like a popover: losing focus dismisses it.
            WindowEvent::Focused(false) => self.hide_panel(),

            WindowEvent::CursorMoved { position, .. } => {
                self.input.pointer = [
                    position.x as f32 / self.scale,
                    position.y as f32 / self.scale,
                ];
                self.input.pointer_in_window = true;
            }

            WindowEvent::CursorLeft { .. } => self.input.pointer_in_window = false,

            WindowEvent::MouseInput { state, button, .. } if button == MouseButton::Left => {
                match state {
                    ElementState::Pressed => {
                        self.input.down = true;
                        self.input.just_pressed = true;
                    }
                    ElementState::Released => {
                        self.input.down = false;
                        self.input.just_released = true;
                    }
                }
            }

            WindowEvent::RedrawRequested => {
                let now = Instant::now();
                let dt = now.duration_since(self.last_frame).as_secs_f32().min(0.1);
                self.last_frame = now;

                self.poll();

                let Some(target) = self.panel_target.as_ref() else {
                    return;
                };
                let (pw, ph) = target.size();
                let (width, height) = (pw as f32 / self.scale, ph as f32 / self.scale);

                self.build_panel(width, height, dt);

                let elapsed = self.started.elapsed().as_secs_f32();
                let palette = self.palette;
                if let (Some(gpu), Some(target)) = (self.gpu.as_ref(), self.panel_target.as_mut())
                {
                    target.render(gpu, &self.panel_frame, &palette, elapsed);
                }

                self.input.end_frame();
            }

            _ => {}
        }
    }
}
