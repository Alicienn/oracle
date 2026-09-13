//! One error type for the whole backend.
//!
//! Every failure that can reach the user carries a stable `code` the frontend can switch
//! on, a short `message` fit for a toast, and an optional `detail` for the expandable
//! section. Nothing is swallowed and nothing reaches the user as a bare string.

use serde::Serialize;
use std::path::PathBuf;

pub type Result<T> = std::result::Result<T, OracleError>;

#[derive(Debug, thiserror::Error)]
pub enum OracleError {
    #[error("no project with id {0}")]
    ProjectNotFound(String),

    #[error("{0} has no local target configured")]
    NoLocalTarget(String),

    #[error("{0} has no remote target configured")]
    NoRemoteTarget(String),

    #[error("{0} is already running")]
    AlreadyRunning(String),

    #[error("{0} is not running")]
    NotRunning(String),

    /// A port the project wants is already taken.
    ///
    /// Carries what the frontend needs to offer a way out rather than only a complaint:
    /// who holds the port, and the next port that is genuinely free.
    #[error("port {port} is already in use by {holder}")]
    PortInUse {
        port: u16,
        holder: String,
        suggestion: Option<u16>,
    },

    #[error("the launch command is empty")]
    EmptyCommand,

    #[error("working directory does not exist: {0}")]
    MissingWorkingDir(PathBuf),

    #[error("could not start the process: {0}")]
    SpawnFailed(String),

    #[error("could not stop the process: {0}")]
    StopFailed(String),

    #[error("could not read or write the configuration")]
    Config(#[source] std::io::Error),

    #[error("the configuration file is not valid JSON")]
    ConfigParse(#[source] serde_json::Error),

    #[error("filesystem error at {path}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("git is not available on this machine")]
    GitMissing,

    #[error("git failed: {0}")]
    Git(String),

    #[error("the request failed")]
    Http(#[source] reqwest::Error),

    #[error("{0}")]
    Other(String),
}

impl OracleError {
    /// Stable identifier the frontend can branch on. Never changes once shipped.
    pub fn code(&self) -> &'static str {
        match self {
            Self::ProjectNotFound(_) => "project_not_found",
            Self::NoLocalTarget(_) => "no_local_target",
            Self::NoRemoteTarget(_) => "no_remote_target",
            Self::AlreadyRunning(_) => "already_running",
            Self::NotRunning(_) => "not_running",
            Self::PortInUse { .. } => "port_in_use",
            Self::EmptyCommand => "empty_command",
            Self::MissingWorkingDir(_) => "missing_working_dir",
            Self::SpawnFailed(_) => "spawn_failed",
            Self::StopFailed(_) => "stop_failed",
            Self::Config(_) => "config_io",
            Self::ConfigParse(_) => "config_parse",
            Self::Io { .. } => "io",
            Self::GitMissing => "git_missing",
            Self::Git(_) => "git",
            Self::Http(_) => "http",
            Self::Other(_) => "other",
        }
    }

    /// Machine-readable payload, for the few errors the UI can act on.
    ///
    /// `message` is written for a human to read; an error the frontend has to *do* something
    /// with needs its parts separately, not parsed back out of a sentence.
    fn data(&self) -> Option<serde_json::Value> {
        match self {
            Self::PortInUse {
                port,
                holder,
                suggestion,
            } => Some(serde_json::json!({
                "port": port,
                "holder": holder,
                "suggestion": suggestion,
            })),
            _ => None,
        }
    }

    /// The underlying cause, flattened into a readable chain.
    fn detail(&self) -> Option<String> {
        use std::error::Error;
        let mut chain = Vec::new();
        let mut current = self.source();
        while let Some(err) = current {
            chain.push(err.to_string());
            current = err.source();
        }
        if chain.is_empty() {
            None
        } else {
            Some(chain.join(": "))
        }
    }
}

/// Wire format. `OracleError` itself cannot derive `Serialize` because its sources don't.
#[derive(Serialize)]
struct WireError {
    code: &'static str,
    message: String,
    detail: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<serde_json::Value>,
}

impl serde::Serialize for OracleError {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        WireError {
            code: self.code(),
            message: self.to_string(),
            detail: self.detail(),
            data: self.data(),
        }
        .serialize(serializer)
    }
}

impl From<reqwest::Error> for OracleError {
    fn from(err: reqwest::Error) -> Self {
        Self::Http(err)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serialises_with_code_and_detail() {
        let io = std::io::Error::new(std::io::ErrorKind::PermissionDenied, "access denied");
        let err = OracleError::Config(io);
        let json = serde_json::to_value(&err).unwrap();

        assert_eq!(json["code"], "config_io");
        assert_eq!(json["message"], "could not read or write the configuration");
        assert_eq!(json["detail"], "access denied");
    }

    #[test]
    fn errors_without_a_source_have_no_detail() {
        let json = serde_json::to_value(OracleError::EmptyCommand).unwrap();
        assert_eq!(json["code"], "empty_command");
        assert!(json["detail"].is_null());
    }

    #[test]
    fn a_port_conflict_carries_its_parts_for_the_ui() {
        let json = serde_json::to_value(OracleError::PortInUse {
            port: 3000,
            holder: "Promethee".into(),
            suggestion: Some(3001),
        })
        .unwrap();

        assert_eq!(json["code"], "port_in_use");
        assert_eq!(json["message"], "port 3000 is already in use by Promethee");
        assert_eq!(json["data"]["port"], 3000);
        assert_eq!(json["data"]["holder"], "Promethee");
        assert_eq!(json["data"]["suggestion"], 3001);
    }

    #[test]
    fn errors_the_ui_cannot_act_on_carry_no_data() {
        let json = serde_json::to_value(OracleError::EmptyCommand).unwrap();
        assert!(json.get("data").is_none());
    }
}
