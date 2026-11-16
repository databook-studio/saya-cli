//! The confined-child spawn substrate: the proven [`sandbox::RunnerSpawn`],
//! the captured output, the credential seam (`env`), and the workspace shell
//! (`shell`) that is the only admitted caller.
//!
//! This module has no typed-argv tool of its own: nothing here can start a
//! child that the workspace shell did not resolve. What the shell uses is
//! exactly what the tests measure: the confinement, the empty-then-declared
//! environment, the
//! output ring with its byte caps, the unconditional `redact()` pass, and
//! the process-group kill on timeout and cancellation.

pub mod env;
pub mod error;
pub mod output;
pub mod refuse;
pub mod sandbox;
pub mod shell;
pub(crate) mod spawn;

#[cfg(all(test, unix))]
#[path = "shell_tests.rs"]
mod shell_tests;

pub use env::{Credential, CredentialSource, SharedCredentialSource, StaticCredentialSource};
pub use error::RunnerError;
pub use output::{
    OUTPUT_CAP_BYTES, OutputRing, ProgramOutcome, SHELL_SPOOL_CAPTURE_CAP_BYTES, StreamCapture,
    elision_marker,
};
pub use shell::{
    BackgroundRuns, HostPolicy, MAX_BACKGROUND_RUNS, MAX_TOOLCHAIN_DIRS, ReadReport, RunStatus,
    SHELL_PATH, ShellCall, ShellOutcome, WorkspaceShell, path_with_toolchain, toolchain_dirs,
    toolchain_dirs_for_process,
};

pub use refuse::{MAX_ARG_BYTES, MAX_ARG_COUNT, MAX_ARGV_BYTES};
