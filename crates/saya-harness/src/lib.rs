//! The saya harness: the home of the run engine.
//!
//! A run is a persistent, resumable, budgeted, capability-scoped unit of
//! delegated work. This crate owns its mechanics — where run directories
//! live, how they are created, and how a single writer claims one. The
//! engine that drives episodes arrives in later milestones; nothing here
//! talks to databases or providers.

pub mod lock;
pub mod paths;
pub mod run_dir;

use std::path::Path;

use thiserror::Error;

/// Errors from the harness's filesystem and locking surface. Rendered for
/// the user by `saya-cli`; here they stay data.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum HarnessError {
    /// Another live process holds the run; the diagnostic names its pid.
    #[error("run is held by a live process (pid {pid})")]
    LockHeld { pid: u32 },

    /// The lock changed hands while being claimed and no live holder could
    /// be identified; retrying is safe.
    #[error("run lock is contended; no live holder was identified")]
    LockContended,

    #[error("{context}: {source}")]
    Io {
        context: String,
        #[source]
        source: std::io::Error,
    },
}

pub(crate) fn io_error(context: &str, path: &Path, error: std::io::Error) -> HarnessError {
    HarnessError::Io {
        context: format!("{context} {}", path.display()),
        source: error,
    }
}
