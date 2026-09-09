//! The saya harness: the home of the run engine.
//!
//! A run is a persistent, resumable, budgeted, capability-scoped unit of
//! delegated work. This crate owns its mechanics — where run directories
//! live, how they are created, how a single writer claims one, and the
//! append-only event journal a resume replays. The engine that drives
//! episodes arrives in later milestones; nothing here talks to databases or
//! providers.

pub mod journal;
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

    /// A newline-terminated line in the run journal does not parse as a
    /// `RunEvent` — corruption, or an event written by a build that knows
    /// more variants than this one. Fails closed rather than guessed at.
    #[error("run journal is corrupt: line {line} does not parse as a run event")]
    JournalCorrupt { line: usize },

    /// A run event could not be serialized for the journal. Unreachable for
    /// the current event payloads; typed rather than panicked on.
    #[error("run event could not be serialized for the journal: {source}")]
    JournalEncode {
        #[source]
        source: serde_json::Error,
    },
}

pub(crate) fn io_error(context: &str, path: &Path, error: std::io::Error) -> HarnessError {
    HarnessError::Io {
        context: format!("{context} {}", path.display()),
        source: error,
    }
}
