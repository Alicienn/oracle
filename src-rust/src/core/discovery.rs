//! Finding projects on disk.
//!
//! The scan is deliberately shallow and skips the directories that make recursive walks
//! expensive. A project is identified by a marker file at its root, and a directory that
//! looks like a project is never descended into — nested `node_modules` packages and Cargo
//! workspace members would otherwise flood the results.

use crate::core::config::model::{ProjectKind, RepoProvider, RepoRef};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

/// How deep below a scan root to look. Four levels covers `Documents/work/client/app`
/// without walking an entire drive.
const MAX_DEPTH: usize = 4;

/// Directories that never contain a project root worth surfacing.
const SKIP: &[&str] = &[
    "node_modules",
    "target",
    "dist",
    "build",
    ".git",
    ".next",
    ".venv",
    "venv",
    "__pycache__",
    ".cache",
    "vendor",
    ".svelte-kit",
    "out",
];

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Candidate {
    pub name: String,
    pub root: PathBuf,
    pub kind: ProjectKind,
    pub suggested_command: String,
    pub suggested_port: Option<u16>,
    pub repo: Option<RepoRef>,
    /// True when a project with this root is already in the config.
    pub already_known: bool,
}

/// Walks every root and returns the deduplicated candidates, sorted by name.
pub fn scan(roots: &[PathBuf], known: &[PathBuf]) -> Vec<Candidate> {
    let mut found: Vec<Candidate> = Vec::new();

    for root in roots {
        walk(root, 0, &mut found);
    }

    // The same directory can sit under two overlapping scan roots.
    found.sort_by(|a, b| a.root.cmp(&b.root));
    found.dedup_by(|a, b| a.root == b.root);

    for candidate in &mut found {
        candidate.already_known = known.iter().any(|k| paths_equal(k, &candidate.root));
    }

    found.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    found
}

fn walk(dir: &Path, depth: usize, out: &mut Vec<Candidate>) {
    if depth > MAX_DEPTH {
        return;
    }

    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        // An unreadable directory is not an error worth surfacing during a scan; it is
        // usually a permissions quirk on a system folder.
        Err(_) => return,
    };

    if let Some(kind) = detect_kind(dir) {
        out.push(build_candidate(dir, kind));
        // A project root is a leaf as far as the scan is concerned.
        return;
    }

    for entry in entries.flatten() {
        let path = entry.path();
        if !entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            continue;
        }

        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with('.') || SKIP.contains(&name.as_ref()) {
            continue;
        }

        walk(&path, depth + 1, out);
    }
}

/// Identifies a project by its marker files. Order matters: a Next.js app also has a
/// `package.json`, and a Dockerised Rust service also has a `Cargo.toml`.
pub fn detect_kind(dir: &Path) -> Option<ProjectKind> {
    let package_json = dir.join("package.json");
    if package_json.is_file() {
        if is_next_app(&package_json) {
            return Some(ProjectKind::NextJs);
        }
        return Some(ProjectKind::Node);
    }

    if dir.join("Cargo.toml").is_file() {
        return Some(ProjectKind::Rust);
    }

    if dir.join("pyproject.toml").is_file()
        || dir.join("requirements.txt").is_file()
        || dir.join("Pipfile").is_file()
    {
        return Some(ProjectKind::Python);
    }

    if dir.join("docker-compose.yml").is_file() || dir.join("docker-compose.yaml").is_file() {
        return Some(ProjectKind::Docker);
    }

    if dir.join("index.html").is_file() {
        return Some(ProjectKind::Static);
    }

    None
}

/// Looks for `next` in either dependency block. Parsed as generic JSON because the exact
/// shape of a `package.json` in the wild is not worth modelling.
fn is_next_app(package_json: &Path) -> bool {
    let Ok(raw) = fs::read_to_string(package_json) else {
        return false;
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&raw) else {
        return false;
    };

    ["dependencies", "devDependencies"]
        .iter()
        .any(|block| value.get(block).and_then(|d| d.get("next")).is_some())
}

/// Folder names that say nothing about which project they belong to.
///
/// A scan of a real machine turns up three directories called `api` and two called
/// `backend`. Importing those as-is gives a list nobody can read, so a generic name is
/// qualified with its parent: `veln-leads/api` rather than `api`.
const GENERIC: &[&str] = &[
    "api", "app", "backend", "client", "frontend", "server", "web", "www", "src", "site",
    "ui", "admin", "dashboard", "core", "main", "service", "services", "packages",
];

/// The name a discovered project should carry.
pub fn display_name(dir: &Path) -> String {
    let own = dir
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| dir.display().to_string());

    if !GENERIC.contains(&own.to_lowercase().as_str()) {
        return own;
    }

    match dir.parent().and_then(|p| p.file_name()) {
        Some(parent) => format!("{}/{}", parent.to_string_lossy(), own),
        None => own,
    }
}

fn build_candidate(dir: &Path, kind: ProjectKind) -> Candidate {
    let name = display_name(dir);

    Candidate {
        name,
        root: dir.to_path_buf(),
        kind,
        suggested_command: suggested_command(dir, kind),
        suggested_port: kind.default_port(),
        repo: read_repo(dir),
        already_known: false,
    }
}

/// Prefers a script the project actually declares over the generic default for its kind.
fn suggested_command(dir: &Path, kind: ProjectKind) -> String {
    if matches!(kind, ProjectKind::NextJs | ProjectKind::Node) {
        if let Ok(raw) = fs::read_to_string(dir.join("package.json")) {
            if let Ok(value) = serde_json::from_str::<serde_json::Value>(&raw) {
                if let Some(scripts) = value.get("scripts") {
                    for name in ["dev", "start", "serve"] {
                        if scripts.get(name).is_some() {
                            return format!("npm run {name}");
                        }
                    }
                }
            }
        }
    }
    kind.default_command().to_string()
}

/// Reads the origin remote straight out of `.git/config` rather than shelling out, so a
/// scan of a hundred directories does not spawn a hundred processes.
fn read_repo(dir: &Path) -> Option<RepoRef> {
    let raw = fs::read_to_string(dir.join(".git").join("config")).ok()?;

    let mut in_origin = false;
    for line in raw.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_origin = line.starts_with("[remote \"origin\"]");
            continue;
        }
        if in_origin {
            if let Some(url) = line.strip_prefix("url = ").or_else(|| line.strip_prefix("url=")) {
                return Some(repo_from_url(url.trim()));
            }
        }
    }

    None
}

/// Understands both `https://host/owner/name.git` and `git@host:owner/name.git`.
pub fn repo_from_url(url: &str) -> RepoRef {
    let provider = if url.contains("github.com") {
        RepoProvider::GitHub
    } else if url.contains("gitlab.com") {
        RepoProvider::GitLab
    } else {
        RepoProvider::Other
    };

    let tail = url
        .rsplit_once(':')
        .filter(|(head, _)| !head.ends_with("https") && !head.ends_with("http"))
        .map(|(_, tail)| tail)
        .unwrap_or(url);

    let trimmed = tail.trim_end_matches(".git").trim_end_matches('/');
    let parts: Vec<&str> = trimmed.rsplit('/').take(2).collect();

    let slug = if parts.len() == 2 {
        Some(format!("{}/{}", parts[1], parts[0]))
    } else {
        None
    };

    RepoRef {
        provider,
        slug,
        remote_url: Some(url.to_string()),
    }
}

/// Path comparison that tolerates the casing and separator differences Windows allows.
fn paths_equal(a: &Path, b: &Path) -> bool {
    let normalise = |p: &Path| {
        p.to_string_lossy()
            .replace('\\', "/")
            .trim_end_matches('/')
            .to_lowercase()
    };
    normalise(a) == normalise(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a throwaway directory tree and cleans it up on drop.
    struct TempTree(PathBuf);

    impl TempTree {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!("oracle-scan-{}", uuid::Uuid::new_v4()));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        fn file(&self, relative: &str, contents: &str) -> &Self {
            let path = self.0.join(relative);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, contents).unwrap();
            self
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempTree {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).ok();
        }
    }

    #[test]
    fn a_next_app_is_not_mistaken_for_plain_node() {
        let tree = TempTree::new();
        tree.file(
            "app/package.json",
            r#"{ "dependencies": { "next": "15.0.0" } }"#,
        );

        assert_eq!(
            detect_kind(&tree.path().join("app")),
            Some(ProjectKind::NextJs)
        );
    }

    #[test]
    fn plain_node_is_detected_without_next() {
        let tree = TempTree::new();
        tree.file("api/package.json", r#"{ "dependencies": { "express": "4" } }"#);

        assert_eq!(detect_kind(&tree.path().join("api")), Some(ProjectKind::Node));
    }

    #[test]
    fn cargo_python_docker_and_static_are_recognised() {
        let tree = TempTree::new();
        tree.file("svc/Cargo.toml", "[package]");
        tree.file("bot/requirements.txt", "requests");
        tree.file("stack/docker-compose.yml", "services:");
        tree.file("site/index.html", "<html>");

        assert_eq!(detect_kind(&tree.path().join("svc")), Some(ProjectKind::Rust));
        assert_eq!(
            detect_kind(&tree.path().join("bot")),
            Some(ProjectKind::Python)
        );
        assert_eq!(
            detect_kind(&tree.path().join("stack")),
            Some(ProjectKind::Docker)
        );
        assert_eq!(
            detect_kind(&tree.path().join("site")),
            Some(ProjectKind::Static)
        );
    }

    #[test]
    fn an_empty_directory_is_not_a_project() {
        let tree = TempTree::new();
        fs::create_dir_all(tree.path().join("empty")).unwrap();

        assert_eq!(detect_kind(&tree.path().join("empty")), None);
    }

    #[test]
    fn the_scan_skips_dependency_directories() {
        let tree = TempTree::new();
        // A distinctive name, so this tests the skip list rather than how names are chosen.
        tree.file("apptiktok/package.json", r#"{ "name": "apptiktok" }"#);
        // A package nested inside node_modules must never surface as a project.
        tree.file(
            "apptiktok/node_modules/left-pad/package.json",
            r#"{ "name": "left-pad" }"#,
        );

        let found = scan(&[tree.path().to_path_buf()], &[]);

        assert_eq!(found.len(), 1);
        assert_eq!(found[0].name, "apptiktok");
    }

    #[test]
    fn the_scan_does_not_descend_into_a_project_root() {
        let tree = TempTree::new();
        tree.file("workspace/Cargo.toml", "[workspace]");
        tree.file("workspace/crates/core/Cargo.toml", "[package]");

        let found = scan(&[tree.path().to_path_buf()], &[]);

        assert_eq!(found.len(), 1);
        assert_eq!(found[0].name, "workspace");
    }

    #[test]
    fn a_declared_script_beats_the_generic_default() {
        let tree = TempTree::new();
        tree.file(
            "app/package.json",
            r#"{ "scripts": { "start": "node server.js" } }"#,
        );

        let found = scan(&[tree.path().to_path_buf()], &[]);
        assert_eq!(found[0].suggested_command, "npm run start");
    }

    #[test]
    fn known_roots_are_flagged_regardless_of_casing() {
        let tree = TempTree::new();
        tree.file("app/package.json", "{}");
        let known = vec![PathBuf::from(
            tree.path().join("APP").to_string_lossy().to_uppercase(),
        )];

        let found = scan(&[tree.path().to_path_buf()], &known);
        assert!(found[0].already_known);
    }

    #[test]
    fn a_distinctive_folder_name_is_left_alone() {
        assert_eq!(display_name(Path::new("C:/dev/AppTiktok")), "AppTiktok");
        assert_eq!(display_name(Path::new("C:/dev/oracle")), "oracle");
    }

    #[test]
    fn a_generic_folder_name_is_qualified_by_its_parent() {
        // A real scan turned up three directories called `api`. Unqualified, the import
        // list is unusable.
        assert_eq!(display_name(Path::new("C:/dev/veln-leads/api")), "veln-leads/api");
        assert_eq!(display_name(Path::new("C:/dev/morlier/backend")), "morlier/backend");
    }

    #[test]
    fn qualification_is_case_insensitive() {
        assert_eq!(display_name(Path::new("C:/dev/thing/API")), "thing/API");
        assert_eq!(display_name(Path::new("C:/dev/thing/Client")), "thing/Client");
    }

    #[test]
    fn a_generic_name_with_no_parent_keeps_what_it_has() {
        assert_eq!(display_name(Path::new("api")), "api");
    }

    #[test]
    fn scanning_gives_sibling_projects_distinguishable_names() {
        let tree = TempTree::new();
        tree.file("alpha/api/package.json", "{}");
        tree.file("beta/api/package.json", "{}");

        let found = scan(&[tree.path().to_path_buf()], &[]);
        let names: Vec<&str> = found.iter().map(|c| c.name.as_str()).collect();

        assert_eq!(names.len(), 2);
        assert_ne!(names[0], names[1], "two projects called api is unusable");
        assert!(names.contains(&"alpha/api"));
        assert!(names.contains(&"beta/api"));
    }

    #[test]
    fn ssh_and_https_remotes_both_yield_a_slug() {
        let ssh = repo_from_url("git@github.com:alicien/oracle.git");
        assert_eq!(ssh.slug.as_deref(), Some("alicien/oracle"));
        assert_eq!(ssh.provider, RepoProvider::GitHub);

        let https = repo_from_url("https://github.com/alicien/oracle.git");
        assert_eq!(https.slug.as_deref(), Some("alicien/oracle"));
        assert_eq!(https.provider, RepoProvider::GitHub);

        let other = repo_from_url("https://git.example.com/team/thing");
        assert_eq!(other.slug.as_deref(), Some("team/thing"));
        assert_eq!(other.provider, RepoProvider::Other);
    }

    #[test]
    fn the_origin_remote_is_read_from_git_config() {
        let tree = TempTree::new();
        tree.file("app/package.json", "{}");
        tree.file(
            "app/.git/config",
            "[core]\n\trepositoryformatversion = 0\n[remote \"origin\"]\n\turl = git@github.com:alicien/oracle.git\n",
        );

        let found = scan(&[tree.path().to_path_buf()], &[]);
        let repo = found[0].repo.as_ref().expect("origin should be detected");
        assert_eq!(repo.slug.as_deref(), Some("alicien/oracle"));
    }
}
