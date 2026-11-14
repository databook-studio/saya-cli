use super::super::attempt::ReasoningTrace;
use super::attach;
use crate::{ChatMessage, MAX_REPLAYED_REASONING_BYTES};

fn assistant(content: &str) -> ChatMessage {
    ChatMessage::text("assistant", content)
}

fn trace(text: &str) -> Option<ReasoningTrace> {
    Some(ReasoningTrace {
        text: Some(text.into()),
        signature: None,
    })
}

#[test]
fn attach_leaves_every_other_message_untouched() {
    let earlier = assistant("old answer").with_reasoning(Some("old reasoning".into()));
    let newest = attach(assistant("new answer"), trace("new reasoning"));
    assert_eq!(newest.reasoning.as_deref(), Some("new reasoning"));
    assert_eq!(
        earlier.reasoning.as_deref(),
        Some("old reasoning"),
        "an earlier turn's copy is never cleared: clearing it rewrites the cached prefix"
    );
}

#[test]
fn a_provider_that_reports_no_reasoning_adds_nothing() {
    let newest = attach(assistant("new"), None);
    assert_eq!(newest.reasoning, None);
    assert_eq!(newest.reasoning_signature, None);
}

#[test]
fn a_signature_rides_alongside_the_text() {
    let newest = attach(
        assistant("answer"),
        Some(ReasoningTrace {
            text: Some("plan".into()),
            signature: Some("sig-1".into()),
        }),
    );
    assert_eq!(newest.reasoning.as_deref(), Some("plan"));
    assert_eq!(newest.reasoning_signature.as_deref(), Some("sig-1"));
}

#[test]
fn reasoning_is_capped_at_the_replay_limit() {
    let oversized = "x".repeat(MAX_REPLAYED_REASONING_BYTES + 100);
    let newest = attach(assistant("answer"), trace(&oversized));
    let kept = newest.reasoning.expect("reasoning is kept");
    assert_eq!(kept.len(), MAX_REPLAYED_REASONING_BYTES);
}

#[test]
fn the_cap_lands_on_a_utf8_boundary() {
    // Three-byte characters, so the 32 KiB boundary is inside a character.
    let oversized = "€".repeat(MAX_REPLAYED_REASONING_BYTES);
    let newest = attach(assistant("answer"), trace(&oversized));
    let kept = newest.reasoning.expect("reasoning is kept");
    assert!(kept.len() <= MAX_REPLAYED_REASONING_BYTES);
    assert!(kept.chars().all(|character| character == '€'));
}

#[test]
fn reasoning_and_its_signature_never_reach_the_serialized_message() {
    let message = assistant("answer").with_reasoning_trace(
        Some("secret chain-of-thought".into()),
        Some("secret-signature".into()),
    );
    let json = serde_json::to_string(&message).expect("serializes");
    assert!(
        !json.contains("secret chain-of-thought")
            && !json.contains("secret-signature")
            && !json.contains("reasoning"),
        "reasoning must never be persisted: {json}"
    );
    let back: ChatMessage = serde_json::from_str(&json).expect("deserializes");
    assert_eq!(back.reasoning, None);
    assert_eq!(back.reasoning_signature, None);
}
