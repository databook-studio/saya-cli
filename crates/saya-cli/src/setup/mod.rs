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

// S16 adapters: the interactive flow and its steps, the prompts and the
// question sets, the probes, and the review renderer. The engine above stays
// pure; everything terminal-shaped lives in these.
pub(crate) mod flow;
mod flow_commit;
mod flow_options;
mod flow_probe;
mod flow_recover;
mod probe;
mod probe_database;
mod probe_provider;
mod prompt;
mod prompt_database;
mod prompt_provider;
mod review;

#[cfg(test)]
#[path = "setup_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "flow_tests.rs"]
mod flow_tests;

pub const CONFIG_FILE: &str = "config.toml";
pub const CONNECTIONS_FILE: &str = "connections.toml";
/// Existing files are read bounded; the same bound backs up and restores them.
pub const MAX_FILE_BYTES: u64 = 1024 * 1024;

pub use commit::commit;
pub use draft::SetupDraft;
pub use plan::{PlannedWrite, SetupPlan, plan};
pub use recover::pending;

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
    #[error(
        "setup could not be applied ({message}); the automatic restore did NOT complete \
             ({restore}); `saya setup` will offer to restore the original files on the next run"
    )]
    ReloadRestoreFailed { message: String, restore: String },
    #[error("setup marker unusable: {0}")]
    Marker(String),
    #[error("an interrupted setup commit is pending in {path}; restore or finish it first")]
    CommitPending { path: std::path::PathBuf },
}
