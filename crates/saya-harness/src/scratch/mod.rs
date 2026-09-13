//! The run's scratch database — the one writable SQL surface (ADR 0003).
//!
//! One DuckDB file per run, at `runs/<id>/scratch.duckdb`, opened with
//! external access **off** (the 2026-09-11 amendment and sign-off, measured
//! by `tests/scratch_semantics.rs`): `INSTALL`/`LOAD` and every file reader
//! are refused at the permission layer, so the validator rejects the whole
//! file-reading family outright, by name, and there is no in-run-dir
//! path carve-out. What survives is the capability the ADR was written to
//! get — DDL, DML and joins on the scratch file itself.
//!
//! The scratch surface never travels the `DatabaseConnector` path and never
//! enters the `ConnectionRegistry`: it is a different type with a different
//! policy, validated by its own `sqlparser` pass, with the engine's
//! timeout/interrupt/caps discipline re-implemented locally.
//!
//! Modules: [`validate`] (the policy pass), [`open`] (the pinned
//! configuration and file mode), [`decode`] (staged rows to JSON), [`execute`]
//! (timeout, interrupt, budgets), [`tools`] (the `scratch_sql` tool and its
//! admission).

mod decode;
mod execute;
mod open;
mod tools;
mod validate;

pub use open::{SCRATCH_FILE_NAME, SCRATCH_QUERY_TIMEOUT, ScratchDb};
pub use tools::{SCRATCH_SQL_TOOL, ScratchSql};
pub use validate::{SCRATCH_ROW_CAP, ScratchRejection, Validated, validate};

use std::path::PathBuf;

use thiserror::Error;

/// Scratch's own error vocabulary — open, typed refusal, timeout, execution.
/// The refusal variants surface verbatim to the model so it can self-correct.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum ScratchError {
    /// The DuckDB file could not be created or opened under the pinned
    /// configuration.
    #[error("scratch database could not be opened: {context}")]
    Open { context: String },

    /// The post-create 0600 restriction failed. DuckDB creates the file at
    /// 0644; failing to harden it is refused rather than left wide.
    #[error("scratch file could not be restricted to 0600: {path}")]
    FileMode {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    /// The validator refused the statement; the detail is the typed
    /// [`ScratchRejection`]'s own text.
    #[error("scratch statement refused: {0}")]
    Refused(#[from] ScratchRejection),

    /// The statement exceeded the per-statement ceiling and was interrupted;
    /// the connection was proven released before this is returned.
    #[error("scratch statement timed out")]
    TimedOut,

    /// The statement failed at the engine. The detail carries only
    /// allow-listed DuckDB message classes — never a staged value.
    #[error("scratch statement failed: {message}")]
    Execution { message: String },
}
