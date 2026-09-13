//! Everything Oracle does that is not drawing.
//!
//! Carried over from the Tauri build unchanged. These modules never knew what shell they ran
//! inside — the runner reports through a plain closure, the monitor and the health checker
//! are functions over data — which is why replacing the entire interface touched none of
//! them.

pub mod autostart;
pub mod config;
pub mod discovery;
pub mod error;
pub mod monitor;
pub mod remote;
pub mod runner;
pub mod vcs;
