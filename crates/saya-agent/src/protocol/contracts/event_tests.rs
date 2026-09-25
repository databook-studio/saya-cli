use super::AgentEvent;

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
