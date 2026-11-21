//! One question pending at a time, whatever asks: a second caller waits its
//! turn rather than being refused, a stopped turn drops what is pending and what
//! is still waiting and lets the queue move on, and a front with nobody to ask
//! denies at once.

use std::sync::Arc;
use std::time::Duration;

use saya_agent::CancellationToken;
use serde_json::json;

use super::SessionOperator;
use crate::agent::tools::consent::{Answer, Operator};
use crate::chat::operator_front_fixture::{self, ask, helper_subject, root_subject};

#[tokio::test]
async fn a_second_ask_waits_for_the_first_answer() {
    let mut fixture = operator_front_fixture::interactive();
    let operator = Arc::new(SessionOperator::new(fixture.front()));
    operator.set_turn(CancellationToken::new());

    let root = ask(
        &operator,
        root_subject(),
        json!({"command": "cargo test", "workdir": "crates"}),
    );
    let first = fixture.next().await;
    assert_eq!(first.question.id, 1, "the session's first question");
    assert!(
        first.question.facts.contains("$ cargo test"),
        "the first caller's own call: {}",
        first.question.facts
    );

    // A helper asks while the root's question is still unanswered.
    let helper = ask(
        &operator,
        helper_subject(),
        json!({"url": "https://example.test/x"}),
    );
    fixture.nothing_further().await;
    assert!(
        !helper.is_finished(),
        "the second ask waits its turn; it is not refused"
    );
    assert_eq!(fixture.seen(), 1, "one question is pending at a time");

    first.answer(Answer::Once);
    assert_eq!(
        root.await.expect("the first ask finished"),
        Answer::Once,
        "the first ask gets its own answer"
    );

    let second = fixture.next().await;
    assert_eq!(
        second.question.caller,
        helper_subject(),
        "the next in line is asked once the first is answered"
    );
    assert_eq!(second.question.id, 2, "each question has its own id");
    second.answer(Answer::Session);
    assert_eq!(
        helper.await.expect("the second ask finished"),
        Answer::Session,
        "the second ask gets its own answer"
    );
    assert_eq!(
        fixture.most_at_once(),
        1,
        "the front never holds two questions at once"
    );
}

#[tokio::test]
async fn a_stopped_turn_denies_the_pending_ask_and_the_queue_moves_on() {
    let mut fixture = operator_front_fixture::interactive();
    let operator = Arc::new(SessionOperator::new(fixture.front()));
    let turn = CancellationToken::new();
    operator.set_turn(turn.clone());

    let first = ask(&operator, root_subject(), json!({"command": "cargo test"}));
    let open = fixture.next().await;
    let waiting = ask(
        &operator,
        helper_subject(),
        json!({"url": "https://example.test/x"}),
    );
    fixture.nothing_further().await;

    turn.cancel();
    assert_eq!(
        first.await.expect("the pending ask finished"),
        Answer::Deny,
        "the stopped turn's pending question is denied"
    );
    assert_eq!(
        waiting.await.expect("the waiting ask finished"),
        Answer::Deny,
        "an ask still waiting its turn is denied too"
    );
    drop(open);
    assert_eq!(
        fixture.seen(),
        1,
        "a stopped turn never reaches the front with its queued ask"
    );

    // The queue is not wedged by the stop: the next turn's ask is asked.
    operator.set_turn(CancellationToken::new());
    let next = ask(&operator, root_subject(), json!({"command": "cargo test"}));
    let question = fixture.next().await;
    assert_eq!(
        question.question.caller,
        root_subject(),
        "the queue moves on after a stopped turn"
    );
    question.answer(Answer::Deny);
    assert_eq!(
        next.await.expect("the next turn's ask finished"),
        Answer::Deny
    );
}

/// A person is answering: no deadline of the operator's ends the wait,
/// however long they take.
#[tokio::test(start_paused = true)]
async fn an_unanswered_question_has_no_deadline() {
    let mut fixture = operator_front_fixture::interactive();
    let operator = Arc::new(SessionOperator::new(fixture.front()));
    operator.set_turn(CancellationToken::new());

    let asked = ask(&operator, root_subject(), json!({"command": "cargo test"}));
    let question = fixture.next().await;
    tokio::time::advance(Duration::from_secs(3_600)).await;
    assert!(
        !asked.is_finished(),
        "no deadline ends a question a person is answering"
    );
    question.answer(Answer::Once);
    assert_eq!(
        asked.await.expect("the ask finished"),
        Answer::Once,
        "the answer still admits the call"
    );
}

/// A front with nobody to ask answers `Deny` itself, so the operator never
/// waits for a person who is not there.
#[tokio::test]
async fn a_front_with_nobody_to_ask_denies_at_once() {
    let fixture = operator_front_fixture::non_interactive();
    let operator = Arc::new(SessionOperator::new(fixture.front()));
    operator.set_turn(CancellationToken::new());

    let denied = tokio::time::timeout(
        Duration::from_secs(1),
        operator.answer(&root_subject(), &json!({"command": "cargo test"})),
    )
    .await;
    assert_eq!(
        denied.expect("a front with nobody to ask answers without waiting"),
        Answer::Deny
    );
    assert_eq!(
        fixture.seen(),
        1,
        "the question was still put to the front, which denied it"
    );
}
