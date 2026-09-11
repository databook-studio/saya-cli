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

use std::{
    io::{self, Read},
    process::{ExitStatus, Stdio},
    sync::Arc,
    time::Instant,
};

use saya_agent::CancellationToken;

use super::{
    env::{Credential, inject},
    error::RunnerError,
    output::{OUTPUT_CAP_BYTES, OutputRing, ProgramOutcome, capture},
    refuse::ValidatedCall,
    sandbox::RunnerSpawn,
};

#[cfg(unix)]
use std::os::unix::process::{CommandExt as _, ExitStatusExt as _};

/// Spawns the validated call, waits on it under the timeout and the run's
/// cancellation, and reports. Exit code is data, not an error.
pub(super) async fn run(
    spawn: &RunnerSpawn,
    call: ValidatedCall,
    credentials: &[Credential],
    source: &dyn super::env::CredentialSource,
    cancellation: &CancellationToken,
) -> Result<ProgramOutcome, RunnerError> {
    let started = Instant::now();
    let mut command = spawn.command(&call.program_path);
    // Typed argv, verbatim: every element is one argv element on the
    // sandbox-exec (or confined) command line. There is no string here that
    // becomes a command line.
    command.args(&call.argv);
    // The child's environment is built here, not inherited: empty by
    // default, then exactly the declared credentials. A planted variable in
    // the parent's environment cannot reach the child.
    command.env_clear();
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
    let stdout_ring = Arc::new(OutputRing::new(OUTPUT_CAP_BYTES));
    let stderr_ring = Arc::new(OutputRing::new(OUTPUT_CAP_BYTES));
    let stdout_pipe = take_pipe(&mut child.stdout, "stdout")?;
    let stderr_pipe = take_pipe(&mut child.stderr, "stderr")?;
    let pumps = [
        pump(Arc::clone(&stdout_ring), stdout_pipe),
        pump(Arc::clone(&stderr_ring), stderr_pipe),
    ];

    // The blocking wait runs on a blocking task; the timeout and the
    // cancellation both reach it only by killing the process group, and the
    // killed child's wait is awaited to completion before anything is
    // reported.
    let mut wait = tokio::task::spawn_blocking(move || child.wait());
    let deadline = tokio::time::Instant::now() + call.timeout;
    let (mut killed_by_timeout, mut cancelled) = (false, false);
    enum End {
        Joined(Result<Result<ExitStatus, io::Error>, tokio::task::JoinError>),
        Cancelled,
        TimedOut,
    }
    let end = tokio::select! {
        joined = &mut wait => End::Joined(joined),
        _ = cancellation.cancelled() => End::Cancelled,
        _ = tokio::time::sleep_until(deadline) => End::TimedOut,
    };
    let joined = match end {
        End::Joined(joined) => flatten_join(joined),
        End::Cancelled => {
            cancelled = true;
            kill_process_group(pid);
            flatten_join(wait.await)
        }
        End::TimedOut => {
            killed_by_timeout = true;
            kill_process_group(pid);
            flatten_join(wait.await)
        }
    };
    let status = joined.map_err(|detail| RunnerError::WaitFailed {
        source: io::Error::new(io::ErrorKind::Interrupted, detail),
    })?;

    // The group sweep: a child that exited normally can still have detached
    // descendants living in its process group. The runner leaves no
    // orphans; the report says the group was swept.
    let killed_orphans = sweep_process_group(pid);
    for pump in pumps {
        let _ = pump.join();
    }
    Ok(ProgramOutcome {
        program: call.program,
        pid,
        exit_code: status.code(),
        signal: unix_signal(&status),
        killed_by_timeout,
        cancelled,
        killed_orphans,
        duration_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
        stdout: capture(&stdout_ring, &resolved),
        stderr: capture(&stderr_ring, &resolved),
    })
}

/// Flattens the wait task's `Result<io::Result<ExitStatus>, JoinError>` into
/// the detail string the typed wait-failure carries: the wait's own I/O
/// result passes through; a panicked or abandoned wait becomes the failure.
fn flatten_join(
    joined: Result<Result<ExitStatus, io::Error>, tokio::task::JoinError>,
) -> Result<ExitStatus, String> {
    match joined {
        Ok(status) => status.map_err(|source| source.to_string()),
        Err(join) => Err(join.to_string()),
    }
}

fn take_pipe<P: Read + Send>(pipe: &mut Option<P>, name: &'static str) -> Result<P, RunnerError> {
    pipe.take().ok_or_else(|| RunnerError::WaitFailed {
        source: io::Error::new(io::ErrorKind::BrokenPipe, format!("{name} not captured")),
    })
}

/// The signal that killed the child, when one did — a unix-only property of
/// an exit status, `None` elsewhere.
fn unix_signal(status: &ExitStatus) -> Option<i32> {
    #[cfg(unix)]
    return status.signal();
    #[cfg(not(unix))]
    {
        let _ = status;
        None
    }
}

/// The child led its own process group (`process_group(0)`), so the group id
/// is the child's pid and every descendant it created — daemonized or not —
/// is a member. SIGKILL reaches the whole group, not just the child.
#[cfg(unix)]
fn kill_process_group(pgid: u32) {
    unsafe { libc::killpg(pgid as libc::pid_t, libc::SIGKILL) };
}

/// After a natural exit, probes the group and kills any live members left
/// behind. `true` means orphans were found and killed — reported, never
/// silent.
#[cfg(unix)]
fn sweep_process_group(pgid: u32) -> bool {
    // Signal 0 probes membership without killing: ESRCH means the group is
    // already gone.
    if unsafe { libc::killpg(pgid as libc::pid_t, 0) } == 0 {
        kill_process_group(pgid);
        true
    } else {
        false
    }
}

#[cfg(not(unix))]
fn sweep_process_group(pgid: u32) -> bool {
    let _ = pgid;
    false
}

fn pump(ring: Arc<OutputRing>, pipe: impl Read + Send + 'static) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || ring.pump(pipe))
}
