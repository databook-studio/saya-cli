//! What the line front says, and what it hands back.
//!
//! The wording is the contract the golden rows and the plain REPL's readers
//! depend on, so it is asserted as the rendered text — stderr in text mode —
//! rather than as a variant name.

use super::wording;
use crate::agent::tools::consent::Answer;
use crate::chat::{Budget, ChatEvent, ChatFront, ChatOutcome};
use crate::interactive::turn_summary::tally_handle;
use crate::render::RenderFormat;

fn front() -> super::LineFront {
    super::LineFront::new(RenderFormat::Text, false, tally_handle())
}

/// The stderr text one settled turn says, as a caller of the front would see
/// it: the render of the event the front says for that outcome.
fn said(outcome: &ChatOutcome) -> String {
    let Some(event) = wording(outcome) else {
        return String::new();
    };
    crate::render_event(&event, RenderFormat::Text).stderr
}

#[test]
fn an_answered_turn_says_nothing_of_its_own() {
    assert!(
        wording(&ChatOutcome::Answered).is_none(),
        "the answer streamed"
    );
    assert_eq!(said(&ChatOutcome::Answered), "");
}

#[test]
fn a_budget_stop_says_the_partial_sentence_on_stderr() {
    let outcome = ChatOutcome::Stopped(Budget {
        dimension: saya_agent::BudgetDimension::ElapsedTime,
    });
    assert_eq!(
        said(&outcome),
        "the run stopped on its elapsed-time budget; any answer it produced is partial or absent\n"
    );
    // The same sentence the shared wording states, not a second copy of it.
    assert_eq!(
        said(&outcome),
        format!(
            "{}\n",
            crate::budget_wording::budget_partial_message(saya_agent::BudgetDimension::ElapsedTime)
        )
    );
}

#[test]
fn a_pause_says_the_guards_own_message() {
    let outcome = ChatOutcome::Paused {
        message: "paused: no progress in limits.job.no_progress_turns turns".to_owned(),
    };
    assert_eq!(
        said(&outcome),
        "paused: no progress in limits.job.no_progress_turns turns\n"
    );
}

#[test]
fn a_failure_and_a_cancel_say_their_own_words() {
    assert_eq!(
        said(&ChatOutcome::Failed {
            message: "the provider refused the request".to_owned()
        }),
        "the provider refused the request\n"
    );
    assert_eq!(said(&ChatOutcome::Cancelled), "Request cancelled.\n");
}

#[test]
fn a_settled_turn_is_handed_back_once() {
    let front = front();
    assert!(front.take_outcome().is_none(), "nothing has settled yet");
    front.emit(ChatEvent::TurnEnded(ChatOutcome::Cancelled));
    assert_eq!(front.take_outcome(), Some(ChatOutcome::Cancelled));
    assert!(
        front.take_outcome().is_none(),
        "an outcome is read once, so the next turn's is its own"
    );
}

#[test]
fn a_piped_front_denies_a_question_without_asking() {
    // `can_prompt` is false for a pipe: the walk of the answer decision must
    // end in a denial with no read of stdin (which the harness has no terminal
    // for). The question's own facts never reach the screen.
    let front = front();
    let question = crate::chat::Question {
        id: 1,
        caller: "root".into(),
        facts: "in: .\n$ cargo test".into(),
        grant: None,
    };
    let answer = crate::interactive::session::lifecycle::resume::block_on(async {
        front.ask(&question).await
    });
    assert_eq!(answer, Answer::Deny);
}
