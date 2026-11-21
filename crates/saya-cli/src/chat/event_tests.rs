//! How one turn's stop reads on the chat side: the question budget keeps its
//! dimension, a pause keeps the guard's own sentence, a failure keeps its
//! message, a plain finish is an answer, and a cancelled turn is cancelled
//! whatever latched.

use saya_agent::BudgetDimension;

use super::{Budget, ChatOutcome};
use crate::commands::coding::TurnStop;

/// The dimensions a session's question budget can run out on.
fn dimensions() -> [BudgetDimension; 3] {
    [
        BudgetDimension::LogicalRequests,
        BudgetDimension::RequestedToolCalls,
        BudgetDimension::ElapsedTime,
    ]
}

/// The sentence a progress guard latches, naming the key to raise.
fn guard_sentence() -> &'static str {
    "stopped: no progress for 8 of 8 turns (last progress at turn 1); resume with \
     `saya job resume <job>` or raise limits.job.no_progress_turns"
}

#[test]
fn a_budget_stop_keeps_its_dimension() {
    for dimension in dimensions() {
        let outcome = ChatOutcome::from_stop(Some(TurnStop::budget(dimension)), false);
        assert!(
            matches!(outcome, ChatOutcome::Stopped(Budget { dimension: kept }) if kept == dimension),
            "a budget stop on {dimension:?} must read as stopped on that dimension: {outcome:?}"
        );
    }
}

#[test]
fn a_pause_keeps_the_guards_message() {
    let stop = TurnStop::Paused {
        message: guard_sentence().to_owned(),
    };
    assert_eq!(
        ChatOutcome::from_stop(Some(stop), false),
        ChatOutcome::Paused {
            message: guard_sentence().to_owned(),
        },
        "a pause carries the guard's own sentence, unchanged"
    );
}

#[test]
fn a_failure_keeps_its_message() {
    let sentence = "stopped: the limits.job.logical_requests backstop was reached; raise \
                    limits.job.logical_requests only if the job is healthy";
    let stop = TurnStop::Failed {
        message: sentence.to_owned(),
    };
    assert_eq!(
        ChatOutcome::from_stop(Some(stop), false),
        ChatOutcome::Failed {
            message: sentence.to_owned(),
        },
        "a failure carries its own message, unchanged"
    );
}

#[test]
fn a_turn_with_no_stop_answered() {
    assert_eq!(ChatOutcome::from_stop(None, false), ChatOutcome::Answered);
}

#[test]
fn a_cancelled_turn_is_cancelled_even_over_a_stop() {
    for stop in [
        None,
        Some(TurnStop::budget(BudgetDimension::ElapsedTime)),
        Some(TurnStop::Paused {
            message: guard_sentence().to_owned(),
        }),
        Some(TurnStop::Failed {
            message: "stopped: the provider failed".to_owned(),
        }),
    ] {
        assert_eq!(
            ChatOutcome::from_stop(stop.clone(), true),
            ChatOutcome::Cancelled,
            "a cancelled turn is cancelled whatever latched: {stop:?}"
        );
    }
}
