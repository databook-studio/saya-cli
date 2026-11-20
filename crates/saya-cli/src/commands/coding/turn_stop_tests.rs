//! The turn's stop reading, by failure class: a guard class is a pause, and a
//! backstop — the runaway bound an operator raises — is a failure.
//!
//! This is the half of test 4 that cannot be staged as a run: a real backstop
//! needs a ledger that refuses the step, while the class it resolves to is what
//! decides the wording chat shows.

use saya_types::{CodingFailureClass as Class, CodingRunFailure};

use super::TurnStop;

/// The backstop failure the ledger's root view states, word for word, when it
/// refuses the next logical request.
fn backstop() -> CodingRunFailure {
    CodingRunFailure::root(Class::Backstop).with_guard(
        "limits.job.logical_requests",
        Some(1),
        Some(1),
        "stopped: the limits.job.logical_requests backstop was reached; raise limits.job.logical_requests only if the job is healthy",
    )
}

/// Test 4b — every progress-guard class reads as paused, and a backstop reads
/// as a failure with the ledger's own message.
#[test]
fn a_guard_class_is_a_pause_and_a_backstop_is_a_failure() {
    for class in [
        Class::NoProgress,
        Class::Repeating,
        Class::WallClock,
        Class::Cost,
    ] {
        let stop = TurnStop::of_failure(&CodingRunFailure::root(class).with_guard(
            "limits.job.no_progress_turns",
            Some(8),
            Some(9),
            "stopped: no progress for 8 of 8 turns (last progress at turn 1); resume with `saya job resume <job>` or raise limits.job.no_progress_turns",
        ));
        assert!(
            stop.paused(),
            "{class:?} is a guard stop, so the turn is paused: {stop:?}"
        );
        assert!(
            stop.message().contains("limits.job.no_progress_turns"),
            "the pause line is the guard's own: {}",
            stop.message()
        );
    }

    let failure = backstop();
    let stop = TurnStop::of_failure(&failure);
    assert!(
        !stop.paused(),
        "a backstop stop stays a failure, never a pause: {stop:?}"
    );
    assert_eq!(
        stop.message(),
        "stopped: the limits.job.logical_requests backstop was reached; raise limits.job.logical_requests only if the job is healthy",
        "the failure keeps the wording the stop was latched with"
    );
    assert_eq!(stop.dimension(), None, "no question budget stopped it");
}

/// A stop that carries no message still says which class it was, so a turn is
/// never silent about what ended it.
#[test]
fn a_message_less_failure_still_names_its_class() {
    let stop = TurnStop::of_failure(&CodingRunFailure::root(Class::ProviderError));
    assert!(!stop.paused(), "{stop:?}");
    assert!(
        stop.message().contains("provider_error"),
        "the fallback names the class: {}",
        stop.message()
    );
}

/// A question-budget stop is neither a pause nor a failure: it is chat's
/// partial wording, and it says which dimension stopped it.
#[test]
fn a_question_budget_stop_keeps_chats_partial_wording() {
    let stop = TurnStop::budget(saya_agent::BudgetDimension::LogicalRequests);
    assert!(!stop.paused(), "a budget stop is not a pause: {stop:?}");
    assert_eq!(
        stop.dimension(),
        Some(saya_agent::BudgetDimension::LogicalRequests)
    );
    assert_eq!(
        stop.message(),
        "the run stopped on its logical-requests budget; any answer it produced is partial or absent"
    );
}
