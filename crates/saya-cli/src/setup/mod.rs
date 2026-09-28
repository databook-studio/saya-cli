//! The `saya setup` engine: draft → plan → commit with private backups and a
//! recoverable interruption marker.
//!
//! Pure: no terminal I/O, no prompts, no network. The adapters (the clap
//! command, the interactive flow, the probes) live outside this module (S16)
//! and drive it through [`plan`] and [`commit`].

mod atomic;
mod commit;
mod draft;
mod plan;
mod recover;
mod render;

#[cfg(test)]
#[path = "setup_tests.rs"]
mod tests;

pub const CONFIG_FILE: &str = "config.toml";
pub const CONNECTIONS_FILE: &str = "connections.toml";
/// Existing files are read bounded; the same bound backs up and restores them.
pub const MAX_FILE_BYTES: u64 = 1024 * 1024;

pub use commit::{CommitReport, commit};
pub use draft::{ProfileDraft, ProviderDraft, SetupDraft};
pub use plan::{PlannedWrite, SetupPlan, plan};
pub use recover::{MarkerEntry, PendingCommit, finish, pending, restore};

/// Everything the setup engine can fail with.
#[derive(Debug, thiserror::Error)]
pub enum SetupError {
    #[error("{path}: {source}")]
    Io {
        path: std::path::PathBuf,
        source: std::io::Error,
    },
    #[error("refusing to follow the symlink {path}")]
    Symlink { path: std::path::PathBuf },
    #[error("{path} is larger than the {max} byte bound")]
    TooLarge { path: std::path::PathBuf, max: u64 },
    #[error("invalid setup input: {0}")]
    Draft(String),
    #[error("profile {0:?} already exists in connections.toml")]
    ProfileExists(String),
    #[error("existing file is unusable: {0}")]
    Existing(String),
    #[error("planned setup content is invalid: {0}")]
    InvalidResult(String),
    #[error("could not render setup content: {0}")]
    Render(String),
    #[error("{0}")]
    UnsupportedEngine(String),
    #[error("setup could not be applied ({0}); the original files were restored")]
    ReloadFailed(String),
    #[error("setup marker unusable: {0}")]
    Marker(String),
    #[error("an interrupted setup commit is pending in {path}; restore or finish it first")]
    CommitPending { path: std::path::PathBuf },
}
