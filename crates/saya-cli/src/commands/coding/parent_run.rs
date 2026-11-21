//! The state of one job's open root run.
//!
//! A run outlives its turns: it holds the composition every turn shares, the
//! one turn slot, the turn numbering, the per-turn inputs a surface states and
//! the stop signal of the turn running now. Closing it is what makes a further
//! message a finished run rather than another turn.

use super::CodingError;
use super::parent_turn_limits::{TurnState, TurnToken};
use crate::commands::coding::TurnStop;
use saya_agent::{CancellationReason, CancellationToken, TurnPolicy};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicU64, Ordering},
};

/// The open root run of one job: the one-time composition every turn of the
/// run shares, and whether a turn is running right now.
pub(crate) struct RootRun {
    /// The job this run belongs to: the identity its own liveness is recorded
    /// under, so a verb of the same process can ask whether a run is open.
    job: String,
    /// One turn at a time: a second is refused `Busy` rather than interleaved.
    running: AtomicBool,
    /// How many turns this run has started. Every turn records the next number
    /// at its boundary, so the run's diagnostics show the second turn as
    /// plainly as the first, in one increasing sequence.
    turns: AtomicU64,
    /// The run's composition, present from the first turn until the owner
    /// closes the run. `None` before it and after it.
    pub(super) session: Mutex<Option<Arc<super::parent_turn::RootSession>>>,
    /// The limits a surface set for this run's turns, how the turn that ended
    /// most recently ended, and the stop signal of the turn running now.
    inputs: TurnState,
}

impl RootRun {
    pub(crate) fn new(job: &saya_types::JobId) -> Self {
        Self {
            job: job.as_str().to_owned(),
            running: AtomicBool::new(false),
            turns: AtomicU64::new(0),
            session: Mutex::new(None),
            inputs: TurnState::default(),
        }
    }

    /// Sets this run's per-turn limits, or clears them: an explicit input a
    /// surface states before a turn.
    pub(super) fn set_turn_policy(&self, policy: Option<TurnPolicy>) {
        self.inputs.set_policy(policy);
    }

    /// The limits the next turn applies.
    pub(super) fn turn_policy(&self) -> Option<TurnPolicy> {
        self.inputs.policy()
    }

    /// States how the turn that just ended ended, for the surface that ran it.
    pub(super) fn note_turn_stop(&self, stop: Option<TurnStop>) {
        self.inputs.note_stop(stop);
    }

    /// How the turn that ended most recently ended.
    pub(super) fn turn_stop(&self) -> Option<TurnStop> {
        self.inputs.stop()
    }

    /// Starts a turn of this run on a child of the run's stop signal, held for
    /// as long as the turn runs.
    pub(super) fn begin_turn(&self, run: &CancellationToken) -> TurnToken<'_> {
        self.inputs.begin_turn(run)
    }

    /// The stop signal of the turn running now, or `None` between turns.
    pub(super) fn turn_cancellation(&self) -> Option<CancellationToken> {
        self.inputs.turn_cancellation()
    }

    /// Why the turn running now stopped, when its own token latched a reason.
    pub(crate) fn live_stop(&self) -> Option<CancellationReason> {
        self.inputs.live_stop()
    }

    /// Why the turn that ended most recently stopped, or `None` for one that
    /// simply finished.
    pub(crate) fn ended_stop(&self) -> Option<CancellationReason> {
        self.inputs.ended_stop()
    }

    /// The fresh, monotonic number this turn's boundary record carries: 1 for
    /// the run's first turn, one more for every turn after it.
    pub(super) fn next_turn(&self) -> u64 {
        self.turns.fetch_add(1, Ordering::AcqRel) + 1
    }

    /// Claims the job's one turn slot for as long as the guard lives.
    pub(super) fn claim(&self) -> Result<TurnGuard<'_>, CodingError> {
        if self.running.swap(true, Ordering::AcqRel) {
            return Err(CodingError::Busy);
        }
        Ok(TurnGuard { run: self })
    }

    pub(super) fn hold(&self, session: Arc<super::parent_turn::RootSession>) {
        *self.lock() = Some(session);
        // From here until the run closes, this job numbers its own events: no
        // verb that is not the run may append to the job's stream.
        super::super::live_runs::run_opened(&self.job);
    }

    #[allow(dead_code)] // only `run_next_turn` reads it, and that entry is not wired yet
    pub(super) fn session(&self) -> Option<Arc<super::parent_turn::RootSession>> {
        self.lock().clone()
    }

    /// The run is over: a further turn is refused as one on a finished run.
    pub(super) fn close(&self) {
        *self.lock() = None;
        super::super::live_runs::run_closed(&self.job);
    }

    /// A guard that closes the run when it drops: the caller holds it for as
    /// long as the run is its own, so no exit path leaves the run open.
    pub(super) fn closable(&self) -> CloseOnDrop<'_> {
        CloseOnDrop { run: self }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Option<Arc<super::parent_turn::RootSession>>> {
        self.session
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

/// Holds the job's one turn slot; dropping it lets the next turn start.
pub(super) struct TurnGuard<'a> {
    run: &'a RootRun,
}

impl Drop for TurnGuard<'_> {
    fn drop(&mut self) {
        self.run.running.store(false, Ordering::Release);
    }
}

/// Closes a run on every exit path, a dropped one included: a message can
/// never land on a run that ended, however it ended.
pub(super) struct CloseOnDrop<'a> {
    run: &'a RootRun,
}

impl Drop for CloseOnDrop<'_> {
    fn drop(&mut self) {
        self.run.close();
    }
}
