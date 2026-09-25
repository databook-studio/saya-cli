//! The runner child's spawn path: confine, wait, kill, report. Nothing here
//! can be reached without a proven [`RunnerSpawn`] — the sandbox module's
//! verdict is consumed, never re-derived — and nothing reaches this module
//! until [`super::refuse::validate`] has refused every escape it names.
//!
//! The two measured rules this file exists to honour (docs/sandbox-spike.md):
//! - **Typed argv, never a shell.** Every argument arrives as exactly one
//!   argv element; there is no string that becomes a command line. A child
//!   that needs a shell would be a child the allowlist refuses.
//! - **Timeout and cancellation kill the process group.** The child is
//!   created with `process_group(0)`, so it leads its own group and every
//!   descendant — daemonized or not — is a member; `libc::killpg` reaches
//!   all of them. The kill is issued, then the blocking wait is awaited to
//!   completion (the interrupt-then-await honesty pattern): the child is
//!   reaped before anything is reported, and partial output is reported as
//!   partial.

use std::{path::PathBuf, process::Stdio};

use saya_agent::CancellationToken;

use super::{
    env::{Credential, inject},
    error::RunnerError,
    output::ProgramOutcome,
    refuse::ValidatedCall,
    sandbox::RunnerSpawn,
};

#[cfg(unix)]
use std::os::unix::process::CommandExt as _;

/// Spawns the validated call, waits on it under the timeout and the run's
/// cancellation, and reports. Exit code is data, not an error.
///
/// `pinned_env` is the nested re-entry's (M5-5) run-pinned paths, set ahead
/// of the declared credentials; a `run_program` child passes none and its
/// environment stays empty-but-declared.
pub(super) async fn run(
    spawn: &RunnerSpawn,
    call: ValidatedCall,
    pinned_env: &[(&'static str, PathBuf)],
    credentials: &[Credential],
    source: &dyn super::env::CredentialSource,
    cancellation: &CancellationToken,
) -> Result<ProgramOutcome, RunnerError> {
    let mut command = spawn.command(&call.program_path);
    // Typed argv, verbatim: every element is one argv element on the
    // sandbox-exec (or confined) command line. There is no string here that
    // becomes a command line.
    command.args(&call.argv);
    // The child's environment is built here, not inherited: empty by
    // default, then the nested re-entry's pinned run paths, then exactly
    // the declared credentials. A planted variable in the parent's
    // environment cannot reach the child.
    command.env_clear();
    for (name, path) in pinned_env {
        command.env(*name, path);
    }
    let resolved = inject(&mut command, credentials, source)?;
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(unix)]
    command.process_group(0);

    let mut child = command
        .spawn()
        .map_err(|source| RunnerError::Spawn { source })?;
    let pid = child.id();
    let pipes = crate::proc::Pipes::take(&mut child.stdout, &mut child.stderr)
        .map_err(|source| RunnerError::WaitFailed { source })?;
    let settled = crate::proc::wait(child, call.timeout, cancellation)
        .await
        .map_err(|source| RunnerError::WaitFailed { source })?;
    let (stdout, stderr) = pipes.finish(&resolved);
    Ok(crate::proc::outcome(
        call.program,
        pid,
        &settled,
        stdout,
        stderr,
    ))
}
