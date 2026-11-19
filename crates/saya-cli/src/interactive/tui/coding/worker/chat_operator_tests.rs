//! A job started from the TUI asks its egressing calls in the chat's own
//! approval prompt: the pending question names the caller and the exact
//! target, waits for the person however long they take, and admits the call
//! only on a yes. Time is paused wherever a wait is involved, so no test
//! really waits.

use super::chat_prompt_fixture::{frame, helper_subject, root_subject};
use crate::agent::tools::consent::{Operator, SerialisedOperator};
use saya_agent::ApprovalChoice;
use serde_json::json;
use std::sync::Arc;
use std::time::Duration;

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

/// `AllowSession` admits this call and remembers nothing: the next identical
/// call is asked again.
#[tokio::test]
async fn an_allow_session_admits_only_this_call() {
    let mut frame = frame();
    let arguments = json!({"command": "cargo test"});
    let first = frame.ask(root_subject(), arguments.clone());
    frame.until_asked().await;
    frame.answer(ApprovalChoice::AllowSession {
        token: "command:cargo".into(),
    });
    assert!(first.await.unwrap(), "AllowSession admits this call");

    let second = frame.ask(root_subject(), arguments.clone());
    frame.until_asked().await;
    let (tool, _, _) = frame.question();
    assert_eq!(tool, root_subject(), "the next identical call asks again");
    frame.answer(ApprovalChoice::Deny);
    assert!(
        !second.await.unwrap(),
        "the remembered answer never answers it"
    );
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
