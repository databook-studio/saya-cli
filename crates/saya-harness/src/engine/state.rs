//! The run lifecycle as a transition table: a pure function over
//! `(current state, event) → next state`, or a typed rejection when the pair
//! is illegal. No I/O, no async, no dependency on the journal or the store.
//!
//! The machine is `planned → approved → executing ⇄ paused → completed |
//! failed | cancelled`. Approval is explicit — no event begins execution from
//! `planned`. A paused run resumes into `executing` or is cancelled; only
//! `executing` completes or fails. Terminal states are terminal.

use std::fmt;

use thiserror::Error;

/// Where a run stands in its lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunState {
    /// Created and claimed, but not yet approved.
    Planned,
    /// The plan, scopes, and budgets were approved.
    Approved,
    /// An episode is running.
    Executing,
    /// Resumable, never a silent stop.
    Paused,
    /// Terminal: the run succeeded.
    Completed,
    /// Terminal: the run failed by cause.
    Failed,
    /// Terminal: the run was cancelled.
    Cancelled,
}

impl RunState {
    /// Terminal states are terminal: nothing leaves `completed`, `failed`,
    /// or `cancelled`.
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Cancelled)
    }
}

impl fmt::Display for RunState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::Planned => "planned",
            Self::Approved => "approved",
            Self::Executing => "executing",
            Self::Paused => "paused",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        };
        f.write_str(name)
    }
}

/// The events that drive the run machine. Payload-free: the journal event
/// that records a transition — with its failure code, pause reason, or step
/// index — belongs to the event sink, not to the state machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunTransition {
    /// Approve the plan; the only exit from `planned` besides cancellation.
    Approve,
    /// Begin executing an approved run.
    Begin,
    /// Pause an executing run, resumable.
    Pause,
    /// Resume a paused run into executing.
    Resume,
    /// Complete an executing run successfully.
    Complete,
    /// Fail an executing run terminally.
    Fail,
    /// Cancel a non-terminal run.
    Cancel,
}

impl fmt::Display for RunTransition {
    fn fmt(&self, self_: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::Approve => "approve",
            Self::Begin => "begin",
            Self::Pause => "pause",
            Self::Resume => "resume",
            Self::Complete => "complete",
            Self::Fail => "fail",
            Self::Cancel => "cancel",
        };
        self_.write_str(name)
    }
}

/// A transition the machine refuses. Carries the offending state and event as
/// data so the caller can diagnose it; it is never silently ignored.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[non_exhaustive]
#[error("run cannot {event} while {state}")]
pub enum RunTransitionError {
    /// `(state, event)` is not a legal transition.
    Invalid {
        state: RunState,
        event: RunTransition,
    },
}

/// The transition table: `(current state, event) → next state`. The legal
/// pairs mirror the run statuses the store accepts — without the store's
/// idempotent same-status re-record, which the engine must never emit.
const TRANSITIONS: &[(RunState, RunTransition, RunState)] = &[
    (
        RunState::Planned,
        RunTransition::Approve,
        RunState::Approved,
    ),
    (
        RunState::Planned,
        RunTransition::Cancel,
        RunState::Cancelled,
    ),
    (
        RunState::Approved,
        RunTransition::Begin,
        RunState::Executing,
    ),
    (
        RunState::Approved,
        RunTransition::Cancel,
        RunState::Cancelled,
    ),
    (RunState::Executing, RunTransition::Pause, RunState::Paused),
    (
        RunState::Executing,
        RunTransition::Complete,
        RunState::Completed,
    ),
    (RunState::Executing, RunTransition::Fail, RunState::Failed),
    (
        RunState::Executing,
        RunTransition::Cancel,
        RunState::Cancelled,
    ),
    (RunState::Paused, RunTransition::Resume, RunState::Executing),
    (RunState::Paused, RunTransition::Cancel, RunState::Cancelled),
];

/// Advance the run machine one step: the next state when `(state, event)` is
/// in the table, otherwise a typed rejection.
///
/// ```
/// use saya_harness::engine::{RunState, RunTransition, transition};
///
/// let approved = transition(RunState::Planned, RunTransition::Approve).unwrap();
/// assert_eq!(approved, RunState::Approved);
/// assert!(transition(approved, RunTransition::Complete).is_err());
/// ```
pub fn transition(state: RunState, event: RunTransition) -> Result<RunState, RunTransitionError> {
    let Some((_, _, next)) = TRANSITIONS
        .iter()
        .find(|(from, trigger, _)| *from == state && *trigger == event)
    else {
        return Err(RunTransitionError::Invalid { state, event });
    };
    Ok(*next)
}
