/// `KnowledgeOverridden` serializes under its `knowledge_overridden` type tag
/// and carries the finding's fields, with no opaque identity — the
/// DTO has no such field, by construction.
#[test]
fn knowledge_overridden_serializes_with_type_tag_and_findings() {
    use super::{AgentEvent, OverrideFindingDto};
    use saya_types::ClaimId;
    let event = AgentEvent::knowledge_overridden(vec![OverrideFindingDto {
        claim_id: ClaimId::parse("c-rental-time").unwrap(),
        kind: "default_time_column".into(),
        claimed_value: "return_date".into(),
        observed_columns: vec!["rental_date".into()],
    }]);
    let json = serde_json::to_string(&event).expect("serializes");
    assert!(json.contains(r#""type":"knowledge_overridden""#), "{json}");
    assert!(
        json.contains("return_date"),
        "carries the claimed value: {json}"
    );
    assert!(
        json.contains("rental_date"),
        "carries the observed column: {json}"
    );
    // No opaque identity field exists on the DTO; a fabricated one must not
    // appear in the serialized event.
    let fake_identity = "sha256:9f2a8c7b1e4d0a6f3c5b8e2d7a9f1c4b6e8a0d2f4c6b8e0a2d4f6c8b0e2d4f6";
    assert!(!json.contains(fake_identity), "identity leaked: {json}");
}

/// `KnowledgeLearningSkipped` serializes under its `knowledge_learning_skipped`
/// type tag and carries the reason; both reasons round-trip.
#[test]
fn knowledge_learning_skipped_serializes_with_type_tag_and_reason() {
    use super::{AgentEvent, LearningSkipReason};
    for (reason, token) in [
        (LearningSkipReason::TimedOut, "timed_out"),
        (LearningSkipReason::Failed, "failed"),
    ] {
        let event = AgentEvent::knowledge_learning_skipped(reason);
        let json = serde_json::to_string(&event).expect("serializes");
        assert!(
            json.contains(r#""type":"knowledge_learning_skipped""#),
            "type tag for {reason:?}: {json}"
        );
        assert!(
            json.contains(&format!(r#""reason":"{token}""#)),
            "reason token for {reason:?}: {json}"
        );
        let back: AgentEvent = serde_json::from_str(&json).expect("deserializes back");
        assert_eq!(back, event, "round-trips for {reason:?}");
    }
}

/// `ReasoningText` serializes under its `reasoning_text`
/// type tag and carries the text, and round-trips through the same derive set
/// as `AssistantText` (the variant it mirrors). A machine consumer reading an
/// event stream sees the chain-of-thought under its own tag, never folded into
/// the answer.
#[test]
fn reasoning_text_serializes_with_type_tag_and_round_trips() {
    use super::AgentEvent;
    let event = AgentEvent::reasoning_text("I considered the time column");
    let json = serde_json::to_string(&event).expect("serializes");
    assert!(
        json.contains(r#""type":"reasoning_text""#),
        "type tag: {json}"
    );
    assert!(
        json.contains(r#""text":"I considered the time column""#),
        "carries the text: {json}"
    );
    let back: AgentEvent = serde_json::from_str(&json).expect("deserializes back");
    assert_eq!(back, event, "round-trips");
}

/// Non-persistence survives the crossing into the CLI: the turn's
/// reasoning reaches `AgentEvent::ReasoningText`, which is the only way it
/// leaves `saya-agent`. It must never reach the persisted message types.
/// `ChatMessage` has no reasoning field, so the
/// message that gets replayed as history and shaped around for session
/// persistence carries nothing of the reasoning, however hard a caller tries
/// to put it there — there is nothing to copy. This pins the boundary the CLI-boundary slice
/// must not cross.
#[test]
fn reasoning_event_does_not_place_reasoning_on_the_replayed_message() {
    use super::{AgentEvent, ChatMessage};
    let reasoning = "the secret chain-of-thought about row values 9f3a";
    // The event the turn emits for the reasoning...
    let event = AgentEvent::reasoning_text(reasoning);
    let event_json = serde_json::to_string(&event).expect("serializes");
    assert!(
        event_json.contains(reasoning),
        "the event carries its reasoning: {event_json}"
    );
    //...and the message the turn replays as history. There is no constructor
    // that takes reasoning, and no field for it, so it cannot carry the text.
    let message = ChatMessage::text("assistant", "the answer is 42");
    let message_json = serde_json::to_string(&message).expect("serializes");
    assert!(
        !message_json.contains(reasoning),
        "the replayed message carries reasoning: {message_json}"
    );
    assert!(
        !message_json.contains("reasoning"),
        "a `reasoning` key appeared on ChatMessage: {message_json}"
    );
}
