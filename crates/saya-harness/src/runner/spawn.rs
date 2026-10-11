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

use std::{path::PathBuf, process::Stdio, time::Duration};

use saya_agent::CancellationToken;

use crate::proc::OutputSinks;

use super::{
    env::{Credential, inject},
    error::RunnerError,
    output::ProgramOutcome,
    process_state::CommandProcessState,
    refuse::ValidatedCall,
    sandbox::RunnerSpawn,
    stdin::{StdinDelivery, StdinWriter},
    stdin_channel::StdinInputPipe,
};
use tokio::sync::watch;

#[cfg(unix)]
use std::os::unix::process::CommandExt as _;

/// How long a wait-failed child's pumps may keep running before their
/// handles are dropped and the threads detach: the join must stay bounded
/// because the child may still hold its pipes open.
const PUMP_JOIN_DEADLINE: Duration = Duration::from_secs(2);

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
    execute(
        spawn,
        call,
        pinned_env,
        credentials,
        source,
        cancellation,
        Channels {
            input: None,
            process_state: None,
            sinks: None,
        },
    )
    .await
    .map(|result| result.outcome)
}

pub(super) struct CommandSpawnResult {
    pub(super) outcome: ProgramOutcome,
    pub(super) stdin: Option<StdinDelivery>,
}

/// The optional side-channels a supervised run can carry: stdin piping, the
/// process-state publication the owner observes, and the output sinks the
/// store writes through. A plain `run_program` call carries none of them.
pub(super) struct Channels {
    pub(super) input: Option<StdinInputPipe>,
    pub(super) process_state: Option<watch::Sender<CommandProcessState>>,
    pub(super) sinks: Option<OutputSinks>,
}

pub(super) async fn run_command(
    spawn: &RunnerSpawn,
    call: ValidatedCall,
    pinned_env: &[(&'static str, PathBuf)],
    credentials: &[Credential],
    source: &dyn super::env::CredentialSource,
    cancellation: &CancellationToken,
    channels: Channels,
) -> Result<CommandSpawnResult, RunnerError> {
    execute(
        spawn,
        call,
        pinned_env,
        credentials,
        source,
        cancellation,
        channels,
    )
    .await
}

async fn execute(
    spawn: &RunnerSpawn,
    call: ValidatedCall,
    pinned_env: &[(&'static str, PathBuf)],
    credentials: &[Credential],
    source: &dyn super::env::CredentialSource,
    cancellation: &CancellationToken,
    channels: Channels,
) -> Result<CommandSpawnResult, RunnerError> {
    let Channels {
        input,
        process_state,
        sinks,
    } = channels;
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
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(unix)]
    command.process_group(0);

    let mut child = command
        .spawn()
        .map_err(|source| RunnerError::Spawn { source })?;
    let pid = child.id();
    if let Some(state) = &process_state {
        state.send_replace(CommandProcessState::Spawned { pid });
    }
    let pipes =
        crate::proc::Pipes::take_with_sinks(&mut child.stdout, &mut child.stderr, sinks, &resolved)
            .map_err(|source| RunnerError::WaitFailed { source })?;
    let mut input_failure = None;
    let mut input_writer = None;
    if let Some(input) = input {
        match child.stdin.take() {
            Some(stdin) => match StdinWriter::start(stdin, input) {
                Ok(writer) => input_writer = Some(writer),
                Err(delivery) => input_failure = Some(delivery),
            },
            None => {
                input_failure = Some(StdinDelivery::failed(
                    &input,
                    std::io::Error::new(
                        std::io::ErrorKind::BrokenPipe,
                        "child stdin pipe was unavailable",
                    ),
                ));
            }
        }
    }
    let wait = crate::proc::wait(child, call.timeout, cancellation).await;
    let writer_delivery = input_writer.map(StdinWriter::finish);
    let stdin = input_failure.or(writer_delivery);
    // The wait result is inspected before the pipes are finished: a failed
    // wait means the child may still hold its pipes open, so an
    // unconditional pump join could block forever on a stream that never
    // reaches EOF. The bounded finish seals the sinks without waiting on
    // such a pump, and the WaitFailed error is returned without publishing
    // Reaped.
    let settled = match wait {
        Ok(settled) => settled,
        Err(source) => {
            pipes.finish_bounded(&resolved, PUMP_JOIN_DEADLINE);
            return Err(RunnerError::WaitFailed { source });
        }
    };
    // The pipes are finished — pumps joined, sinks flushed — before the
    // Reaped state is published, so a consumer that sees Reaped knows the
    // output store's final flush has already happened.
    let (stdout, stderr) = pipes.finish(&resolved);
    if let Some(state) = &process_state {
        state.send_replace(CommandProcessState::Reaped { pid });
    }
    Ok(CommandSpawnResult {
        outcome: crate::proc::outcome(call.program, pid, &settled, stdout, stderr),
        stdin,
    })
}
