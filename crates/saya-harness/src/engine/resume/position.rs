//! Where the journal says a run stood when its process died: the position
//! analysis the resume entry point branches on.

use saya_types::RunEvent;

use crate::engine::state::RunState;
use crate::journal::JournalState;

/// Where the journal says the run stood when its process died.
pub(super) enum Position {
    /// No `RunStarted` — the directory holds no run.
    NoRun,
    /// Started, never approved: nothing may run without approval.
    Unapproved,
    /// The journal's last lifecycle event is terminal.
    Terminal(RunState),
    /// Approved, never began — the first step begins fresh.
    Approved,
    /// Paused by a recorded reason; resume records the resume.
    Paused,
    /// A step was in flight: the process died executing.
    MidFlight,
}

pub(super) fn position(state: &JournalState) -> Position {
    if !state.started {
        return Position::NoRun;
    }
    if !state.plan_approved {
        return Position::Unapproved;
    }
    match state.last {
        Some(RunEvent::Completed) => Position::Terminal(RunState::Completed),
        Some(RunEvent::Failed { .. }) => Position::Terminal(RunState::Failed),
        Some(RunEvent::Cancelled) => Position::Terminal(RunState::Cancelled),
        Some(RunEvent::Paused { .. }) => Position::Paused,
        // A step event with no terminal or pause after it — the process
        // died mid-flight, and its last step restarts from its beginning.
        _ if !state.steps.is_empty() => Position::MidFlight,
        _ => Position::Approved,
    }
}
