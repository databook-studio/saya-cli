//! Which yes the run remembers, and whose call the question says it is: a
//! session-wide yes records the question's own grant in the run's store, one
//! call's yes records nothing, and the caller line is the asking tool's own —
//! a helper's names the helper and the job it works for.

use std::sync::Arc;

use saya_agent::CancellationToken;
use serde_json::json;

use super::SessionOperator;
use crate::agent::tools::consent::{Answer, Operator};
use crate::chat::operator_front_fixture::{ask, helper_subject, interactive, root_subject};

#[tokio::test]
async fn a_session_yes_records_the_questions_grant() {
    let mut fixture = interactive();
    let operator = Arc::new(SessionOperator::new(fixture.front()));
    operator.set_turn(CancellationToken::new());

    let asked = ask(
        &operator,
        root_subject(),
        json!({"command": "cargo test -p saya-cli"}),
    );
    let question = fixture.next().await;
    assert_eq!(
        question.question.grant.as_deref(),
        Some("shell:cargo test"),
        "the question suggests the narrowest grant its call could record"
    );
    question.answer(Answer::Session);
    assert_eq!(
        asked.await.expect("the ask finished"),
        Answer::Session,
        "a yes for the run is reported as one"
    );

    let grants = operator
        .grants()
        .expect("the operator keeps the run's store");
    assert!(
        grants.is_granted("shell:cargo test"),
        "a session-wide yes is remembered by the run: {:?}",
        grants.tokens()
    );
}

#[tokio::test]
async fn a_once_yes_records_nothing() {
    let mut fixture = interactive();
    let operator = Arc::new(SessionOperator::new(fixture.front()));
    operator.set_turn(CancellationToken::new());

    let asked = ask(
        &operator,
        root_subject(),
        json!({"command": "cargo test -p saya-cli"}),
    );
    let question = fixture.next().await;
    question.answer(Answer::Once);
    assert_eq!(
        asked.await.expect("the ask finished"),
        Answer::Once,
        "one call's yes admits that call"
    );

    let grants = operator
        .grants()
        .expect("the operator keeps the run's store");
    assert!(
        grants.is_empty(),
        "one call's yes records no grant: {:?}",
        grants.tokens()
    );
}

#[tokio::test]
async fn a_denial_records_nothing() {
    let mut fixture = interactive();
    let operator = Arc::new(SessionOperator::new(fixture.front()));
    operator.set_turn(CancellationToken::new());

    let asked = ask(&operator, root_subject(), json!({"command": "cargo test"}));
    let question = fixture.next().await;
    question.answer(Answer::Deny);
    assert_eq!(asked.await.expect("the ask finished"), Answer::Deny);

    let grants = operator
        .grants()
        .expect("the operator keeps the run's store");
    assert!(
        grants.is_empty(),
        "a denial records no grant: {:?}",
        grants.tokens()
    );
}

/// A helper's call is put to the front under the helper's own line: the helper
/// and the job it works for are named, so one front can answer for every caller.
#[tokio::test]
async fn the_caller_line_names_a_helper_and_its_job() {
    let mut fixture = interactive();
    let operator = Arc::new(SessionOperator::new(fixture.front()));
    operator.set_turn(CancellationToken::new());

    let subject = helper_subject();
    let asked = ask(&operator, subject.clone(), json!({"command": "cargo test"}));
    let question = fixture.next().await;
    assert_eq!(
        question.question.caller, subject,
        "the caller line is the asking tool's own, unchanged"
    );
    assert!(
        question.question.caller.contains("helper-2")
            && question.question.caller.contains("job job-a"),
        "the question names the helper and its job: {}",
        question.question.caller
    );
    question.answer(Answer::Once);
    assert_eq!(asked.await.expect("the ask finished"), Answer::Once);
}
