//! The background path's registry: the runs one shell holds, the id each is
//! reached by, and the bounds that keep them finite.
//!
//! A background call is the same call as a foreground one — the same path, the
//! same confinement, the same empty-then-declared environment, the same
//! process group, the same ring cap and redaction gate — with one thing
//! changed: the shell does not wait for it. The child is left running, its
//! pumps feed the run's own unread rings, and the caller gets an id.
//!
//! Three rules bound what that costs:
//!
//! - **At most [`MAX_BACKGROUND_RUNS`] at once.** The run past the cap is
//!   refused with the cap's own number, never queued; a refused start kills
//!   the child it had already spawned.
//! - **One ring per stream, per run.** A run keeps the same head-and-tail
//!   window inside the same [`OUTPUT_CAP_BYTES`] budget a joined capture
//!   honours, so an unread runaway child cannot grow a run's memory without
//!   bound. A read drains what is new and reports what the cap threw away.
//! - **Every run dies with its surface.** The surface's cancellation token
//!   kills and reaps every live group, and the registry's own drop — the
//!   surface's tools going away — does the same, so no background group
//!   outlives the job, helper or session that started it.
//!
//! [`OUTPUT_CAP_BYTES`]: crate::runner::OUTPUT_CAP_BYTES

mod run;

use std::{
    collections::HashMap,
    sync::{Arc, Mutex, MutexGuard},
    time::{Duration, Instant},
};

use saya_agent::CancellationToken;

use crate::runner::{error::RunnerError, output::OutputRing, spawn::SpawnedBackground};

use super::WorkspaceShell;
use super::call::ShellCall;
use super::detach::DetachedBackground;
use super::spawn;

use run::BackgroundRun;
pub use run::{ReadReport, RunStatus};

/// The most background runs one shell may hold at once. Four is enough for the
/// work this exists for — an install, a watch, a server — and small enough
/// that the cap, not the machine, is what bounds a runaway model.
pub const MAX_BACKGROUND_RUNS: usize = 4;

/// How often a wait re-checks the registry for an exit. The same short
/// interval a read uses, so a notice follows the exit promptly.
const WAIT_POLL_INTERVAL: Duration = Duration::from_millis(20);

impl WorkspaceShell {
    /// Starts one background call and returns the id it is reached by. The
    /// path, the validation, and the confinement are the ones
    /// [`run`](Self::run) uses — a background call takes the same decision a
    /// foreground one takes, and nothing here is a second spawn path — with
    /// one thing changed: the shell does not wait for the child.
    ///
    /// The run is bounded three ways: by the shell's cap on concurrent runs
    /// (a refusal, never a queue), by its per-stream ring, and by the shell's
    /// cancellation token, which kills and reaps every live group.
    pub async fn start_background(&self, call: ShellCall) -> Result<u64, RunnerError> {
        let sandboxed = self.resolve_lane(&call)?;
        let (validated, cwd) = self.validated_call(&call)?;
        let unread = spawn::unread_rings(self.capture_cap);
        let spawned = spawn::spawn_background(
            self.exec_target(sandboxed),
            &validated,
            &cwd,
            Arc::clone(&unread.0),
            Arc::clone(&unread.1),
            self.capture_cap,
        )?;
        self.background
            .start(spawned, unread, self.background_watch())
    }

    /// Reads a run this shell started, waiting up to `wait` for it to end.
    /// The report carries the output written since the last read — redacted
    /// and ring-bounded — and the run's status. Reading a run that has ended
    /// releases its record.
    pub async fn read_background(
        &self,
        id: u64,
        wait: Duration,
    ) -> Result<ReadReport, RunnerError> {
        self.background
            .read(id, wait, self.background_watch())
            .await
    }

    /// Kills a run's whole process group, reaps it, and returns its final
    /// account. The record is released: a killed run's slot is free for the
    /// next start.
    pub async fn kill_background(&self, id: u64) -> Result<ReadReport, RunnerError> {
        self.background.kill(id).await
    }

    /// How many background runs this shell holds: live ones, and finished ones
    /// whose last output no read has taken yet. The cap counts exactly this.
    pub fn background_runs(&self) -> usize {
        self.background.len()
    }

    /// The runs that ended since the last call, each exactly once. A run's
    /// record stays readable after its notice, so `shell_output` still answers.
    pub fn take_background_exits(&self) -> Vec<saya_agent::BackgroundExit> {
        self.background.take_exits()
    }

    /// Whether any background run this shell holds is still going.
    pub fn background_live(&self) -> bool {
        self.background.live()
    }

    /// Whether a run has ended and its notice has not been taken.
    pub fn background_notice_pending(&self) -> bool {
        self.background.notice_pending()
    }

    /// Waits until a background run ends, the deadline passes, or the run is
    /// cancelled. Returns whether a notice is ready to deliver.
    pub async fn wait_background_exit(
        &self,
        deadline: std::time::Instant,
        cancellation: &CancellationToken,
    ) -> bool {
        self.background.wait_for_exit(deadline, cancellation).await
    }

    /// The token that ends this shell's background runs: its own cancellation
    /// unless it shares a registry with a longer-lived owner (a session's
    /// per-turn shells), in which case the owner's token is the run's life.
    pub(super) fn background_watch(&self) -> &CancellationToken {
        self.background_cancellation
            .as_ref()
            .unwrap_or(&self.cancellation)
    }
}

#[derive(Default)]
struct Registry {
    next_id: u64,
    runs: HashMap<u64, Arc<BackgroundRun>>,
}

/// The live background runs of one shell: bounded, reachable by id, and gone
/// with the shell.
pub struct BackgroundRuns {
    cap: usize,
    inner: Mutex<Registry>,
}

impl Default for BackgroundRuns {
    fn default() -> Self {
        Self::new(MAX_BACKGROUND_RUNS)
    }
}

impl BackgroundRuns {
    pub(super) fn new(cap: usize) -> Self {
        Self {
            cap,
            inner: Mutex::new(Registry {
                // Ids start at one: zero reads as "no run" in a model's
                // answer, and an id is always an id.
                next_id: 1,
                runs: HashMap::new(),
            }),
        }
    }

    /// How many runs this shell holds — live ones and finished ones whose
    /// output has not been read yet. The cap counts exactly this.
    pub fn len(&self) -> usize {
        self.lock().runs.len()
    }

    /// Whether this shell holds no runs at all.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Registers a child that is already running and returns its id. At the
    /// cap the start is refused — and the child it had already spawned is
    /// killed and reaped here, because a refused start may not leave a process
    /// nothing holds.
    pub(super) fn start(
        &self,
        spawned: SpawnedBackground,
        unread: (Arc<OutputRing>, Arc<OutputRing>),
        cancellation: &CancellationToken,
    ) -> Result<u64, RunnerError> {
        let (id, run) = {
            let mut registry = self.lock();
            if registry.runs.len() >= self.cap {
                drop(registry);
                BackgroundRun::new(0, spawned, unread, Instant::now()).terminate(RunStatus::Killed);
                return Err(RunnerError::BackgroundLimitReached { limit: self.cap });
            }
            let id = registry.next_id;
            registry.next_id += 1;
            let run = Arc::new(BackgroundRun::new(id, spawned, unread, Instant::now()));
            registry.runs.insert(id, Arc::clone(&run));
            (id, run)
        };
        self.watch(run, cancellation);
        Ok(id)
    }

    /// Takes over a foreground child that outlived its timeout and returns
    /// the id it is reached by. The child, its pumps and its unread rings are
    /// exactly what a background start leaves; only the wait differs. At the
    /// cap the child is handed back untouched, so the caller can kill it and
    /// report the timeout it would have reported before.
    pub(super) fn adopt(
        &self,
        detached: DetachedBackground,
        cancellation: &CancellationToken,
    ) -> Result<u64, Box<DetachedBackground>> {
        let mut registry = self.lock();
        if registry.runs.len() >= self.cap {
            drop(registry);
            return Err(Box::new(detached));
        }
        let elapsed = detached.elapsed();
        let (spawned, unread) = detached.into_parts();
        let id = registry.next_id;
        registry.next_id += 1;
        let started = Instant::now()
            .checked_sub(elapsed)
            .unwrap_or_else(Instant::now);
        let run = Arc::new(BackgroundRun::new(id, spawned, unread, started));
        registry.runs.insert(id, Arc::clone(&run));
        drop(registry);
        self.watch(run, cancellation);
        Ok(id)
    }

    /// Reads `id`, waiting up to `wait` for it to end. A read that sees the
    /// run ended returns the last output and releases the record: the slot is
    /// free, and the next read of that id is a typed refusal rather than an
    /// empty answer.
    pub(super) async fn read(
        &self,
        id: u64,
        wait: Duration,
        cancellation: &CancellationToken,
    ) -> Result<ReadReport, RunnerError> {
        let run = self.get(id)?;
        let report = run.read(wait, cancellation).await;
        if report.status.is_terminal() {
            self.release(id);
        }
        Ok(report)
    }

    /// Kills `id`'s whole process group, reaps it, and returns its final
    /// account. The record is released either way: a kill of a run that had
    /// already ended reports that exit and frees the slot. Asynchronous for
    /// the same reason [`read`](Self::read) is — the group's death closes its
    /// pipes, and the pumps are joined before anything is reported.
    pub(super) async fn kill(&self, id: u64) -> Result<ReadReport, RunnerError> {
        let run = self.get(id)?;
        run.poll_exit();
        run.terminate(RunStatus::Killed);
        let report = run.report();
        self.release(id);
        Ok(report)
    }

    /// The runs that ended since the last call, each exactly once. A taken
    /// exit does not release the record: the notice tells the model to read
    /// it, so `shell_output` must still answer.
    pub(super) fn take_exits(&self) -> Vec<saya_agent::BackgroundExit> {
        self.runs()
            .iter()
            .filter_map(|run| run.take_exit())
            .collect()
    }

    /// Whether any run this registry holds is still going.
    pub(super) fn live(&self) -> bool {
        self.runs().iter().any(|run| run.live())
    }

    /// Whether a run ended and its notice has not been taken.
    pub(super) fn notice_pending(&self) -> bool {
        self.runs().iter().any(|run| run.notice_pending())
    }

    /// Waits until a notice is ready, no run is live, the deadline passes, or
    /// the surface is cancelled. Returns whether a notice is ready.
    pub(super) async fn wait_for_exit(
        &self,
        deadline: Instant,
        cancellation: &CancellationToken,
    ) -> bool {
        loop {
            let runs = self.runs();
            let ready = runs.iter().any(|run| run.notice_pending());
            let live = runs.iter().any(|run| run.live());
            if ready {
                return true;
            }
            if !live || cancellation.is_cancelled() || Instant::now() >= deadline {
                return false;
            }
            tokio::time::sleep(WAIT_POLL_INTERVAL).await;
        }
    }

    fn runs(&self) -> Vec<Arc<BackgroundRun>> {
        self.lock().runs.values().cloned().collect()
    }

    fn get(&self, id: u64) -> Result<Arc<BackgroundRun>, RunnerError> {
        self.lock()
            .runs
            .get(&id)
            .cloned()
            .ok_or(RunnerError::BackgroundUnknown { id })
    }

    fn release(&self, id: u64) {
        self.lock().runs.remove(&id);
    }

    /// The run's cancellation watcher: the job's token firing kills this run's
    /// group and reaps it, whatever the model does next. Without a runtime to
    /// watch from the run is still bounded — a read of a cancelled token ends
    /// it, and the registry's drop kills it — so the watcher is the prompt
    /// path, never the only one.
    fn watch(&self, run: Arc<BackgroundRun>, cancellation: &CancellationToken) {
        let cancellation = cancellation.clone();
        let Ok(handle) = tokio::runtime::Handle::try_current() else {
            return;
        };
        handle.spawn(async move {
            cancellation.cancelled().await;
            let _ = tokio::task::spawn_blocking(move || run.terminate(RunStatus::Cancelled)).await;
        });
    }

    fn lock(&self) -> MutexGuard<'_, Registry> {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

impl Drop for BackgroundRuns {
    /// Every run this registry still holds dies with its last owner. The
    /// surface's tools drop when the job, helper or session ends, so this is
    /// that ending: the groups are killed and the children reaped here, and no
    /// background process outlives the surface that started it.
    fn drop(&mut self) {
        let runs: Vec<Arc<BackgroundRun>> = self
            .inner
            .get_mut()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .runs
            .drain()
            .map(|(_, run)| run)
            .collect();
        for run in runs {
            run.terminate(RunStatus::Cancelled);
        }
    }
}
