//! The shape of everything Oracle persists.
//!
//! These types are the contract between the backend, the config file on disk, and the
//! frontend. Every field is optional-tolerant on read (`#[serde(default)]`) so that a config
//! written by an older build still loads after an upgrade.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

/// Bumped whenever a migration becomes necessary.
pub const CONFIG_VERSION: u32 = 1;

// ---------------------------------------------------------------------------
// Project
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Project {
    pub id: String,
    pub name: String,

    #[serde(default)]
    pub kind: ProjectKind,

    #[serde(default)]
    pub icon: IconSource,

    /// Overrides the colour derived from `kind`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accent: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local: Option<LocalTarget>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote: Option<RemoteTarget>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo: Option<RepoRef>,

    #[serde(default)]
    pub tags: Vec<String>,

    #[serde(default)]
    pub favorite: bool,

    /// Position in the rail. Lower sorts first.
    #[serde(default)]
    pub order: i32,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
}

impl Project {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            name: name.into(),
            kind: ProjectKind::Other,
            icon: IconSource::Auto,
            accent: None,
            local: None,
            remote: None,
            repo: None,
            tags: Vec::new(),
            favorite: false,
            order: 0,
            notes: None,
        }
    }

    /// The URL the "open" button should use, if any.
    pub fn open_url(&self) -> Option<String> {
        if let Some(local) = &self.local {
            if let Some(url) = &local.open_url {
                return Some(url.clone());
            }
            if let Some(port) = local.port {
                return Some(format!("http://localhost:{port}"));
            }
        }
        self.remote.as_ref().map(|r| r.url.clone())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ProjectKind {
    NextJs,
    Node,
    Rust,
    Python,
    Docker,
    Static,
    #[default]
    Other,
}

impl ProjectKind {
    /// Fallback accent when the project does not define its own.
    pub fn accent(self) -> &'static str {
        match self {
            Self::NextJs => "#C15F3C",
            Self::Node => "#6E9B5E",
            Self::Rust => "#B3623C",
            Self::Python => "#4E7CA8",
            Self::Docker => "#3F7EA8",
            Self::Static => "#8A8578",
            Self::Other => "#B1ADA1",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::NextJs => "Next.js",
            Self::Node => "Node",
            Self::Rust => "Rust",
            Self::Python => "Python",
            Self::Docker => "Docker",
            Self::Static => "Static",
            Self::Other => "Other",
        }
    }

    /// The command a freshly discovered project of this kind most likely needs.
    pub fn default_command(self) -> &'static str {
        match self {
            Self::NextJs | Self::Node => "npm run dev",
            Self::Rust => "cargo run",
            Self::Python => "python main.py",
            Self::Docker => "docker compose up",
            Self::Static => "npx serve .",
            Self::Other => "",
        }
    }

    /// The port that command conventionally binds to.
    pub fn default_port(self) -> Option<u16> {
        match self {
            Self::NextJs | Self::Node => Some(3000),
            Self::Static => Some(3000),
            Self::Python => Some(8000),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "camelCase")]
pub enum IconSource {
    /// Derived from the project kind.
    #[default]
    Auto,
    /// One of the glyphs bundled with the frontend.
    Builtin(String),
    /// A PNG or SVG the user picked, copied into the Oracle data directory.
    File(PathBuf),
    /// Fetched from the project's own URL.
    Favicon(String),
}

// ---------------------------------------------------------------------------
// Local target
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalTarget {
    pub root: PathBuf,

    /// Shell command line, run through `cmd /C` on Windows.
    pub command: String,

    /// Extra environment on top of the inherited one.
    #[serde(default)]
    pub env: BTreeMap<String, String>,

    /// Port the process is expected to bind. Drives the readiness probe.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,

    /// Overrides `http://localhost:{port}`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub open_url: Option<String>,

    /// Launch this project when Oracle itself starts.
    #[serde(default)]
    pub autostart_with_oracle: bool,
}

impl LocalTarget {
    pub fn new(root: PathBuf, command: impl Into<String>) -> Self {
        Self {
            root,
            command: command.into(),
            env: BTreeMap::new(),
            port: None,
            open_url: None,
            autostart_with_oracle: false,
        }
    }
}

// ---------------------------------------------------------------------------
// Remote target
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteTarget {
    pub url: String,

    #[serde(default)]
    pub check: RemoteCheck,

    #[serde(default = "default_interval")]
    pub interval_secs: u32,
}

fn default_interval() -> u32 {
    30
}

/// How a remote target is probed. An enum so SSH and agent-based checks can be added
/// later without breaking configs written today.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum RemoteCheck {
    #[serde(rename_all = "camelCase")]
    Http {
        #[serde(default = "default_method")]
        method: HttpMethod,
        /// Status codes treated as healthy. Empty means "any 2xx or 3xx".
        #[serde(default)]
        expect_status: Vec<u16>,
    },
}

impl Default for RemoteCheck {
    fn default() -> Self {
        Self::Http {
            method: HttpMethod::Get,
            expect_status: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum HttpMethod {
    #[default]
    Get,
    Head,
}

fn default_method() -> HttpMethod {
    HttpMethod::Get
}

// ---------------------------------------------------------------------------
// Repository
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RepoRef {
    #[serde(default)]
    pub provider: RepoProvider,

    /// "owner/name" when it could be parsed from the remote URL.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slug: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_url: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RepoProvider {
    GitHub,
    GitLab,
    #[default]
    Other,
}

// ---------------------------------------------------------------------------
// Settings
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    #[serde(default)]
    pub start_with_windows: bool,

    #[serde(default)]
    pub start_hidden: bool,

    #[serde(default = "default_true")]
    pub autostart_projects: bool,

    #[serde(default)]
    pub theme: Theme,

    #[serde(default)]
    pub glass: GlassLevel,

    #[serde(default = "default_interval")]
    pub health_interval_secs: u32,

    #[serde(default)]
    pub scan_roots: Vec<PathBuf>,

    #[serde(default = "default_shortcut")]
    pub panel_shortcut: String,

    #[serde(default = "default_true")]
    pub minimise_to_tray: bool,

    #[serde(default)]
    pub view: ViewMode,
}

fn default_true() -> bool {
    true
}

fn default_shortcut() -> String {
    "CmdOrCtrl+Shift+Space".to_string()
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            start_with_windows: false,
            start_hidden: false,
            autostart_projects: true,
            theme: Theme::default(),
            glass: GlassLevel::default(),
            health_interval_secs: default_interval(),
            scan_roots: default_scan_roots(),
            panel_shortcut: default_shortcut(),
            minimise_to_tray: true,
            view: ViewMode::default(),
        }
    }
}

/// Sensible places to look for projects on a fresh install.
fn default_scan_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Some(home) = dirs::home_dir() {
        for name in ["Documents", "Projects", "dev", "code"] {
            let candidate = home.join(name);
            if candidate.is_dir() {
                roots.push(candidate);
            }
        }
    }
    roots
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Theme {
    Light,
    Dark,
    #[default]
    System,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum GlassLevel {
    /// Refraction, blur, and specular highlight.
    #[default]
    Full,
    /// Blur and highlight, no refraction filter.
    Reduced,
    /// Flat surfaces. Cheapest, and the accessibility fallback.
    Opaque,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ViewMode {
    #[default]
    List,
    Grid,
}

// ---------------------------------------------------------------------------
// Root document
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Config {
    #[serde(default = "default_version")]
    pub version: u32,

    #[serde(default)]
    pub projects: Vec<Project>,

    #[serde(default)]
    pub settings: Settings,
}

fn default_version() -> u32 {
    CONFIG_VERSION
}

impl Default for Config {
    fn default() -> Self {
        Self {
            version: CONFIG_VERSION,
            projects: Vec::new(),
            settings: Settings::default(),
        }
    }
}

impl Config {
    pub fn project(&self, id: &str) -> Option<&Project> {
        self.projects.iter().find(|p| p.id == id)
    }

    pub fn project_mut(&mut self, id: &str) -> Option<&mut Project> {
        self.projects.iter_mut().find(|p| p.id == id)
    }

    /// Renumbers `order` to match current array position, closing any gaps.
    pub fn normalise_order(&mut self) {
        self.projects.sort_by_key(|p| p.order);
        for (index, project) in self.projects.iter_mut().enumerate() {
            project.order = index as i32;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_default_config_round_trips() {
        let config = Config::default();
        let json = serde_json::to_string(&config).unwrap();
        let back: Config = serde_json::from_str(&json).unwrap();

        assert_eq!(back.version, CONFIG_VERSION);
        assert!(back.projects.is_empty());
        assert_eq!(back.settings.health_interval_secs, 30);
    }

    #[test]
    fn missing_fields_fall_back_to_defaults() {
        // What a much older, much sparser config file might look like.
        let json = r#"{ "projects": [ { "id": "a", "name": "Legacy" } ] }"#;
        let config: Config = serde_json::from_str(json).unwrap();

        assert_eq!(config.version, CONFIG_VERSION);
        assert_eq!(config.projects.len(), 1);
        assert_eq!(config.projects[0].kind, ProjectKind::Other);
        assert!(!config.projects[0].favorite);
        assert_eq!(config.settings.theme, Theme::System);
    }

    #[test]
    fn open_url_prefers_the_explicit_override() {
        let mut project = Project::new("Site");
        let mut local = LocalTarget::new(PathBuf::from("/tmp"), "npm run dev");
        local.port = Some(3000);
        project.local = Some(local);

        assert_eq!(project.open_url().as_deref(), Some("http://localhost:3000"));

        project.local.as_mut().unwrap().open_url = Some("http://localhost:3000/admin".into());
        assert_eq!(
            project.open_url().as_deref(),
            Some("http://localhost:3000/admin")
        );
    }

    #[test]
    fn normalising_order_closes_gaps_and_sorts() {
        let mut config = Config::default();
        for (name, order) in [("c", 40), ("a", 5), ("b", 12)] {
            let mut project = Project::new(name);
            project.order = order;
            config.projects.push(project);
        }

        config.normalise_order();

        let names: Vec<_> = config.projects.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["a", "b", "c"]);
        assert_eq!(
            config.projects.iter().map(|p| p.order).collect::<Vec<_>>(),
            [0, 1, 2]
        );
    }

    #[test]
    fn remote_check_defaults_to_a_plain_get() {
        let target: RemoteTarget =
            serde_json::from_str(r#"{ "url": "https://example.com" }"#).unwrap();

        assert_eq!(target.interval_secs, 30);
        match target.check {
            RemoteCheck::Http { method, expect_status } => {
                assert!(matches!(method, HttpMethod::Get));
                assert!(expect_status.is_empty());
            }
        }
    }
}
