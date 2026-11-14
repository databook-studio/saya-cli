//! The confined child's spawn path: confine, wait, kill, report. Nothing
//! here can be reached without a proven [`RunnerSpawn`] — the sandbox
//! module's verdict is consumed, never re-derived — and nothing reaches this
//! module until the workspace shell has refused every call it cannot honour.
//!
//! The two measured rules this file exists to honour:
//! - **Typed argv, never an interpolation.** Every argument arrives as
//!   exactly one argv element; the shell's `-c` string is one element, and
//!   there is no string here that becomes a second command line.
//! - **Timeout and cancellation kill the process group.** The child is
//!   created with `process_group(0)`, so it leads its own group and every
//!   descendant — daemonized or not — is a member; `libc::killpg` reaches
//!   all of them. The kill is issued, then the blocking wait is awaited to
//!   completion (the interrupt-then-await honesty pattern): the child is
//!   reaped before anything is reported, and partial output is reported as
//!   partial.

use std::{
    ffi::OsString,
    path::Path,
    process::{Child, Command, Stdio},
    sync::Arc,
    time::Duration,
};

use saya_agent::CancellationToken;

use super::{
    env::{Credential, inject},
    error::RunnerError,
    output::{OutputRing, ProgramOutcome},
    sandbox::RunnerSpawn,
    shell::ValidatedCall,
};

#[cfg(unix)]
use std::os::unix::process::CommandExt as _;

/// How long a wait-failed child's pumps may keep running before their
/// handles are dropped and the threads detach: the join must stay bounded
/// because the child may still hold its pipes open.
pub(super) const PUMP_JOIN_DEADLINE: Duration = Duration::from_secs(2);

/// The workspace shell's one entry into the spawn path: `execute` with the
/// shell's resolved working directory and whichever path the caller
/// resolved. No credentials exist on this path — a sandboxed shell's whole
/// environment is the target's `env`, which the caller built, and a host
/// shell adds none — so `env::inject` has nothing to inject either way.
pub(super) async fn run_shell(
    target: ExecTarget<'_>,
    call: ValidatedCall,
    cwd: &Path,
    cancellation: &CancellationToken,
    capture_cap: usize,
) -> Result<ProgramOutcome, RunnerError> {
    let source = super::env::StaticCredentialSource::new(Vec::<(String, String)>::new());
    execute(
        target,
        call,
        ChildEnv {
            credentials: &[],
            source: &source,
        },
        Some(cwd),
        cancellation,
        capture_cap,
    )
    .await
}

/// How `execute` builds the child's command, and what the child's
/// environment is: the two path decisions, resolved by the caller before
/// anything spawns. One `execute`, two ways to build the initial `Command` —
/// the cwd, the argv, the ring, the redaction gate, the timeout, and the
/// process-group kill are the same for both, so the host path cannot drift
/// from the sandboxed one.
pub(super) enum ExecTarget<'a> {
    /// The proven sandbox: `RunnerSpawn::command` builds the confined command
    /// line, and the child's environment is exactly `env` — empty-but-declared
    /// where the caller pins none. A planted variable in the parent's
    /// environment cannot reach this child.
    Sandboxed {
        spawn: &'a RunnerSpawn,
        env: &'a [(&'static str, OsString)],
    },
    /// The host: a plain `Command::new(program)`, inheriting this process's
    /// own environment minus `remove_env`. `secret_values` are scrubbed from
    /// the capture on top of the `redact()` pass. Nothing is confined, and no
    /// credential is added: the caller passes none on this arm.
    Host {
        remove_env: &'a [String],
        secret_values: &'a [String],
    },
}

/// The child's environment as `execute` builds it: the declared credentials
/// and the seam their values are resolved through. One value, because these
/// two are one decision — what `env::inject` adds to a child — and the
/// path's own pairs travel with the path, in [`ExecTarget`], where they
/// cannot be applied to the wrong child.
pub(super) struct ChildEnv<'a> {
    pub(super) credentials: &'a [Credential],
    pub(super) source: &'a dyn super::env::CredentialSource,
}

/// Builds the child's command line from the one path decision, the validated
/// call, and the caller's environment: the single place the confinement, the
/// empty-then-declared environment, the typed argv, and the credential
/// injection are resolved. Both spawn paths — the joined one and the
/// background one — build their child here, so neither can drift from the
/// other. The returned vector is the capture's value registry.
pub(super) fn build_command(
    target: &ExecTarget<'_>,
    call: &ValidatedCall,
    env: &ChildEnv<'_>,
    cwd: Option<&Path>,
) -> Result<(Command, Vec<String>), RunnerError> {
    let mut command = match target {
        ExecTarget::Sandboxed { spawn, env } => {
            let mut command = spawn.command(&call.program_path);
            // The child's environment is built here, not inherited: empty by
            // default, then the caller's pinned pairs, then exactly the
            // declared credentials. A planted variable in the parent's
            // environment cannot reach the child.
            command.env_clear();
            for (name, value) in *env {
                command.env(*name, value);
            }
            command
        }
        ExecTarget::Host { remove_env, .. } => {
            // The host child inherits this process's environment — that is
            // what a host run is — minus the names the caller removed. No
            // credential may be added on this arm: a host child is not
            // confined, so a saya credential in it would be a credential in
            // the caller's own session, one `echo` from the model.
            debug_assert!(
                env.credentials.is_empty(),
                "a host run carries no saya credential"
            );
            let mut command = Command::new(&call.program_path);
            for name in *remove_env {
                command.env_remove(name);
            }
            command
        }
    };
    // The spawn pins the child's cwd to the first filesystem root; the
    // workspace shell overrides it with the workdir it resolved exactly once
    // (`shell::WorkspaceShell`), which is inside that same root. Nothing
    // re-joins a path here. A host target has no spawn-set cwd at all, so
    // the shell's resolved workdir is the whole of it.
    if let Some(cwd) = cwd {
        command.current_dir(cwd);
    }
    // Typed argv, verbatim: every element is one argv element on the
    // sandbox-exec (or confined) command line. There is no string here that
    // becomes a command line.
    command.args(&call.argv);
    let mut resolved = inject(&mut command, env.credentials, env.source)?;
    // The capture's value registry: the injected credential values, plus —
    // on the host arm — the values the caller declared secret. A host child
    // holds no injected credential, so this is the whole of what it can
    // leak by echo.
    if let ExecTarget::Host { secret_values, .. } = target {
        resolved.extend(secret_values.iter().cloned());
    }
    // A call that carries no input gets the null stdin every call had before
    // this existed; one that carries input gets a pipe, written and closed by
    // `write_input` before the wait.
    command
        .stdin(if call.stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(unix)]
    command.process_group(0);
    Ok((command, resolved))
}

async fn execute(
    target: ExecTarget<'_>,
    call: ValidatedCall,
    env: ChildEnv<'_>,
    cwd: Option<&Path>,
    cancellation: &CancellationToken,
    capture_cap: usize,
) -> Result<ProgramOutcome, RunnerError> {
    let (mut command, resolved) = build_command(&target, &call, &env, cwd)?;
    let mut child = command
        .spawn()
        .map_err(|source| RunnerError::Spawn { source })?;
    let pid = child.id();
    // The payload is written and the pipe closed before anything waits, so a
    // child that reads its stdin to EOF ends rather than holding the call.
    write_input(&mut child, call.stdin.as_deref());
    let pipes = crate::proc::Pipes::take_with_sinks(
        &mut child.stdout,
        &mut child.stderr,
        None,
        &resolved,
        capture_cap,
    )
    .map_err(|source| RunnerError::WaitFailed { source })?;
    let wait = crate::proc::wait(child, call.timeout, cancellation).await;
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
    // The pipes are finished — pumps joined, sinks flushed — before anything
    // is reported, so a consumer that sees the outcome knows the capture is
    // complete.
    let (stdout, stderr) = pipes.finish(&resolved);
    Ok(crate::proc::outcome(
        call.program,
        pid,
        &settled,
        stdout,
        stderr,
    ))
}

/// Writes one call's stdin payload and closes the pipe. The write is a single
/// `write_all` of at most [`MAX_STDIN_BYTES`], which is below the pipe
/// capacity every supported platform gives a fresh pipe, so it completes
/// whether or not the child ever reads. A child that exited first closes the
/// pipe; the resulting error is the child's business, not the caller's — the
/// run's outcome already carries the exit code. The handle is dropped either
/// way, so the child sees EOF rather than waiting on a caller that has moved
/// on.
pub(super) fn write_input(child: &mut Child, input: Option<&[u8]>) {
    use std::io::Write as _;

    let Some(input) = input else {
        return;
    };
    let Some(mut stdin) = child.stdin.take() else {
        return;
    };
    let _ = stdin.write_all(input);
    let _ = stdin.flush();
}

/// The child a background start leaves running, and the pieces the shell needs
/// to keep it: the child itself (reaped by whoever ends it), the pumps still
/// running from spawn, the process-group id every kill reaches, and the
/// capture's value registry. The shell wraps this in its run record; nothing
/// else may.
pub(super) struct SpawnedBackground {
    pub(super) child: Child,
    pub(super) pipes: crate::proc::Pipes,
    pub(super) pgid: u32,
    pub(super) secrets: Arc<[String]>,
}

/// The background start's one entry into the spawn path: the same
/// [`build_command`] the joined path uses — same confinement, same environment,
/// same argv, same credential injection, same process group — then the spawn,
/// and then **no wait**. The pumps are handed the run's unread rings and are
/// left running, so the child's output accumulates, bounded, while the caller
/// holds only the handle.
///
/// A spawn that cannot take its pipes kills the group it just created before
/// returning the error: a child whose output nothing can read is exactly the
/// orphan this path may not leave behind.
pub(super) fn spawn_background(
    target: ExecTarget<'_>,
    call: &ValidatedCall,
    cwd: &Path,
    stdout: Arc<OutputRing>,
    stderr: Arc<OutputRing>,
    capture_cap: usize,
) -> Result<SpawnedBackground, RunnerError> {
    let source = super::env::StaticCredentialSource::new(Vec::<(String, String)>::new());
    let env = ChildEnv {
        credentials: &[],
        source: &source,
    };
    let (mut command, resolved) = build_command(&target, call, &env, Some(cwd))?;
    let mut child = command
        .spawn()
        .map_err(|source| RunnerError::Spawn { source })?;
    let pgid = child.id();
    let sinks = crate::proc::OutputSinks {
        stdout: Some(Arc::new(UnreadSink(Arc::clone(&stdout)))),
        stderr: Some(Arc::new(UnreadSink(stderr))),
    };
    match crate::proc::Pipes::take_with_sinks(
        &mut child.stdout,
        &mut child.stderr,
        Some(sinks),
        &resolved,
        capture_cap,
    ) {
        Ok(pipes) => Ok(SpawnedBackground {
            child,
            pipes,
            pgid,
            secrets: Arc::from(resolved),
        }),
        Err(source) => {
            // The child is a zombie here, so its group id is still ours to
            // kill and the pid cannot have been recycled.
            let _ = crate::proc::kill_process_group(pgid);
            let _ = child.wait();
            Err(RunnerError::WaitFailed { source })
        }
    }
}

/// The sink a background run attaches to each pump: every raw block the pump
/// reads is pushed straight into the run's unread ring, which a later read
/// drains. Nothing is finalised — the ring is the whole record — so the two
/// sink finalisers are no-ops.
pub(super) struct UnreadSink(pub(super) Arc<OutputRing>);

impl crate::proc::RawChunkSink for UnreadSink {
    fn observe(&self, chunk: &[u8], _secrets: &[String]) {
        self.0.push(chunk);
    }

    fn finish(&self, _secrets: &[String]) {}

    fn finish_bounded(&self, _deadline: Duration) {}
}

/// A run's unread pair: the two rings a background child's pumps feed, each
/// bounded by the same per-stream cap a joined capture honours.
pub(super) fn unread_rings(cap: usize) -> (Arc<OutputRing>, Arc<OutputRing>) {
    (
        Arc::new(OutputRing::new(cap)),
        Arc::new(OutputRing::new(cap)),
    )
}
