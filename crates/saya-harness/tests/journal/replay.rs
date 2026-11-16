//! The pure `replay()` reduction over an event list: a bounded retry wins
//! over its earlier failure, usage reports stand aside from lifecycle
//! events, terminal states record their failure codes, and clock
//! observations never replace the last lifecycle event.

use saya_harness::journal::{StepState, replay};
use saya_types::{PauseReason, RunEvent, RunFailureCode};

#[test]
fn replay_records_a_bounded_retry_and_lets_usage_stand_aside() {
    let events = vec![
        RunEvent::RunStarted,
        RunEvent::PlanApproved { scopes: None },
        RunEvent::StepStarted { step: 0 },
        RunEvent::StepFailed { step: 0 },
        RunEvent::StepStarted { step: 0 },
        RunEvent::Paused {
            reason: PauseReason::BudgetExhausted,
        },
        RunEvent::Usage {
            endpoint: "deepseek".to_string(),
            tokens: Some(1),
            turns: None,
            tool_calls: None,
            cached_input_tokens: None,
            cache_creation_input_tokens: None,
        },
    ];

    let state = replay(&events);
    // The retry's `StepStarted` wins over the earlier `StepFailed`.
    assert_eq!(state.steps.get(&0), Some(&StepState::Started));
    // A pause is the last lifecycle event; a usage report after it must not
    // stand in for one.
    assert_eq!(
        state.last,
        Some(RunEvent::Paused {
            reason: PauseReason::BudgetExhausted
        })
    );
}

#[test]
fn replay_records_terminal_states_and_their_failure_code() {
    let events = vec![
        RunEvent::RunStarted,
        RunEvent::PlanApproved { scopes: None },
        RunEvent::StepStarted { step: 0 },
        RunEvent::Failed {
            code: RunFailureCode::SafetyQuery,
        },
    ];

    let state = replay(&events);
    assert_eq!(
        state.last,
        Some(RunEvent::Failed {
            code: RunFailureCode::SafetyQuery
        })
    );
}

#[test]
fn clock_observations_do_not_replace_the_last_lifecycle_event() {
    let events = [
        RunEvent::RunStarted,
        RunEvent::PlanApproved { scopes: None },
        RunEvent::WallClockObserved {
            origin_unix_ms: 1_000,
            high_water_unix_ms: 1_250,
        },
    ];
    let state = saya_harness::journal::replay(&events);
    assert!(state.started);
    assert!(state.plan_approved);
    assert_eq!(state.last, Some(RunEvent::PlanApproved { scopes: None }));
}
