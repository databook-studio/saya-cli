//! The runner tool's typed error surface. Every variant names what the
//! model needs to correct — the allowlist, the argv shape, the sandbox, the
//! timeout — never a generic failure. The refusal battery (`refuse`), the
//! credential seam (`env`), the spawn itself, and the output record all
//! raise it, so a call's outcome is always one of these variants.

use std::io;

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum RunnerError {
    #[error("run_program refused: `{program}` is not in this step's allowlist")]
    ProgramNotAllowlisted { program: String },

    #[error("run_program refused: `{program}` — {reason}")]
    ProgramRefused {
        program: String,
        reason: &'static str,
    },

    #[error("run_program refused: no allowlisted program file exists at `{path}`")]
    ProgramMissing { program: String, path: String },

    #[error("run_program refused: the program file could not be inspected at `{path}`")]
    ProgramFile { path: String, source: io::Error },

    #[error("run_program refused: the call is not typed argv — {detail}")]
    ArgsNotTyped { detail: &'static str },

    #[error("run_program refused: the timeout must be at least one second")]
    TimeoutNotPositive,

    #[error(
        "run_program refused: the requested timeout {requested}s exceeds the declared default {default}s"
    )]
    TimeoutExceedsDefault { requested: u64, default: u64 },

    #[error("run_program refused: credential `{credential}` is invalid — {reason}")]
    CredentialInvalid {
        credential: String,
        reason: &'static str,
    },

    #[error("run_program failed: credential `{credential}` could not be resolved: {detail}")]
    CredentialUnresolved { credential: String, detail: String },

    #[error("run_program failed: the child could not be started: {source}")]
    Spawn { source: io::Error },

    #[error("run_program failed: the child's exit could not be observed: {source}")]
    WaitFailed { source: io::Error },

    #[error("run_program failed: the output record could not be written: {source}")]
    RecordFailed { source: io::Error },
}
