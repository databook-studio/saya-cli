//! A job started from the TUI asks its egressing calls in the chat's own
//! approval prompt: the pending question names the caller and the exact
//! target, waits for the person however long they take, and admits the call
//! only on a yes. Time is paused wherever a wait is involved, so no test
//! really waits.

use super::chat_operator::ChatOperator;
use super::chat_prompt_fixture::{frame, helper_subject, root_subject};
use crate::agent::tools::consent::{Answer, Operator, SerialisedOperator};
use saya_agent::ApprovalChoice;
use serde_json::{Value, json};
use std::sync::Arc;
use std::time::Duration;

/// Puts one call to a held operator, from its own task, as a job's shell does.
fn ask(
    operator: &Arc<ChatOperator>,
    subject: String,
    arguments: Value,
) -> tokio::task::JoinHandle<bool> {
    let operator: Arc<dyn Operator> = operator.clone();
    tokio::spawn(async move { operator.ask(&subject, &arguments).await })
}

#[tokio::test]
async fn a_job_egress_question_reaches_the_chat_approval_prompt() {
    let mut frame = frame();
    let asked = frame.ask(
        root_subject(),
        json!({"command": "cargo test", "workdir": "crates"}),
    );
    frame.until_asked().await;
    let (tool, detail, grant) = frame.question();
    assert_eq!(tool, root_subject(), "the prompt names who asks");
    assert!(
        detail.contains("in: crates"),
        "the working directory is shown: {detail}"
    );
    assert!(
        detail.contains("$ cargo test"),
        "the exact command is shown: {detail}"
    );
    assert!(grant.is_none(), "a job's answer remembers nothing");
    frame.answer(ApprovalChoice::AllowOnce);
    assert!(asked.await.unwrap(), "AllowOnce admits the call");
}

/// A person is sitting at the TUI: no deadline ends the wait, however long
/// they take, and the answer still admits the call.
#[tokio::test(start_paused = true)]
async fn an_unanswered_question_has_no_deadline() {
    let mut frame = frame();
    let asked = frame.ask(root_subject(), json!({"command": "cargo test"}));
    frame.until_asked().await;
    tokio::time::advance(Duration::from_secs(3_600)).await;
    assert!(
        !asked.is_finished(),
        "no deadline ends a question a person is answering"
    );
    assert!(frame.prompt_is_open(), "the question is still pending");
    frame.answer(ApprovalChoice::AllowOnce);
    assert!(asked.await.unwrap(), "the answer still admits the call");
}

#[tokio::test]
async fn a_denial_refuses_the_call() {
    let mut frame = frame();
    let asked = frame.ask(root_subject(), json!({"command": "cargo test"}));
    frame.until_asked().await;
    frame.answer(ApprovalChoice::Deny);
    assert!(!asked.await.unwrap(), "a denial is a refusal");
}

/// Cancelling the job while its question is pending refuses it: the wait ends
/// with the run, not with an approval nobody gave.
#[tokio::test]
async fn cancelling_the_job_refuses_a_pending_question() {
    let mut frame = frame();
    let asked = frame.ask(root_subject(), json!({"command": "cargo test"}));
    frame.until_asked().await;
    frame.cancel();
    assert!(!asked.await.unwrap(), "cancellation is a refusal");
}

/// `AllowSession` admits this call and remembers nothing it could not name: a
/// command that is not one plain segment has no `shell:` token, so the job's
/// store stays empty and the next call is asked again.
#[tokio::test]
async fn an_allow_session_admits_a_call_it_cannot_remember() {
    let mut frame = frame();
    let operator = Arc::new(frame.operator());
    let arguments = json!({"command": "cargo test && rm -rf x"});
    let first = ask(&operator, root_subject(), arguments.clone());
    frame.until_asked().await;
    frame.answer(ApprovalChoice::AllowSession {
        token: "shell:cargo test".into(),
    });
    assert!(first.await.unwrap(), "AllowSession admits this call");
    let grants = operator
        .grants()
        .expect("the operator keeps the job's store");
    assert!(
        grants.is_empty(),
        "a compound command names no token to remember: {:?}",
        grants.tokens()
    );

    let second = ask(&operator, root_subject(), arguments);
    frame.until_asked().await;
    let (tool, _, _) = frame.question();
    assert_eq!(tool, root_subject(), "the next identical call asks again");
    frame.answer(ApprovalChoice::Deny);
    assert!(!second.await.unwrap(), "the store never answers it");
}

/// `AllowSession` records the call's own `shell:` grant, so a later call of the
/// same prefix is answered by the store rather than by the person.
#[tokio::test]
async fn an_allow_session_remembers_the_call_for_the_job() {
    let mut frame = frame();
    let operator = Arc::new(frame.operator());
    let first = ask(
        &operator,
        root_subject(),
        json!({"command": "cargo test -p x"}),
    );
    frame.until_asked().await;
    frame.answer(ApprovalChoice::AllowSession {
        token: "shell:cargo test".into(),
    });
    assert!(first.await.unwrap(), "AllowSession admits this call");
    let grants = operator
        .grants()
        .expect("the operator keeps the job's store");
    assert!(
        grants.is_granted("shell:cargo test"),
        "the call's own token is remembered: {:?}",
        grants.tokens()
    );
}

/// `AllowOnce` is one call and no memory: the job's store stays empty. Denial
/// likewise.
#[tokio::test]
async fn an_allow_once_remembers_nothing() {
    let mut frame = frame();
    let operator = Arc::new(frame.operator());
    let first = ask(
        &operator,
        root_subject(),
        json!({"command": "cargo test -p x"}),
    );
    frame.until_asked().await;
    frame.answer(ApprovalChoice::AllowOnce);
    assert!(first.await.unwrap(), "AllowOnce admits this call");
    let grants = operator
        .grants()
        .expect("the operator keeps the job's store");
    assert!(
        grants.is_empty(),
        "one call remembers nothing: {:?}",
        grants.tokens()
    );

    let second = ask(
        &operator,
        root_subject(),
        json!({"command": "cargo test -p x"}),
    );
    frame.until_asked().await;
    frame.answer(ApprovalChoice::Deny);
    assert!(!second.await.unwrap());
    assert!(grants.is_empty(), "a denial remembers nothing either");
}

/// The store is the job's, not the operator's to answer from: the operator
/// itself still asks, so the shell policy is the one place a remembered answer
/// can pre-answer a call — after its rules and its mode.
#[tokio::test]
async fn an_allow_session_does_not_answer_the_next_call_itself() {
    let mut frame = frame();
    let operator = Arc::new(frame.operator());
    let arguments = json!({"command": "cargo test"});
    let first = ask(&operator, root_subject(), arguments.clone());
    frame.until_asked().await;
    frame.answer(ApprovalChoice::AllowSession {
        token: "shell:cargo test".into(),
    });
    assert!(first.await.unwrap(), "AllowSession admits this call");

    let second = ask(&operator, root_subject(), arguments);
    frame.until_asked().await;
    let (tool, _, _) = frame.question();
    assert_eq!(
        tool,
        root_subject(),
        "the next call is put to a person again"
    );
    frame.answer(ApprovalChoice::Deny);
    assert!(!second.await.unwrap());
}

/// Two callers ask at once — the root and a helper — and the single-pending
/// prompt is never replaced: the second question waits for the first answer.
#[tokio::test]
async fn two_questions_arrive_one_at_a_time() {
    let mut frame = frame();
    let operator: Arc<dyn Operator> = Arc::new(SerialisedOperator::new(
        Arc::new(frame.operator()),
        frame.token_ref(),
    ));
    let root = tokio::spawn({
        let operator = Arc::clone(&operator);
        async move {
            operator
                .ask(&root_subject(), &json!({"command":"cargo test"}))
                .await
        }
    });
    let helper = tokio::spawn({
        let operator = Arc::clone(&operator);
        async move {
            operator
                .ask(&helper_subject(), &json!({"url":"https://example.test/x"}))
                .await
        }
    });
    frame.until_asked().await;
    let (tool, _, _) = frame.question();
    assert_eq!(
        tool,
        root_subject(),
        "the first question is the first caller's"
    );
    frame.frames_without_a_question().await;
    let (tool, _, _) = frame.question();
    assert_eq!(
        tool,
        root_subject(),
        "the second question never replaces the first"
    );
    frame.answer(ApprovalChoice::AllowOnce);
    assert!(root.await.unwrap());

    frame.until_asked().await;
    let (tool, detail, _) = frame.question();
    assert_eq!(
        tool,
        helper_subject(),
        "the second caller is asked once the first is answered"
    );
    assert!(detail.contains("GET https://example.test/x"), "{detail}");
    frame.answer(ApprovalChoice::Deny);
    assert!(!helper.await.unwrap());
}

/// A write-shaped call is shown by the path it would change, and the answer
/// reports which yes it was — the difference the job's write ask records a
/// grant from.
#[tokio::test]
async fn a_write_question_shows_its_path_and_reports_the_yes() {
    let mut frame = frame();
    let operator: Arc<dyn Operator> = Arc::new(frame.operator());
    let asked = tokio::spawn({
        let operator = Arc::clone(&operator);
        async move {
            operator
                .answer(
                    &root_subject(),
                    &json!({"path": "src/lib.rs", "content": "changed"}),
                )
                .await
        }
    });
    frame.until_asked().await;
    let (tool, detail, _) = frame.question();
    assert_eq!(tool, root_subject(), "the prompt names who asks");
    assert!(
        detail.contains("path: src/lib.rs"),
        "the path being changed is shown: {detail}"
    );
    frame.answer(ApprovalChoice::AllowSession {
        token: "workspace-write".into(),
    });
    assert_eq!(
        asked.await.unwrap(),
        Answer::Session,
        "a yes for the run is reported as one"
    );
}
