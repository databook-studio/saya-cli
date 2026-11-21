//! The reducer's arms for the blocks that end a turn: a budget stop, a pause,
//! a cancel, a failure and the model's clarifying question each land as their
//! own entry (`02` §3.7-§3.9), never as a step of the activity group.

use saya_agent::{AgentEvent, BudgetDimension};

use super::reduce_fixture::fold;
use super::{Clarification, Entry, Error, Stop};
use crate::chat::{Budget, ChatEvent, ChatOutcome};

/// The entries of the turn that ended with `outcome`.
fn after(outcome: ChatOutcome) -> Vec<Entry> {
    let view = fold(&[ChatEvent::TurnEnded(outcome)]);
    view.turns()[0].entries().to_vec()
}

/// A turn whose own budget ran out ends in a stop block of its own, naming the
/// dimension that ran out.
#[test]
fn a_budget_stop_lands_as_its_own_block() {
    let entries = after(ChatOutcome::Stopped(Budget {
        dimension: BudgetDimension::ElapsedTime,
    }));
    assert_eq!(
        entries
            .iter()
            .filter(|entry| matches!(entry, Entry::Stop(_)))
            .count(),
        1,
        "a budget stop is one block of the turn: {entries:?}"
    );
    assert!(
        matches!(
            entries.last(),
            Some(Entry::Stop(Stop::Budget {
                dimension: BudgetDimension::ElapsedTime,
                ..
            }))
        ),
        "the stop names the dimension that ran out: {entries:?}"
    );
}

/// A guard's pause keeps the guard's own sentence, so the operator reads what
/// stopped the turn and which key raises it.
#[test]
fn a_pause_keeps_the_guards_own_sentence() {
    let message = "stopped: no progress for 8 of 8 turns (limits.job.no_progress_turns)";
    let entries = after(ChatOutcome::Paused {
        message: message.to_owned(),
    });
    assert!(
        entries.iter().any(|entry| matches!(
            entry,
            Entry::Stop(Stop::Paused { message: held }) if held == message
        )),
        "the pause carries the guard's sentence unchanged: {entries:?}"
    );
}

/// The operator's cancel is a stop block too: the turn ended, and nothing after
/// that point ran.
#[test]
fn a_cancel_lands_as_a_cancel() {
    let entries = after(ChatOutcome::Cancelled);
    assert!(
        matches!(entries.last(), Some(Entry::Stop(Stop::Cancelled))),
        "a cancelled turn ends in its cancel: {entries:?}"
    );
}

/// A failure is an error block with its reason on it — never a folded line and
/// never a step of the activity group.
#[test]
fn a_failed_turn_keeps_its_reason_in_an_error_block() {
    let message = "Provider error — rate limited (429). Retried 3 times.";
    let entries = after(ChatOutcome::Failed {
        message: message.to_owned(),
    });
    assert!(
        entries.iter().any(|entry| matches!(
            entry,
            Entry::Error(Error { message: held }) if held == message
        )),
        "the error keeps its reason visible: {entries:?}"
    );
    assert_eq!(
        entries
            .iter()
            .filter(|entry| matches!(entry, Entry::Error(_)))
            .count(),
        1,
        "one failure is one error block: {entries:?}"
    );
}

/// The model's clarifying question lands as the question block, with the
/// candidate answers it offered.
#[test]
fn a_clarification_lands_as_a_question_block() {
    let question = "Which revenue do you mean?";
    let options = vec![
        "Gross order value (orders.total)".to_owned(),
        "Net of refunds (orders.total − refunds.amount)".to_owned(),
    ];
    let view = fold(&[ChatEvent::Agent(AgentEvent::ClarificationNeeded {
        question: question.to_owned(),
        options: options.clone(),
    })]);
    let entries = view.turns()[0].entries();
    assert!(
        entries.iter().any(|entry| matches!(
            entry,
            Entry::Question(Clarification { question: held, options: offered })
                if held == question && *offered == options
        )),
        "the question and its options are kept: {entries:?}"
    );
}
