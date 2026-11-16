//! The foreground run that outlives its timeout: the wait that does not kill,
//! and the pieces a background run needs to take over.
//!
//! The spawn is the joined path's own — same confinement, environment, argv,
//! process group and redaction gate — with one thing changed: the wait polls
//! the child and, when the timeout fires, hands the still-running child to
//! the caller instead of killing its group. The caller (the shell) either
//! registers it as a background run or, with no slot free, kills it and
//! reports today's `timed_out`.
//!
//! The pumps feed the run's unread rings from the first byte, so output
//! written before the move is what the first `shell_output` returns.

use std::{
    path::Path,
    process::ExitStatus,
    sync::Arc,
    time::{Duration, Instant},
};

use saya_agent::CancellationToken;

use crate::{
    proc::{self, OutputSinks, Pipes, Settled, WaitEnd},
    runner::{
        RunnerError,
        env::StaticCredentialSource,
        output::{OutputRing, ProgramOutcome},
        spawn::{self, ChildEnv, ExecTarget, PUMP_JOIN_DEADLINE, SpawnedBackground, UnreadSink},
    },
};

use super::validate::ValidatedCall;

/// How often the wait re-checks a live child. The same short interval a
/// background read uses, so a command that ends is noticed promptly.
const POLL_INTERVAL: Duration = Duration::from_millis(20);

/// How a detachable foreground run ended: normally, or still going when its
/// timeout fired.
pub(super) enum ShellRun {
    Finished(ProgramOutcome),
    Detached(DetachedBackground),
}

/// A child that outlived its timeout: everything a background run is built
/// from, plus the timeout that moved it.
pub(super) struct DetachedBackground {
    program: String,
    spawned: SpawnedBackground,
    unread: (Arc<OutputRing>, Arc<OutputRing>),
    elapsed: Duration,
}

impl DetachedBackground {
    /// Kills the group and reports the run as timed out, with the output
    /// captured before the kill. The fallback when the background cap is full.
    pub(super) fn kill_timed_out(self) -> Result<ProgramOutcome, RunnerError> {
        let Self {
            program,
            spawned,
            elapsed,
            ..
        } = self;
        let SpawnedBackground {
            mut child,
            pipes,
            pgid,
            secrets,
        } = spawned;
        let _ = proc::kill_process_group(pgid);
        let status = child
            .wait()
            .map_err(|source| RunnerError::WaitFailed { source })?;
        let killed_orphans = proc::sweep_process_group(pgid);
        let (stdout, stderr) = pipes.finish(&secrets);
        let settled = Settled {
            status,
            end: WaitEnd::TimedOut,
            killed_orphans,
            duration_ms: millis(elapsed),
        };
        Ok(proc::outcome(program, pgid, &settled, stdout, stderr))
    }

    /// How long the command ran before its timeout moved it to the
    /// background, so a notice reports the run's whole life.
    pub(super) fn elapsed(&self) -> Duration {
        self.elapsed
    }

    /// The child, its pumps and the unread rings the background run takes
    /// over.
    pub(super) fn into_parts(self) -> (SpawnedBackground, (Arc<OutputRing>, Arc<OutputRing>)) {
        (self.spawned, self.unread)
    }
}

/// Runs one call and waits for it, unless the timeout fires first: then the
/// child is returned still running, with its pumps feeding `unread`.
pub(super) async fn run(
    target: ExecTarget<'_>,
    call: ValidatedCall,
    cwd: &Path,
    cancellation: &CancellationToken,
    unread: (Arc<OutputRing>, Arc<OutputRing>),
    capture_cap: usize,
) -> Result<ShellRun, RunnerError> {
    let source = StaticCredentialSource::new(Vec::<(String, String)>::new());
    let env = ChildEnv {
        credentials: &[],
        source: &source,
    };
    let (mut command, resolved) = spawn::build_command(&target, &call, &env, Some(cwd))?;
    let mut child = command
        .spawn()
        .map_err(|source| RunnerError::Spawn { source })?;
    let pgid = child.id();
    spawn::write_input(&mut child, call.stdin.as_deref());
    let sinks = OutputSinks {
        stdout: Some(Arc::new(UnreadSink(Arc::clone(&unread.0)))),
        stderr: Some(Arc::new(UnreadSink(Arc::clone(&unread.1)))),
    };
    let pipes = match Pipes::take_with_sinks(
        &mut child.stdout,
        &mut child.stderr,
        Some(sinks),
        &resolved,
        capture_cap,
    ) {
        Ok(pipes) => pipes,
        Err(source) => {
            let _ = proc::kill_process_group(pgid);
            let _ = child.wait();
            return Err(RunnerError::WaitFailed { source });
        }
    };
    let started = Instant::now();
    let deadline = tokio::time::Instant::now() + call.timeout;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                return Ok(ShellRun::Finished(finish(
                    call.program,
                    pgid,
                    status,
                    WaitEnd::Exited,
                    started,
                    pipes,
                    &resolved,
                )));
            }
            Ok(None) => {}
            Err(source) => {
                pipes.finish_bounded(&resolved, PUMP_JOIN_DEADLINE);
                return Err(RunnerError::WaitFailed { source });
            }
        }
        if cancellation.is_cancelled() {
            let _ = proc::kill_process_group(pgid);
            let status = child
                .wait()
                .map_err(|source| RunnerError::WaitFailed { source })?;
            return Ok(ShellRun::Finished(finish(
                call.program,
                pgid,
                status,
                WaitEnd::Cancelled,
                started,
                pipes,
                &resolved,
            )));
        }
        if tokio::time::Instant::now() >= deadline {
            return Ok(ShellRun::Detached(DetachedBackground {
                program: call.program,
                spawned: SpawnedBackground {
                    child,
                    pipes,
                    pgid,
                    secrets: Arc::from(resolved),
                },
                unread,
                elapsed: started.elapsed(),
            }));
        }
        tokio::time::sleep(POLL_INTERVAL).await;
    }
}

/// The settled outcome of a child the wait observed ending or killed.
fn finish(
    program: String,
    pid: u32,
    status: ExitStatus,
    end: WaitEnd,
    started: Instant,
    pipes: Pipes,
    secrets: &[String],
) -> ProgramOutcome {
    let killed_orphans = proc::sweep_process_group(pid);
    let (stdout, stderr) = pipes.finish(secrets);
    let settled = Settled {
        status,
        end,
        killed_orphans,
        duration_ms: millis(started.elapsed()),
    };
    proc::outcome(program, pid, &settled, stdout, stderr)
}

fn millis(elapsed: Duration) -> u64 {
    u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX)
}
