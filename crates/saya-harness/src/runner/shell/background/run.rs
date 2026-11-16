//! One background run: the child, the pumps feeding its unread rings, and the
//! group id every kill reaches.

use std::{
    process::Child,
    sync::{Arc, Mutex, MutexGuard},
    time::{Duration, Instant},
};

use saya_agent::{BackgroundExit, CancellationToken};

use crate::{
    proc::{Pipes, kill_process_group, sweep_process_group},
    runner::{
        output::{OutputRing, StreamCapture, take_capture},
        spawn::{PUMP_JOIN_DEADLINE, SpawnedBackground},
    },
};

/// How often a read re-checks a running child. Short enough that a poll sees
/// the exit as it happens, long enough not to spin.
const POLL_INTERVAL: Duration = Duration::from_millis(20);

/// How a background run stands: still going, or the honest account of how it
/// ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunStatus {
    Running,
    /// The command ended on its own. `None` when a signal ended it.
    Exited {
        exit_code: Option<i32>,
    },
    /// `shell_kill` ended it.
    Killed,
    /// The job's cancellation ended it.
    Cancelled,
}

impl RunStatus {
    /// Whether the run is still going.
    pub fn is_running(self) -> bool {
        matches!(self, Self::Running)
    }

    /// Whether the run has ended. A read that sees this releases the run's
    /// record: the caller has the status and the last output, and the slot is
    /// free for the next start.
    pub fn is_terminal(self) -> bool {
        !self.is_running()
    }

    /// The word the tool reports, so the model reads the same status the shell
    /// recorded.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Exited { .. } => "exited",
            Self::Killed => "killed",
            Self::Cancelled => "cancelled",
        }
    }

    /// The exit code, where the run ended on its own with one.
    pub fn exit_code(self) -> Option<i32> {
        match self {
            Self::Exited { exit_code } => exit_code,
            Self::Running | Self::Killed | Self::Cancelled => None,
        }
    }
}

/// What one read reported: the run's status, and the output it wrote since the
/// last read — redacted, ring-bounded, and reported truncated exactly as a
/// joined capture is.
#[derive(Debug, Clone)]
pub struct ReadReport {
    pub id: u64,
    pub status: RunStatus,
    pub stdout: StreamCapture,
    pub stderr: StreamCapture,
}

impl ReadReport {
    /// Whether either stream's unread bytes outgrew the ring.
    pub fn truncated(&self) -> bool {
        self.stdout.truncated || self.stderr.truncated
    }
}

/// The state a run's child and pumps live behind: taken together by whoever
/// ends the run, so a child is reaped exactly once.
struct RunState {
    child: Option<Child>,
    pipes: Option<Pipes>,
    status: RunStatus,
    /// When the run ended, so a late notice still reports the run's own
    /// duration rather than how long the loop took to ask.
    ended: Option<Instant>,
    /// Whether the loop has already been told this run ended. A notice is
    /// taken once; the record itself stays readable until a read or kill
    /// releases it.
    noticed: bool,
}

/// One background run: the child, the pumps feeding its unread rings, and the
/// group id every kill reaches.
pub(super) struct BackgroundRun {
    id: u64,
    pgid: u32,
    secrets: Arc<[String]>,
    unread: (Arc<OutputRing>, Arc<OutputRing>),
    started: Instant,
    state: Mutex<RunState>,
}

impl BackgroundRun {
    pub(super) fn new(
        id: u64,
        spawned: SpawnedBackground,
        unread: (Arc<OutputRing>, Arc<OutputRing>),
        started: Instant,
    ) -> Self {
        Self {
            id,
            pgid: spawned.pgid,
            secrets: spawned.secrets,
            unread,
            started,
            state: Mutex::new(RunState {
                child: Some(spawned.child),
                pipes: Some(spawned.pipes),
                status: RunStatus::Running,
                ended: None,
                noticed: false,
            }),
        }
    }

    fn status(&self) -> RunStatus {
        self.lock().status
    }

    /// Whether the run is still going. Reaps a child that ended first, so a
    /// finished run is never reported as live.
    pub(super) fn live(&self) -> bool {
        self.poll_exit();
        self.status().is_running()
    }

    /// The notice for a run that ended since the last call, or `None` while it
    /// runs, after it was already noticed, or when cancellation ended it: a
    /// cancelled run is the surface ending, not news for the model. The output
    /// is not part of the notice — the model reads it with `shell_output`.
    pub(super) fn take_exit(&self) -> Option<BackgroundExit> {
        self.poll_exit();
        let mut state = self.lock();
        if state.status.is_running() || state.noticed || state.status == RunStatus::Cancelled {
            return None;
        }
        state.noticed = true;
        Some(BackgroundExit {
            id: self.id,
            exit_code: state.status.exit_code(),
            killed: state.status == RunStatus::Killed,
            elapsed: state
                .ended
                .unwrap_or_else(Instant::now)
                .saturating_duration_since(self.started),
            output_lines: self.unread.0.lines() + self.unread.1.lines(),
        })
    }

    /// Whether an ended run's notice has not been taken yet.
    pub(super) fn notice_pending(&self) -> bool {
        self.poll_exit();
        let state = self.lock();
        state.status.is_terminal() && !state.noticed && state.status != RunStatus::Cancelled
    }

    /// Ends the run under `status`: the whole group is killed, the child is
    /// reaped, and the pumps are joined within their deadline — a descendant
    /// holding the pipes open cannot block the caller. A run that has already
    /// ended is left with the account it earned: a kill of a finished run
    /// reports the exit, not a kill that never happened.
    pub(super) fn terminate(&self, status: RunStatus) {
        let (child, pipes) = {
            let mut state = self.lock();
            if state.status.is_terminal() {
                return;
            }
            state.status = status;
            state.ended = Some(Instant::now());
            (state.child.take(), state.pipes.take())
        };
        if let Some(mut child) = child {
            // The child is a zombie until it is reaped, so its group id is
            // still ours: the kill reaches every member, and the wait collects
            // the leader before anything is reported.
            let _ = kill_process_group(self.pgid);
            let _ = child.wait();
        }
        if let Some(pipes) = pipes {
            pipes.finish_bounded(&self.secrets, PUMP_JOIN_DEADLINE);
        }
    }

    /// Reaps the child if it has ended on its own, joins its pumps, and
    /// records the exit. A run that is still going — or whose exit could not
    /// be observed — is left alone: nothing has ended, so nothing is claimed.
    pub(super) fn poll_exit(&self) {
        let pipes = {
            let mut state = self.lock();
            if !state.status.is_running() {
                return;
            }
            let Some(child) = state.child.as_mut() else {
                return;
            };
            match child.try_wait() {
                Ok(Some(status)) => {
                    state.child = None;
                    state.status = RunStatus::Exited {
                        exit_code: status.code(),
                    };
                    state.ended = Some(Instant::now());
                    state.pipes.take()
                }
                Ok(None) | Err(_) => return,
            }
        };
        // A member left in the group after a natural exit — a daemonized
        // grandchild — is swept and killed, exactly as the joined path sweeps
        // it. Nothing is left behind silently.
        sweep_process_group(self.pgid);
        if let Some(pipes) = pipes {
            pipes.finish_bounded(&self.secrets, PUMP_JOIN_DEADLINE);
        }
    }

    /// The run's account, and the output it wrote since the last read.
    pub(super) fn report(&self) -> ReadReport {
        ReadReport {
            id: self.id,
            status: self.status(),
            stdout: take_capture(&self.unread.0, &self.secrets),
            stderr: take_capture(&self.unread.1, &self.secrets),
        }
    }

    /// Waits up to `wait` for the run to end, then reports. The wait is a
    /// poll, never a deadline on the child: a read that times out leaves the
    /// run exactly as it found it.
    pub(super) async fn read(
        &self,
        wait: Duration,
        cancellation: &CancellationToken,
    ) -> ReadReport {
        let deadline = Instant::now() + wait;
        loop {
            self.poll_exit();
            if self.status().is_terminal() {
                break;
            }
            if cancellation.is_cancelled() {
                self.terminate(RunStatus::Cancelled);
                break;
            }
            if Instant::now() >= deadline {
                break;
            }
            tokio::time::sleep(POLL_INTERVAL).await;
        }
        self.report()
    }

    fn lock(&self) -> MutexGuard<'_, RunState> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}
