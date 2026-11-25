//! The line the full-screen surface drives: the same session the line loop
//! holds, driven on a thread of its own.
//!
//! `LineChat` owns a runtime and blocks on it, because the line loop runs turns
//! from a plain function on the process's own thread. What it drives is the chat
//! layer's own composition — a run's open, a rollover's new job — which is not
//! `Send` (the engine's snapshot capture holds a writer across a suspension
//! point), so it cannot be a task on this surface's runtime either. It is
//! therefore the same shape as the line front's: one thread owns the [`Line`]
//! and blocks on its own runtime, and the loop hands it operations and reads
//! back what the session is on now.
//!
//! That is also the rule the line front states, kept for free: one operation at
//! a time, in the order they were asked for. A `/clear` typed while a turn runs
//! waits for the turn, so the answer the operator just read is part of what the
//! next run carries — exactly as `LineChat`'s own lock states it.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread::JoinHandle;

use tokio::sync::mpsc;

use super::line_resume::ResumeAsk;
use super::run_cmd_ops::settle;
use super::run_front::Forward;
use crate::agent::turn_config::PromptOverrides;
use crate::chat::{ChatSession, OpsContext, RunPort};
use crate::interactive::session::line_chat::{Line, Said};

/// What `/clear` says the run now holds: one line, in the surface's own voice.
pub(crate) const CLEARED: &str = "conversation cleared";

/// What `/compact` and `/rotate` say the run now holds, with the turns the
/// summary replaced.
fn compacted(turns: usize) -> String {
    format!("conversation compacted: {turns} turns summarised")
}

/// The run the surface is on now, as the line's thread publishes it after every
/// operation: a rollover replaces the run, and the handles taken from it.
#[derive(Default)]
pub(crate) struct Current {
    session: Option<Arc<ChatSession>>,
    /// The run's one mode cell: what a surface states a mode on.
    cell: Option<crate::agent::tools::shell_policy::ModeCell>,
    port: Option<Arc<dyn RunPort>>,
    /// The profile the run was configured on and could not open, when there is
    /// one: the launch names the absence rather than the profile.
    skipped: Option<String>,
}

impl Current {
    fn of(line: &Line) -> Self {
        Self {
            session: line.chat(),
            cell: line.mode_cell(),
            port: line.run_port(),
            skipped: line.skipped_primary().map(str::to_owned),
        }
    }

    fn publish(&mut self, line: &Line) {
        self.session = line.chat();
        self.cell = line.mode_cell();
        self.port = line.run_port();
        self.skipped = line.skipped_primary().map(str::to_owned);
    }
}

/// One operation the loop asks the line for. They are performed in the order
/// they arrive, which is the order the operator typed them.
enum Op {
    /// One operator message, as a turn of the run the session is on now.
    Turn(String),
    /// One operator message replacing the last turn: its file changes are
    /// rewound through the line's own checkpoints before the turn runs.
    Replace(String),
    /// `/continue`: the paused turn's continuation, through the session's own
    /// path.
    Continue,
    /// `/clear`, the line front's own clear, on the live run.
    Clear,
    /// `/compact` and `/rotate`: the line front's own compaction.
    Compact,
    /// A command that changed the facts a run is composed from: the open run
    /// is rolled into a successor composed from them, seeded with the
    /// conversation so far.
    Restate(PromptOverrides),
    /// `/resume <id>` and the picker's `↵`: the record this ask names is opened
    /// in place of the session's own, and the run that record states becomes the
    /// one the surface drives.
    Resume(ResumeAsk),
}

/// The session one surface drives: the line's own thread, and what it is on now.
pub(crate) struct SurfaceLine {
    ops: mpsc::UnboundedSender<Op>,
    cancels: mpsc::UnboundedSender<()>,
    current: Arc<Mutex<Current>>,
    busy: Arc<AtomicBool>,
    /// The operations handed over and not yet started: the turn after one must
    /// run after it, not beside it.
    queued: Arc<AtomicUsize>,
    worker: Option<JoinHandle<()>>,
    front: Arc<Forward>,
}

impl SurfaceLine {
    /// Takes the run the launch opened, and drives it from here on.
    pub(crate) fn new(line: Line, front: Arc<Forward>) -> Self {
        let current = Arc::new(Mutex::new(Current::of(&line)));
        let busy = Arc::new(AtomicBool::new(false));
        let queued = Arc::new(AtomicUsize::new(0));
        let (ops, queue) = mpsc::unbounded_channel::<Op>();
        let (cancels, cancelled) = mpsc::unbounded_channel::<()>();
        let worker = Some(spawn(
            line,
            Arc::clone(&front),
            Worker {
                current: Arc::clone(&current),
                busy: Arc::clone(&busy),
                queued: Arc::clone(&queued),
                queue,
                cancelled,
            },
        ));
        Self {
            ops,
            cancels,
            current,
            busy,
            queued,
            worker,
            front,
        }
    }

    /// The run the surface is on now: its session, the operations its mode is
    /// stated on, and the port its turns run through.
    pub(crate) fn current(&self) -> MutexGuard<'_, Current> {
        self.current.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The chat session the run the surface is on now drives.
    pub(crate) fn session(&self) -> Option<Arc<ChatSession>> {
        self.current().session.clone()
    }

    /// The operations this run's commands state, over its own mode cell.
    pub(crate) fn context(&self, allow_bypass: bool) -> Option<OpsContext> {
        self.current()
            .cell
            .as_ref()
            .map(|cell| OpsContext::new(cell.clone(), allow_bypass))
    }

    /// The profile this run was configured on and could not open.
    pub(crate) fn skipped_primary(&self) -> Option<String> {
        self.current().skipped.clone()
    }

    /// The port this run's turns run through, for the operations a command
    /// states without running a turn.
    pub(crate) fn run_port(&self) -> Option<Arc<dyn RunPort>> {
        self.current().port.clone()
    }

    /// Whether a turn — or a command that moves the run — holds the session now.
    pub(crate) fn busy(&self) -> bool {
        self.busy.load(Ordering::Acquire)
    }

    /// Whether an operation is waiting for the line's thread: it has been
    /// accepted and has not started yet.
    pub(crate) fn queued(&self) -> bool {
        self.queued.load(Ordering::Acquire) > 0
    }

    /// The front this surface's session talks to.
    pub(crate) fn front(&self) -> &Forward {
        &self.front
    }

    /// One operator message, as a turn of the run the session is on now. The
    /// boundary that follows the turn — the automatic compaction, and the one
    /// line it owes — is the line front's own.
    pub(crate) fn send(&self, message: String) {
        self.hand(Op::Turn(message));
    }

    /// A message edited back onto the composer: rewind the last turn's file
    /// changes through the run's own checkpoints, say what changed, and run the
    /// edited line in that turn's place.
    pub(crate) fn replace_last(&self, message: String) {
        self.hand(Op::Replace(message));
    }

    /// Asks the turn in flight to stop. The line's thread performs it while the
    /// turn is suspended, so the turn ends as the session's own cancel.
    pub(crate) fn cancel(&self) {
        let _ = self.cancels.send(());
    }

    /// `/continue`: the paused turn's continuation message.
    pub(crate) fn continue_turn(&self) {
        self.hand(Op::Continue);
    }

    /// `/clear`: the line front's own clear, on the live run.
    pub(crate) fn clear(&self) {
        self.hand(Op::Clear);
    }

    /// `/compact` and `/rotate`: the line front's own compaction, on the live
    /// run.
    pub(crate) fn compact(&self) {
        self.hand(Op::Compact);
    }

    /// A command that changed the facts a run is composed from: the next
    /// message is a turn of a run composed from the session as the command
    /// stated it. After a turn the open run is rolled into a successor seeded
    /// with the conversation so far; before the first turn it is given up, and
    /// the next message composes the run it asks for.
    pub(crate) fn restate(&self, overrides: PromptOverrides) {
        self.hand(Op::Restate(overrides));
    }

    /// `/resume`: the record this ask names, opened in place of the session's
    /// own. The operation is the line's, so it waits for a turn in flight rather
    /// than swapping the run under one.
    pub(crate) fn resume(&self, ask: ResumeAsk) {
        self.hand(Op::Resume(ask));
    }

    /// Hands one operation to the line's thread, in the order it was asked for.
    fn hand(&self, op: Op) {
        self.queued.fetch_add(1, Ordering::AcqRel);
        if self.ops.send(op).is_err() {
            self.queued.fetch_sub(1, Ordering::AcqRel);
        }
    }
}

impl Drop for SurfaceLine {
    /// The line's thread ends with the surface: the senders go, the thread
    /// returns, and the run it holds — the folder's lease with it — is dropped
    /// before this surface's launch goes on.
    fn drop(&mut self) {
        let (ops, _) = mpsc::unbounded_channel();
        drop(std::mem::replace(&mut self.ops, ops));
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

/// What the line's thread is built from: the cell it publishes into, the flag
/// a busy session is read from, and the operations waiting for it.
struct Worker {
    current: Arc<Mutex<Current>>,
    busy: Arc<AtomicBool>,
    queued: Arc<AtomicUsize>,
    queue: mpsc::UnboundedReceiver<Op>,
    cancelled: mpsc::UnboundedReceiver<()>,
}

/// Runs the line on its own thread: one operation at a time, in the order they
/// were asked for, on a runtime that drives the chat layer's non-`Send` futures.
fn spawn(line: Line, front: Arc<Forward>, worker: Worker) -> JoinHandle<()> {
    std::thread::Builder::new()
        .name("surface-line".into())
        .spawn(move || {
            let runtime = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(runtime) => runtime,
                Err(error) => {
                    front.say(format!("the session's line could not start: {error}"));
                    return;
                }
            };
            runtime.block_on(async move { drive(line, &front, worker).await });
        })
        .expect("the line's thread starts")
}

/// Performs the operations the loop asks for, one at a time.
async fn drive(line: Line, front: &Forward, worker: Worker) {
    let Worker {
        current,
        busy,
        queued,
        mut queue,
        mut cancelled,
    } = worker;
    let mut line = line;
    current
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .publish(&line);
    while let Some(op) = queue.recv().await {
        queued.fetch_sub(1, Ordering::AcqRel);
        busy.store(true, Ordering::Release);
        match op {
            Op::Turn(message) => {
                let said = turn(&mut line, front, message, &mut cancelled).await;
                settle(front, said);
            }
            Op::Replace(message) => {
                let said = line.replace_last(message, front).await;
                settle(front, said);
            }
            Op::Continue => continue_turn(&line, front).await,
            Op::Clear => {
                let said = line.clear().await;
                settle(front, said);
                front.note(CLEARED.to_owned());
            }
            Op::Compact => {
                let (said, folded) = line.compact_counted().await;
                settle(front, said);
                if folded > 0 {
                    front.note(compacted(folded));
                }
            }
            Op::Restate(overrides) => {
                let said = line.restate(overrides).await;
                settle(front, said);
            }
            Op::Resume(ask) => {
                line = super::line_resume::resume(line, front, ask).await;
            }
        }
        current
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .publish(&line);
        busy.store(false, Ordering::Release);
    }
}

/// One turn, and its own stop: the operator's cancel ends it from here, with the
/// session's own cancel, exactly as the line loop's `select` does.
async fn turn(
    line: &mut Line,
    front: &Forward,
    message: String,
    cancelled: &mut mpsc::UnboundedReceiver<()>,
) -> Said {
    let session = line.chat();
    let said = line.turn(message, front);
    tokio::pin!(said);
    tokio::select! {
        said = &mut said => said,
        Some(()) = cancelled.recv() => {
            match session {
                Some(session) => session.cancel().await,
                None => return Said::Nothing,
            }
            said.await
        }
    }
}

/// `/continue`: the paused turn's continuation, or the one refusal.
async fn continue_turn(line: &Line, front: &Forward) {
    let Some(session) = line.chat() else {
        return;
    };
    if let Err(refusal) = session.continue_turn().await {
        front.say(refusal.sentence());
    }
}
