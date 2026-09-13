//! Git status, read by shelling out to `git`.
//!
//! `git status --porcelain=v2 --branch` is a stable, machine-readable format designed for
//! exactly this. Linking `libgit2` would mean compiling C and reimplementing config and
//! credential discovery to get the same three facts.

use crate::core::error::{OracleError, Result};
use serde::Serialize;
use std::path::Path;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitStatus {
    pub branch: Option<String>,
    pub upstream: Option<String>,
    pub ahead: u32,
    pub behind: u32,
    /// Number of files with staged or unstaged changes, including untracked.
    pub dirty_files: u32,
    pub last_commit: Option<Commit>,
}

impl GitStatus {
    pub fn is_clean(&self) -> bool {
        self.dirty_files == 0
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Commit {
    pub hash: String,
    pub subject: String,
    pub author: String,
    /// Unix seconds.
    pub at: i64,
}

/// True when a `git` binary is on `PATH`.
///
/// Checked once and cached: a machine does not grow a git installation mid-session, and the
/// alternative is spawning a process on every status call.
pub fn is_available() -> bool {
    use std::sync::OnceLock;
    static AVAILABLE: OnceLock<bool> = OnceLock::new();

    *AVAILABLE.get_or_init(|| {
        run(Path::new("."), &["--version"])
            .map(|out| out.starts_with("git version"))
            .unwrap_or(false)
    })
}

/// Reads the status of the repository at `root`, if it is one.
///
/// Returns `Ok(None)` when the directory is not a repository — a project without version
/// control is a normal state, not an error.
pub fn status(root: &Path) -> Result<Option<GitStatus>> {
    if !is_available() {
        return Err(OracleError::GitMissing);
    }

    if !root.join(".git").exists() {
        return Ok(None);
    }

    let raw = run(root, &["status", "--porcelain=v2", "--branch"])?;
    let mut status = parse_status(&raw);
    status.last_commit = last_commit(root).ok().flatten();

    Ok(Some(status))
}

/// Parses porcelain v2 output.
///
/// Header lines start with `#`, everything else is one line per changed path.
fn parse_status(raw: &str) -> GitStatus {
    let mut status = GitStatus {
        branch: None,
        upstream: None,
        ahead: 0,
        behind: 0,
        dirty_files: 0,
        last_commit: None,
    };

    for line in raw.lines() {
        let Some(header) = line.strip_prefix("# ") else {
            if !line.trim().is_empty() {
                status.dirty_files += 1;
            }
            continue;
        };

        let mut parts = header.split_whitespace();
        match parts.next() {
            Some("branch.head") => {
                // "(detached)" is git's own marker, not a branch name.
                status.branch = parts.next().filter(|n| *n != "(detached)").map(String::from);
            }
            Some("branch.upstream") => {
                status.upstream = parts.next().map(String::from);
            }
            Some("branch.ab") => {
                // Formatted as "+3 -1".
                for token in parts {
                    if let Some(value) = token.strip_prefix('+') {
                        status.ahead = value.parse().unwrap_or(0);
                    } else if let Some(value) = token.strip_prefix('-') {
                        status.behind = value.parse().unwrap_or(0);
                    }
                }
            }
            _ => {}
        }
    }

    status
}

fn last_commit(root: &Path) -> Result<Option<Commit>> {
    // A unit separator is safe here: it cannot appear in a commit subject.
    let raw = match run(root, &["log", "-1", "--format=%h\x1f%s\x1f%an\x1f%ct"]) {
        Ok(raw) => raw,
        // An empty repository has no commits, which git reports as a failure.
        Err(_) => return Ok(None),
    };

    Ok(parse_commit(&raw))
}

fn parse_commit(raw: &str) -> Option<Commit> {
    let line = raw.trim();
    if line.is_empty() {
        return None;
    }

    let fields: Vec<&str> = line.split('\x1f').collect();
    if fields.len() != 4 {
        return None;
    }

    Some(Commit {
        hash: fields[0].to_string(),
        subject: fields[1].to_string(),
        author: fields[2].to_string(),
        at: fields[3].parse().unwrap_or(0),
    })
}

/// Runs a git command in `dir` and returns its stdout.
fn run(dir: &Path, args: &[&str]) -> Result<String> {
    let mut command = std::process::Command::new("git");
    command.args(args).current_dir(dir);

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // Without this a console window flashes on screen for every call.
        command.creation_flags(0x0800_0000);
    }

    let output = command
        .output()
        .map_err(|err| OracleError::Git(err.to_string()))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(OracleError::Git(stderr.trim().to_string()));
    }

    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_clean_branch_with_an_upstream_is_read_correctly() {
        let raw = "\
# branch.oid 1a2b3c4d
# branch.head main
# branch.upstream origin/main
# branch.ab +0 -0
";
        let status = parse_status(raw);

        assert_eq!(status.branch.as_deref(), Some("main"));
        assert_eq!(status.upstream.as_deref(), Some("origin/main"));
        assert_eq!(status.ahead, 0);
        assert_eq!(status.behind, 0);
        assert!(status.is_clean());
    }

    #[test]
    fn ahead_and_behind_counts_are_read() {
        let raw = "\
# branch.head feature
# branch.upstream origin/feature
# branch.ab +3 -2
";
        let status = parse_status(raw);

        assert_eq!(status.ahead, 3);
        assert_eq!(status.behind, 2);
    }

    #[test]
    fn changed_and_untracked_files_are_counted() {
        let raw = "\
# branch.head main
# branch.ab +0 -0
1 .M N... 100644 100644 100644 abc def src/main.rs
1 M. N... 100644 100644 100644 abc def README.md
? notes.txt
";
        let status = parse_status(raw);

        assert_eq!(status.dirty_files, 3);
        assert!(!status.is_clean());
    }

    #[test]
    fn a_detached_head_reports_no_branch() {
        let raw = "\
# branch.oid 1a2b3c4d
# branch.head (detached)
";
        let status = parse_status(raw);

        assert!(status.branch.is_none());
    }

    #[test]
    fn a_repository_with_no_upstream_is_handled() {
        let raw = "\
# branch.head local-only
";
        let status = parse_status(raw);

        assert_eq!(status.branch.as_deref(), Some("local-only"));
        assert!(status.upstream.is_none());
        assert_eq!(status.ahead, 0);
    }

    #[test]
    fn a_commit_line_is_split_on_the_unit_separator() {
        let commit =
            parse_commit("9f2c1ab\x1fAdd the tray panel\x1fAlicien\x1f1757750000").unwrap();

        assert_eq!(commit.hash, "9f2c1ab");
        assert_eq!(commit.subject, "Add the tray panel");
        assert_eq!(commit.author, "Alicien");
        assert_eq!(commit.at, 1757750000);
    }

    #[test]
    fn a_subject_containing_separators_does_not_break_parsing() {
        // Pipes, colons and dashes are common in commit subjects and must survive.
        let commit =
            parse_commit("abc1234\x1ffix: handle a|b - properly\x1fAlicien\x1f1757750000").unwrap();

        assert_eq!(commit.subject, "fix: handle a|b - properly");
    }

    #[test]
    fn an_empty_log_yields_no_commit() {
        assert!(parse_commit("").is_none());
        assert!(parse_commit("   \n").is_none());
    }

    #[test]
    fn a_truncated_commit_line_is_rejected_rather_than_guessed() {
        assert!(parse_commit("abc1234\x1fonly a subject").is_none());
    }
}
