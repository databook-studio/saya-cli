//! Bare `/continue`: the paused turn continued, and the two refusals.
//!
//! The pause is `ChatOutcome::Paused` — a guard stop the run continues from —
//! so the tests drive a fake port that answers one turn with a pause and the
//! next with an answer: what `/continue` sends is the operator's own next
//! message, `Continue.`, and nothing else.

use std::sync::Arc;

use super::super::ops_continue::{CONTINUE_MESSAGE, NOTHING_TO_CONTINUE, TURN_RUNNING};
use super::support::{FakePort, prepared, send_in_background, session};
use super::{Admission, ChatSession, ContinueRefusal, SessionState};

/// A session whose run opened and whose only turn paused: the state `/continue`
/// continues from.
async fn paused_session() -> (Arc<ChatSession>, Arc<FakePort>) {
    let port = FakePort::new([prepared()]);
    port.pause_turns();
    let (session, _front) = session(&port);
    session.open().await;
    assert_eq!(
        session.send("look at the schema".to_owned()).await,
        Admission::Ran
    );
    assert_eq!(session.state(), SessionState::Ready);
    (session, port)
}

#[tokio::test]
async fn a_paused_turn_is_continued_with_exactly_one_turn_that_says_continue() {
    let (session, port) = paused_session().await;

    assert_eq!(session.continue_turn().await, Ok(Admission::Ran));

    assert_eq!(
        CONTINUE_MESSAGE, "Continue.",
        "the continuation turn is the literal message the decision pins"
    );
    assert_eq!(
        port.prompts(),
        vec!["look at the schema".to_owned(), "Continue.".to_owned()],
        "exactly one turn, whose user message is `Continue.`"
    );
    assert_eq!(port.calls(), 2, "one turn was continued, not two");
    assert_eq!(session.state(), SessionState::Ready);
}

#[tokio::test]
async fn a_turn_that_did_not_pause_is_refused_with_one_sentence() {
    let port = FakePort::new([prepared()]);
    let (session, _front) = session(&port);
    session.open().await;
    assert_eq!(session.send("say hi".to_owned()).await, Admission::Ran);

    assert_eq!(
        session.continue_turn().await,
        Err(ContinueRefusal::NothingToContinue)
    );

    assert_eq!(
        ContinueRefusal::NothingToContinue.sentence(),
        "nothing to continue — the last turn did not pause"
    );
    assert_eq!(
        NOTHING_TO_CONTINUE,
        "nothing to continue — the last turn did not pause"
    );
    assert_eq!(
        port.prompts(),
        vec!["say hi".to_owned()],
        "a refused /continue sends no turn"
    );
}

#[tokio::test]
async fn a_session_whose_first_turn_has_not_run_has_nothing_to_continue() {
    let port = FakePort::new([prepared()]);
    let (session, _front) = session(&port);
    session.open().await;

    assert_eq!(
        session.continue_turn().await,
        Err(ContinueRefusal::NothingToContinue)
    );
    assert!(port.prompts().is_empty(), "no turn was sent");
}

#[tokio::test]
async fn a_turn_in_flight_refuses_and_queues_nothing() {
    let port = FakePort::new([prepared()]);
    port.pause_turns();
    port.hold_turns();
    let (session, _front) = session(&port);
    session.open().await;
    let first = send_in_background(&session, "long turn");
    port.wait_started().await;

    assert_eq!(
        session.continue_turn().await,
        Err(ContinueRefusal::TurnRunning),
        "a session op never runs while a turn runs"
    );
    assert_eq!(
        ContinueRefusal::TurnRunning.sentence(),
        TURN_RUNNING,
        "the one sentence names what waits"
    );

    port.release();
    assert_eq!(
        first.await.expect("the first send completes"),
        Admission::Ran
    );
    assert_eq!(
        port.prompts(),
        vec!["long turn".to_owned()],
        "the refusal was not queued behind the turn"
    );
}
