//! The saya harness: the home of the run engine.
//!
//! A run is a persistent, resumable, budgeted, capability-scoped unit of
//! delegated work. This crate owns its mechanics — where run directories
//! live, how they are created, how a single writer claims one, and the
//! append-only event journal a resume replays. The fetch module (`fetch`)
//! is the fail-closed decision of where a run may reach: the policy itself
//! performs no network I/O, and the `http_fetch` tool it gates touches the
//! wire only through that policy. The one database surface the run engine
//! holds is the run-scoped scratch database (`scratch`, ADR 0003): a
//! different type with a different policy from every user-database
//! connector, and never a `DatabaseConnector`.

pub mod engine;
pub mod fetch;
pub mod journal;
pub mod lock;
pub mod paths;
pub mod run_dir;
pub mod scratch;
pub mod workspace;

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

    /// The path argument is not a usable relative name: NUL bytes, an empty
    /// string, empty or dot components, or a Windows drive/UNC shape.
    #[error("workspace path argument is invalid: {path}")]
    InvalidPath { path: String },

    /// The path resolves outside the run workspace: an absolute form or a
    /// `..` rising above the root.
    #[error("workspace path escapes the run workspace: {path}")]
    PathOutsideRoot { path: String },

    /// A path component is a symlink. Links are refused, never followed, at
    /// any component.
    #[error("symlink in workspace path is refused: {path}")]
    SymlinkRefused { path: String },

    /// The path names hygiene-denied content (`.git`). Hygiene only — never
    /// a credentials control.
    #[error("workspace path names denied hygiene content: {path}")]
    DeniedName { path: String },

    /// A bound (read cap, entry count, file count) was exceeded.
    #[error("workspace bound exceeded for {path}: found {found}, max {max}")]
    BoundsExceeded { path: String, found: u64, max: u64 },

    /// The file at the path changed identity — a (dev, inode) mismatch
    /// between the pre-open scan and the opened file. The path changed
    /// hands while it was being opened.
    #[error("workspace file changed identity while it was being used: {path}")]
    IdentityChanged { path: String },

    /// The path names something that is not a regular file (a directory, a
    /// fifo, a device).
    #[error("workspace path is not a regular file: {path}")]
    NotRegularFile { path: String },

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
