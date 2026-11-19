//! The refusals the chat prompt keeps from the panel it replaces: a target the
//! prompt cannot show in full raises no question at all, control and invisible
//! bytes are replaced before they reach the person's screen, and a question
//! that cannot be queued is refused rather than held.

use super::super::CodingMessage;
use super::chat_prompt_fixture::{frame, root_subject};
use crate::agent::tools::consent::Operator;
use saya_agent::CancellationToken;
use serde_json::json;
use std::sync::mpsc;

/// A target the prompt cannot show in full — or no target at all — is a
/// refusal, never a question with nothing to answer for.
#[tokio::test]
async fn a_target_the_prompt_cannot_show_is_refused_without_asking() {
    let mut frame = frame();
    let operator = frame.operator();
    for arguments in [
        json!({}),
        json!({"command": 7}),
        json!({"url": true}),
        json!({"command": "x".repeat(2_049)}),
        json!({"url": "x".repeat(2_049)}),
    ] {
        assert!(
            !operator.ask(&root_subject(), &arguments).await,
            "the operator is never asked about a target it cannot read: {arguments}"
        );
    }
    frame.frames_without_a_question().await;
    assert!(!frame.prompt_is_open(), "nothing was asked");
}

/// The target is model-authored: control and invisible bytes reach the prompt
/// replaced, never raw.
#[tokio::test]
async fn control_and_invisible_bytes_never_reach_the_prompt() {
    let mut frame = frame();
    let asked = frame.ask(
        root_subject(),
        json!({"command": "printf 'a\u{1b}[2Jb\rc\u{0}d'\u{202E}\u{200B}"}),
    );
    frame.until_asked().await;
    let (_tool, detail, _grant) = frame.question();
    for (name, raw) in [
        ("ESC", '\u{1b}'),
        ("CR", '\r'),
        ("NUL", '\u{0}'),
        ("bidi override", '\u{202E}'),
        ("zero width space", '\u{200B}'),
    ] {
        assert!(
            !detail.contains(raw),
            "the raw {name} must not reach the prompt: {detail:?}"
        );
    }
    assert!(
        detail.contains("$ printf 'a\u{FFFD}[2Jb"),
        "the command is shown, replaced: {detail}"
    );
    frame.answer(saya_agent::ApprovalChoice::Deny);
    assert!(!asked.await.unwrap());
}

/// A question that cannot be queued is refused before any wait begins.
#[tokio::test]
async fn a_full_queue_refuses_without_asking() {
    let (sender, queue) = mpsc::sync_channel(1);
    sender
        .try_send(CodingMessage::Activity("queued".into()))
        .expect("room for one event");
    let operator = super::chat_operator::ChatOperator::new(sender, CancellationToken::new());
    assert!(
        !operator
            .ask(&root_subject(), &json!({"command": "cargo test"}))
            .await,
        "a question that cannot be queued is a refusal"
    );
    assert!(matches!(queue.try_recv(), Ok(CodingMessage::Activity(text)) if text == "queued"));
}
