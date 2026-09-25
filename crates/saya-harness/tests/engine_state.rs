//! The run state machine's contract: every legal transition lands in its
//! declared next state, and every illegal one is rejected with a typed error
//! carrying the offending state and event — never silently ignored. Terminal
//! states are terminal.

use saya_harness::engine::{RunState, RunTransition, RunTransitionError, transition};

/// The machine, enumerated independently of the implementation: these ten
/// transitions are exactly the legal ones, and every other (state, event)
/// pair of the 49 possible pairs must be rejected.
const LEGAL: &[(RunState, RunTransition, RunState)] = &[
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

const STATES: &[RunState] = &[
    RunState::Planned,
    RunState::Approved,
    RunState::Executing,
    RunState::Paused,
    RunState::Completed,
    RunState::Failed,
    RunState::Cancelled,
];

const EVENTS: &[RunTransition] = &[
    RunTransition::Approve,
    RunTransition::Begin,
    RunTransition::Pause,
    RunTransition::Resume,
    RunTransition::Complete,
    RunTransition::Fail,
    RunTransition::Cancel,
];

#[test]
fn every_legal_transition_lands_in_its_declared_next_state() {
    for (state, event, to) in LEGAL {
        assert_eq!(
            transition(*state, *event),
            Ok(*to),
            "{state:?} on {event:?} must land in {to:?}"
        );
    }
}

#[test]
fn every_illegal_transition_is_rejected_with_a_typed_error() {
    for state in STATES {
        for event in EVENTS {
            let declared = LEGAL
                .iter()
                .any(|(from, trigger, _)| from == state && trigger == event);
            if declared {
                continue;
            }
            assert_eq!(
                transition(*state, *event),
                Err(RunTransitionError::Invalid {
                    state: *state,
                    event: *event,
                }),
                "{state:?} on {event:?} is illegal and must be rejected as data, not ignored"
            );
        }
    }
}

#[test]
fn approval_is_explicit_and_cannot_repeat_or_apply_mid_flight() {
    // No implicit approval: an unapproved run cannot begin executing.
    assert_eq!(
        transition(RunState::Planned, RunTransition::Begin),
        Err(RunTransitionError::Invalid {
            state: RunState::Planned,
            event: RunTransition::Begin,
        })
    );
    // Approving an already-approved run changes nothing.
    assert_eq!(
        transition(RunState::Approved, RunTransition::Approve),
        Err(RunTransitionError::Invalid {
            state: RunState::Approved,
            event: RunTransition::Approve,
        })
    );
    // A run in flight or paused is past approval.
    for state in [RunState::Executing, RunState::Paused] {
        assert_eq!(
            transition(state, RunTransition::Approve),
            Err(RunTransitionError::Invalid {
                state,
                event: RunTransition::Approve,
            })
        );
    }
}

#[test]
fn a_paused_run_resumes_into_executing_or_is_cancelled() {
    assert_eq!(
        transition(RunState::Paused, RunTransition::Resume),
        Ok(RunState::Executing)
    );
    assert_eq!(
        transition(RunState::Paused, RunTransition::Cancel),
        Ok(RunState::Cancelled)
    );
    // Completing and failing belong to executing: a paused run must resume
    // first, so a pause is never a side door to a terminal state.
    for event in [
        RunTransition::Approve,
        RunTransition::Begin,
        RunTransition::Pause,
        RunTransition::Complete,
        RunTransition::Fail,
    ] {
        assert_eq!(
            transition(RunState::Paused, event),
            Err(RunTransitionError::Invalid {
                state: RunState::Paused,
                event,
            }),
            "paused on {event:?} must be rejected"
        );
    }
}

#[test]
fn terminal_states_are_terminal() {
    for state in [RunState::Completed, RunState::Failed, RunState::Cancelled] {
        for event in EVENTS {
            assert_eq!(
                transition(state, *event),
                Err(RunTransitionError::Invalid {
                    state,
                    event: *event,
                }),
                "nothing leaves {state:?}: {event:?} must be rejected"
            );
        }
    }
}

#[test]
fn is_terminal_marks_exactly_completed_failed_and_cancelled() {
    for state in [RunState::Completed, RunState::Failed, RunState::Cancelled] {
        assert!(state.is_terminal(), "{state:?} must be terminal");
    }
    for state in [
        RunState::Planned,
        RunState::Approved,
        RunState::Executing,
        RunState::Paused,
    ] {
        assert!(!state.is_terminal(), "{state:?} must not be terminal");
    }
}
