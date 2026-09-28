//! Second-query admission guard: refuse a direct-SQL command or an
//! `/investigation run` replay that arrives while one is already running,
//! instead of silently replacing (and dropping) the first — and hold one
//! process-wide worker permit per started query, so the cap on background
//! workers (attached or detached) is honoured at admission time.

use super::super::replay_task::ReplayTask;
use super::super::transcript::BlockKind;
use super::super::types::App;
use super::super::worker_permits;
use std::sync::Arc;

impl App {
    /// Decides whether a direct-SQL command may start now. The primary defence
    /// against silent result loss is the queued-prompt gate: `is_busy()` now
    /// covers SQL tasks and replays, so a second command submitted while one
    /// runs is held in `pending` and dispatches only after the first finishes
    /// (both results report). This guard is the backstop: should a `SqlTask`
    /// ever reach the dispatch handler while one is already running, it is
    /// refused with a message instead of replacing the first receiver. On
    /// [`SecondSqlDecision::Start`] the guard also holds one process-wide
    /// worker permit — acquired here, before the spawn — which the caller
    /// must move into the worker thread.
    pub(crate) fn admit_second_sql(&self) -> super::SecondSqlDecision {
        self.admit_second_command(
            "A SQL command is already running — wait for it to finish before starting another.",
        )
    }

    /// The same guard for a dispatched `/investigation run` replay: a replay
    /// and a SQL task are the same one-query-at-a-time resource, so either
    /// running refuses the other.
    pub(crate) fn admit_second_replay(&self) -> super::SecondSqlDecision {
        self.admit_second_command(
            "An investigation replay is already running — wait for it to finish before \
             starting another.",
        )
    }

    fn admit_second_command(&self, message: &'static str) -> super::SecondSqlDecision {
        if self.sql_task.is_some() || self.replay_task.is_some() {
            return super::SecondSqlDecision::Reject(message);
        }
        // One attached task at a time is the rule above; the process-wide
        // worker cap is the second door, and detached workers — workers the
        // UI no longer tracks — count against it like attached ones. When
        // every permit is held the dispatch is refused, never queued.
        match worker_permits::try_acquire_worker_permit() {
            Some(permit) => super::SecondSqlDecision::Start(permit),
            None => super::SecondSqlDecision::Reject(worker_permits::CAP_REFUSAL),
        }
    }

    /// Starts an admitted replay the way the queued-prompt tick applies it:
    /// the worker spawns off-thread, the app tracks the receiver, and the
    /// status fields the bar reuses name the investigation. A refused replay
    /// is said, never silently dropped.
    pub(crate) fn start_replay(&mut self, task: ReplayTask) {
        match self.admit_second_replay() {
            super::SecondSqlDecision::Start(permit) => {
                let started = std::time::Instant::now();
                self.request.started = Some(started);
                self.request.activity = Some(format!("investigation {}", task.id));
                self.replay_task = Some((
                    super::super::replay_task::spawn(
                        permit,
                        Arc::clone(&self.runtime),
                        task.clone(),
                    ),
                    task,
                    started,
                ));
            }
            super::SecondSqlDecision::Reject(message) => {
                self.transcript.push(BlockKind::System, message)
            }
        }
    }

    /// Detaches whichever off-thread query is in flight — a direct-SQL task or
    /// a saved-investigation replay (never both: the busy gate prevents it) —
    /// and reports whether it detached one. The Esc and Ctrl+C arms detach
    /// through here so the two commands share the decision and the honest
    /// message.
    pub(crate) fn detach_in_flight_query(&mut self) -> bool {
        if self.sql_task.is_some() {
            self.detach_sql_task();
            true
        } else if self.replay_task.is_some() {
            self.detach_replay_task();
            true
        } else {
            false
        }
    }

    /// Detaches the in-flight SQL command so the UI moves on without blocking.
    /// The worker thread is not joined and the connector has no cancellation
    /// token wired here, so the query keeps running **server-side**; its result
    /// lands on a dropped channel and is discarded. The message says exactly
    /// that — it never claims the query was cancelled.
    pub(crate) fn detach_sql_task(&mut self) {
        if let Some((_, _, started)) = self.sql_task.take() {
            // Release the status fields the bar reused while the query ran.
            self.request.started = None;
            self.request.activity = None;
            self.transcript.push(
                BlockKind::System,
                format!(
                    "Detached the running query ({}s elapsed) — it may still be running on the \
                     server; its result will be discarded.",
                    started.elapsed().as_secs()
                ),
            );
        }
    }

    /// Detaches the in-flight replay the same way: the receiver is dropped,
    /// so a late completion is discarded — it never replaces the capture,
    /// the selectable query, or the transcript. The message names the
    /// investigation and says the query may still be running server-side;
    /// nothing claims cancellation.
    pub(crate) fn detach_replay_task(&mut self) {
        if let Some((_, task, started)) = self.replay_task.take() {
            self.request.started = None;
            self.request.activity = None;
            self.transcript.push(
                BlockKind::System,
                format!(
                    "Detached the running investigation {} ({}s elapsed) — the query may still \
                     be running on the server; its result will be discarded.",
                    task.id,
                    started.elapsed().as_secs()
                ),
            );
        }
    }
}

/// The dispatch decision when a second direct-SQL command (or replay) arrives
/// while one is already running — or while the process-wide worker cap is
/// full (detached workers included). A pure function over [`App`] state so
/// the result-loss behaviour is testable without a live database (see
/// [`App::admit_second_sql`]).
pub(crate) enum SecondSqlDecision {
    /// No SQL command is in flight and a worker permit was acquired — start
    /// this one. The permit must be moved into the spawned worker thread; it
    /// is released when that thread function returns.
    Start(worker_permits::WorkerPermit),
    /// One is already running, or all worker permits are held — refuse rather
    /// than silently drop the first result. The message is shown to the user.
    Reject(&'static str),
}
