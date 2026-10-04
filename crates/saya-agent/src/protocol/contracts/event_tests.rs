use super::{AgentEvent, ProviderContinuationPhase};

/// `KnowledgeLearningDisabled` serializes under its
/// `knowledge_learning_disabled` type tag and carries the model and the miss
/// count — the ndjson shape a machine consumer reads.
#[test]
fn knowledge_learning_disabled_serializes_with_type_tag_model_and_misses() {
    let event = AgentEvent::knowledge_learning_disabled("test-model", 2);
    let json = serde_json::to_string(&event).expect("serializes");
    assert!(
        json.contains(r#""type":"knowledge_learning_disabled""#),
        "type tag: {json}"
    );
    assert!(json.contains(r#""model":"test-model""#), "model: {json}");
    assert!(json.contains(r#""misses":2"#), "misses: {json}");
}

/// `ProviderContinuation` serializes under its `provider_continuation` type
/// tag with a snake_case phase and carries the index/limit, and round-trips.
/// The wire shape a machine consumer reads is pinned exactly:
/// `{"type":"provider_continuation","phase":"continuing","index":1,"limit":3}`.
#[test]
fn provider_continuation_serializes_with_type_tag_phase_and_round_trips() {
    for (phase, token) in [
        (ProviderContinuationPhase::Continuing, "continuing"),
        (ProviderContinuationPhase::Exhausted, "exhausted"),
    ] {
        let event = AgentEvent::provider_continuation(phase, 1, 3);
        let json = serde_json::to_string(&event).expect("serializes");
        assert!(
            json.contains(r#""type":"provider_continuation""#),
            "type tag for {phase:?}: {json}"
        );
        assert!(
            json.contains(&format!(r#""phase":"{token}""#)),
            "phase token for {phase:?}: {json}"
        );
        assert!(
            json.contains(r#""index":1"#) && json.contains(r#""limit":3"#),
            "carries the counts for {phase:?}: {json}"
        );
        let back: AgentEvent = serde_json::from_str(&json).expect("deserializes back");
        assert_eq!(back, event, "round-trips for {phase:?}");
    }
    let pinned: AgentEvent = serde_json::from_str(
        r#"{"type":"provider_continuation","phase":"continuing","index":1,"limit":3}"#,
    )
    .expect("the pinned wire shape deserializes");
    assert_eq!(
        pinned,
        AgentEvent::provider_continuation(ProviderContinuationPhase::Continuing, 1, 3),
        "the pinned wire shape is exactly the variant"
    );
}
