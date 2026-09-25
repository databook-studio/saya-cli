//! The one process core both child lanes consume: the wait, the
//! process-tree kill, and the post-exit orphan sweep on unix.
//!
//! The contained lane (`runner`) and the host lane (`host`) share process
//! mechanics — how a child is waited on, how a timeout or cancellation
//! reaches its process tree, and how a detached descendant left behind after
//! a natural unix exit is swept and reported. They share nothing about
//! confinement: what is exec'd, what the child may touch, and what its
//! environment holds are each lane's own decision. A group-kill that drifted
//! between two copies is the bug that leaves orphaned children, so there is
//! exactly one copy, here.

use std::{
    io,
    process::ExitStatus,
    sync::Arc,
    time::{Duration, Instant},
};

use saya_agent::CancellationToken;

use super::runner::output::{OUTPUT_CAP_BYTES, OutputRing, ProgramOutcome, StreamCapture};

#[cfg(windows)]
mod windows;

#[cfg(unix)]
use std::os::unix::process::ExitStatusExt as _;

/// How the wait ended: the child exited on its own, the run was cancelled,
/// or the timeout fired.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WaitEnd {
    Exited,
    Cancelled,
    TimedOut,
}

/// The settled account of one child: its exit status, how the wait ended,
/// whether the post-exit sweep found orphans, and how long it ran.
pub struct Settled {
    pub status: ExitStatus,
    pub end: WaitEnd,
    pub killed_orphans: bool,
    pub duration_ms: u64,
}

/// Waits on a spawned child under the timeout and the cancellation token.
/// Timeout and cancellation terminate the child's process tree. On unix that
/// is its process group; on Windows it is `taskkill /T`. Once termination is
/// accepted, the killed child's wait is awaited to completion before anything
/// is reported, so the child is reaped before the caller sees the outcome.
///
/// Unix children lead their own group (`process_group(0)`), whose id is the
/// child pid. On Windows, cleanup covers the tree while that root process is
/// still present; a descendant that escapes after a natural parent exit has
/// no equivalent post-exit group sweep.
pub async fn wait(
    mut child: std::process::Child,
    timeout: Duration,
    cancellation: &CancellationToken,
) -> io::Result<Settled> {
    let started = Instant::now();
    let pid = child.id();
    let mut wait = tokio::task::spawn_blocking(move || child.wait());
    let deadline = tokio::time::Instant::now() + timeout;
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
    let (end, joined) = match end {
        End::Joined(joined) => (WaitEnd::Exited, flatten_join(joined)?),
        End::Cancelled => {
            kill_process_group(pid)?;
            (WaitEnd::Cancelled, flatten_join(wait.await)?)
        }
        End::TimedOut => {
            kill_process_group(pid)?;
            (WaitEnd::TimedOut, flatten_join(wait.await)?)
        }
    };
    let killed_orphans = sweep_process_group(pid);
    Ok(Settled {
        status: joined,
        end,
        killed_orphans,
        duration_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
    })
}

/// The captured stdio rings for one child: stdout and stderr pumped on
/// dedicated threads from spawn until EOF, bounded by the shared cap.
pub struct Pipes {
    stdout: Arc<OutputRing>,
    stderr: Arc<OutputRing>,
    pumps: Vec<std::thread::JoinHandle<()>>,
}

impl Pipes {
    /// Takes the child's pipes and starts the pumps. The pipes must be
    /// piped; a missing pipe is a wait failure, never a silent gap.
    pub fn take(
        stdout: &mut Option<impl io::Read + Send + 'static>,
        stderr: &mut Option<impl io::Read + Send + 'static>,
    ) -> io::Result<Self> {
        let stdout_ring = Arc::new(OutputRing::new(OUTPUT_CAP_BYTES));
        let stderr_ring = Arc::new(OutputRing::new(OUTPUT_CAP_BYTES));
        let stdout_pipe = stdout
            .take()
            .ok_or_else(|| io::Error::new(io::ErrorKind::BrokenPipe, "stdout not captured"))?;
        let stderr_pipe = stderr
            .take()
            .ok_or_else(|| io::Error::new(io::ErrorKind::BrokenPipe, "stderr not captured"))?;
        let pumps = vec![
            pump(Arc::clone(&stdout_ring), stdout_pipe),
            pump(Arc::clone(&stderr_ring), stderr_pipe),
        ];
        Ok(Self {
            stdout: stdout_ring,
            stderr: stderr_ring,
            pumps,
        })
    }

    /// Joins the pumps and redacts both rings through the single capture
    /// gate both lanes share.
    pub fn finish(self, secrets: &[String]) -> (StreamCapture, StreamCapture) {
        for pump in self.pumps {
            let _ = pump.join();
        }
        (
            super::runner::output::capture(&self.stdout, secrets),
            super::runner::output::capture(&self.stderr, secrets),
        )
    }
}

/// Builds the outcome shape both lanes report from one settled wait and its
/// captured streams: exit code is data, kills and sweeps are reported.
pub fn outcome(
    program: String,
    pid: u32,
    settled: &Settled,
    stdout: StreamCapture,
    stderr: StreamCapture,
) -> ProgramOutcome {
    ProgramOutcome {
        program,
        pid,
        exit_code: settled.status.code(),
        signal: unix_signal(&settled.status),
        killed_by_timeout: settled.end == WaitEnd::TimedOut,
        cancelled: settled.end == WaitEnd::Cancelled,
        killed_orphans: settled.killed_orphans,
        duration_ms: settled.duration_ms,
        stdout,
        stderr,
    }
}

/// Flattens the wait task's `Result<io::Result<ExitStatus>, JoinError>` into
/// the detail a wait failure carries: the wait's own I/O result passes
/// through; a panicked or abandoned wait becomes the failure.
fn flatten_join(
    joined: Result<Result<ExitStatus, io::Error>, tokio::task::JoinError>,
) -> io::Result<ExitStatus> {
    match joined {
        Ok(status) => status,
        Err(join) => Err(io::Error::new(io::ErrorKind::Interrupted, join.to_string())),
    }
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
pub fn kill_process_group(pgid: u32) -> io::Result<()> {
    if unsafe { libc::killpg(pgid as libc::pid_t, libc::SIGKILL) } == 0 {
        return Ok(());
    }
    let error = io::Error::last_os_error();
    if error.raw_os_error() == Some(libc::ESRCH) {
        Ok(())
    } else {
        Err(error)
    }
}

/// Terminates a Windows process and the descendants Windows can still find
/// below it. Windows has no Unix process groups here, so `taskkill /T` is
/// used for timeout and cancellation cleanup while the root process exists.
#[cfg(windows)]
pub fn kill_process_group(pid: u32) -> io::Result<()> {
    windows::kill_process_tree(pid)
}

#[cfg(all(not(unix), not(windows)))]
pub fn kill_process_group(_pid: u32) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "process-tree termination is unavailable on this platform",
    ))
}

/// After the wait completes, probes the group and kills any live members
/// left behind. `true` means orphans were found and killed — reported, never
/// silent.
#[cfg(unix)]
pub fn sweep_process_group(pgid: u32) -> bool {
    // Signal 0 probes membership without killing: ESRCH means the group is
    // already gone.
    if unsafe { libc::killpg(pgid as libc::pid_t, 0) } == 0 {
        let _ = kill_process_group(pgid);
        true
    } else {
        false
    }
}

#[cfg(not(unix))]
pub fn sweep_process_group(pgid: u32) -> bool {
    let _ = pgid;
    false
}

fn pump(
    ring: Arc<OutputRing>,
    pipe: impl io::Read + Send + 'static,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || ring.pump(pipe))
}
