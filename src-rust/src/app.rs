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

use crate::diag;
use crate::core::config::{self, model::*};
use crate::core::monitor::{Monitor, Sample};
use crate::core::runner::{ProcessManager, ProjectStatus, RunnerEvent};
use crate::ui::input::{id, Input, Key};
use crate::ui::paint::{Frame, Rect};
use crate::ui::renderer::Renderer;
use crate::ui::text::{Align, Run};
use crate::ui::theme::{self, gap, radius, text, Colour, GlassStyle, Palette};

/// Height of the custom title bar, in logical pixels.
const TITLEBAR: f32 = 42.0;
/// Width of the project rail.
const RAIL: f32 = 64.0;

pub struct Oracle {
    window: Option<Arc<Window>>,
    renderer: Option<Renderer>,

    config: Config,
    palette: Palette,
    scale: f32,

    input: Input,
    frame: Frame,

    selected: Option<String>,
    query: String,
    filter: Filter,

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

        Self {
            window: None,
            renderer: None,
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
                Filter::Running => false,
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

        let palette = self.palette;
        self.detail
            .set_target(if self.selected.is_some() { 380.0 } else { 0.0 });
        self.detail.tick(dt);
        let detail_width = self.detail.get();

        self.titlebar(width, dt);

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

        // Add and settings sit at the bottom of the rail.
        let bottom = top + height - gap::MD - 88.0;
        for (index, glyph) in ["+", "\u{2699}"].iter().enumerate() {
            let rect: Rect = [(RAIL - 40.0) * 0.5, bottom + index as f32 * 44.0, 40.0, 40.0];
            let response = self.input.interact(id("railaction", glyph), rect, dt);

            if response.hover > 0.01 {
                self.frame
                    .fill(rect, alpha_of(palette.line, response.hover), radius::MD);
            }
            self.frame.centred(
                *glyph,
                rect[0] + 20.0,
                rect[1] + 9.0,
                text::MD,
                palette.muted,
                500,
            );
        }
    }

    fn toolbar(&mut self, x: f32, y: f32, width: f32, dt: f32) {
        let palette = self.palette;
        let inner = x + gap::LG;
        let row_y = y + gap::MD;

        // Search field.
        let filters_width = 240.0;
        let actions_width = 108.0;
        let search: Rect = [
            inner,
            row_y,
            (width - gap::LG * 2.0 - filters_width - actions_width - gap::SM * 2.0).max(120.0),
            34.0,
        ];
        self.frame
            .panel(search, GlassStyle::sunken(&palette), 1.0);

        let label = if self.query.is_empty() {
            "Search projects".to_string()
        } else {
            self.query.clone()
        };
        let colour = if self.query.is_empty() {
            palette.muted
        } else {
            palette.ink
        };
        self.frame
            .label(label, search[0] + gap::MD, search[1] + 9.0, text::BASE, colour);

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

        // Scan and add.
        let mut ax = seg_rect[0] + seg_rect[2] + gap::SM;
        for (key, glyph, primary) in [("scan", "\u{2318}", false), ("add", "+", true)] {
            let rect: Rect = [ax, row_y, 48.0, 34.0];
            let response = self.input.interact(id("action", key), rect, dt);

            if primary {
                let lift = response.hover * 0.12;
                self.frame.fill(
                    rect,
                    [
                        palette.accent[0] + lift,
                        palette.accent[1] + lift,
                        palette.accent[2] + lift,
                        1.0,
                    ],
                    radius::SM,
                );
            } else {
                self.frame
                    .panel(rect, GlassStyle::control(&palette), 1.0);
            }

            self.frame.centred(
                glyph,
                rect[0] + 24.0,
                rect[1] + 8.0,
                text::MD,
                if primary { [1.0, 1.0, 1.0, 1.0] } else { palette.ink_soft },
                600,
            );

            ax += 56.0;
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
        let accent = self.accent_of(&project);

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
            Run::new(&project.name, inner + 46.0, y + gap::LG + 2.0, text::MD, palette.ink)
                .weight(640),
        );
        self.frame.label(
            project.kind.label(),
            inner + 46.0,
            y + gap::LG + 22.0,
            text::SM,
            palette.muted,
        );

        let close: Rect = [x + width - 44.0, y + gap::LG, 30.0, 30.0];
        let close_response = self.input.interact(id("detail", "close"), close, dt);
        if close_response.hover > 0.01 {
            self.frame
                .fill(close, alpha_of(palette.line, close_response.hover), radius::SM);
        }
        self.frame.centred(
            "\u{2715}",
            close[0] + 15.0,
            close[1] + 7.0,
            text::SM,
            palette.muted,
            500,
        );
        if close_response.clicked {
            self.selected = None;
        }

        // Key/value rows.
        let mut row_y = y + 82.0;
        let mut row = |frame: &mut Frame, label: &str, value: String| {
            frame.label(label, inner, row_y, text::SM, palette.muted);
            frame.text(
                Run::new(value, inner + 104.0, row_y, text::BASE, palette.ink)
                    .width(width - 104.0 - gap::LG * 2.0),
            );
            row_y += 26.0;
        };

        if let Some(local) = &project.local {
            row(&mut self.frame, "Folder", short_path(&local.root.to_string_lossy()));
            row(&mut self.frame, "Command", local.command.clone());
            if let Some(port) = local.port {
                row(&mut self.frame, "Port", port.to_string());
            }
        }
        if let Some(remote) = &project.remote {
            row(&mut self.frame, "Health check", remote.url.clone());
        }
        if let Some(repo) = &project.repo {
            if let Some(slug) = &repo.slug {
                row(&mut self.frame, "Repository", slug.clone());
            }
        }
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
                (
                    (size.width as f64 / scale - 120.0).min(1240.0).max(960.0),
                    (size.height as f64 / scale - 120.0).min(820.0).max(600.0),
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
        probe("after window");

        match pollster::block_on(Renderer::new(window.clone(), self.scale)) {
            Ok(mut renderer) => {
                renderer.set_blur(Blur::Standard);
                self.renderer = Some(renderer);
                probe("after renderer");
            }
            Err(err) => {
                eprintln!("Oracle could not start the renderer: {err}");
                event_loop.exit();
                return;
            }
        }

        self.window = Some(window);
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),

            WindowEvent::Resized(size) => {
                if let Some(renderer) = self.renderer.as_mut() {
                    renderer.resize(size.width, size.height);
                }
                self.needs_frame = true;
            }

            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                self.scale = scale_factor as f32;
                self.frame.set_scale(self.scale);
                if let Some(renderer) = self.renderer.as_mut() {
                    renderer.text.set_scale(self.scale);
                }
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
                        if self.selected.is_some() {
                            self.selected = None;
                        }
                    }
                    WinitKey::Named(NamedKey::Backspace) => {
                        self.query.pop();
                    }
                    WinitKey::Character(c) => {
                        self.query.push_str(c.as_str());
                    }
                    WinitKey::Named(NamedKey::Space) => self.query.push(' '),
                    _ => {}
                }
                self.needs_frame = true;
            }

            WindowEvent::RedrawRequested => {
                let now = Instant::now();
                let dt = now.duration_since(self.last_frame).as_secs_f32().min(0.1);
                self.last_frame = now;

                let Some(renderer) = self.renderer.as_mut() else {
                    return;
                };

                let (pw, ph) = renderer.size();
                let (width, height) = (pw as f32 / self.scale, ph as f32 / self.scale);

                self.build(width, height, dt);

                let elapsed = self.started.elapsed().as_secs_f32();
                let palette = self.palette;
                if let Some(renderer) = self.renderer.as_mut() {
                    renderer.render(&self.frame, &palette, elapsed);
                }

                // Another frame is needed only while something is still moving. The wash
                // drifts, so this is effectively always true — but the structure is here so
                // that switching the wash off makes the app genuinely idle.
                self.needs_frame = self.input.animating() || !self.detail.is_settled() || true;
                self.input.end_frame();

                if self.closing {
                    // Children outlive their parent on Windows unless they are killed
                    // explicitly, so nothing Oracle started is left running with no way to
                    // reach it.
                    let runner = self.runner.clone();
                    self.runtime.block_on(async move { runner.stop_all().await });
                    event_loop.exit();
                }
            }

            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if let Some(window) = self.window.as_ref() {
            if self.needs_frame {
                window.request_redraw();
                event_loop.set_control_flow(ControlFlow::Poll);
            } else {
                event_loop.set_control_flow(ControlFlow::Wait);
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
