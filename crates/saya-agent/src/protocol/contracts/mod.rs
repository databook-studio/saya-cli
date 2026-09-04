//! Agent protocol contracts — the request/response shapes, event stream, tool
//! definitions, and errors that cross the `saya-agent` → `saya-cli` boundary.
//! Declarations and the public surface live here; the types are split into
//! per-concern submodules.

mod approval;
mod chat;
mod error;
mod event;
mod knowledge;
mod tool;

pub use approval::{AllowReadOnlyApproval, ApprovalDecider};
pub use chat::{
    AgentRequest, ChatMessage, ChatRequest, ChatResponse, ContextBlock, ReasoningEffort,
    ResponseFormat, ToolCall, ToolMetadata,
};
pub use error::{ProviderError, ToolError};
pub use event::{AgentEvent, LearningSkipReason};
pub use knowledge::{
    KnowledgeOutcome, OverrideFindingDto, ProposedClaimDto, SuppliedClaimDto, SuppliedContractDto,
};
pub use tool::{LocalStateEffect, ToolDefinition, ToolEffect, ToolExecutor};

#[cfg(test)]
mod tests {
    use super::{
        ChatMessage, ChatRequest, ChatResponse, LocalStateEffect, ReasoningEffort, ResponseFormat,
    };

    /// `ChatMessage` — what gets
    /// replayed to the provider as history and what session persistence is
    /// shaped around — has **no** reasoning field. A `ChatMessage` carrying
    /// reasoning-shaped content serializes to the same wire form today had
    /// before reasoning was captured, because there is nowhere on the type to put the
    /// reasoning. If a field is ever added here, this test fails and the
    /// reviewer is forced to justify making reasoning persistable and replayable.
    #[test]
    fn chat_message_has_no_reasoning_field_and_wire_is_unchanged() {
        let message = ChatMessage::text("assistant", "the answer is 42");
        let json = serde_json::to_string(&message).expect("serializes");
        // The reasoning this turn *would have* carried. It must not appear in
        // the message's wire form — there is no field for it.
        let reasoning = "I reasoned about the row values and the time column";
        assert!(
            !json.contains(reasoning),
            "reasoning leaked onto ChatMessage wire form: {json}"
        );
        assert!(
            !json.contains("reasoning"),
            "a `reasoning` key appeared on ChatMessage: {json}"
        );
        // The wire form is exactly role + content + tool_calls + tool_call_id,
        // the shape from before reasoning was captured.
        assert_eq!(
            json, r#"{"role":"assistant","content":"the answer is 42"}"#,
            "ChatMessage wire form changed: {json}"
        );
    }

    ///
    /// a response carrying reasoning serializes the reasoning under a
    /// `reasoning` key, and one with `None` omits it (`skip_serializing_if`),
    /// so a response written before reasoning capture (no `reasoning` key)
    /// deserializes to `None` — old serialized responses stay valid.
    #[test]
    fn chat_response_carries_reasoning_and_round_trips() {
        let with = ChatResponse {
            message: ChatMessage::text("assistant", "ok"),
            reasoning: Some("because the column is nullable".into()),
            ..Default::default()
        };
        let json = serde_json::to_string(&with).expect("serializes");
        assert!(
            json.contains(r#""reasoning":"because the column is nullable""#),
            "reasoning must appear on ChatResponse: {json}"
        );
        let back: ChatResponse = serde_json::from_str(&json).expect("deserializes back");
        assert_eq!(back.reasoning, with.reasoning);

        // `None` is omitted from the wire (a present-but-null would read as
        // "the model reasoned and produced nothing", which is a different
        // fact than "the provider reported no reasoning").
        let without = ChatResponse {
            message: ChatMessage::text("assistant", "ok"),
            ..Default::default()
        };
        let json = serde_json::to_string(&without).expect("serializes");
        assert!(
            !json.contains("reasoning"),
            "None reasoning must be off the wire: {json}"
        );
        // A response from before reasoning capture (no `reasoning` key) deserializes to `None`.
        let old = r#"{"message":{"role":"assistant","content":"ok"}}"#;
        let old_response: ChatResponse = serde_json::from_str(old).expect("old form deserializes");
        assert_eq!(old_response.reasoning, None);
    }

    /// `ResponseFormat::Text` is the default — the whole point of leaving the
    /// field unset on the main loop's request. If this regresses, the opt-in rule
    /// (JSON mode is for the extraction call only) breaks silently.
    #[test]
    fn response_format_defaults_to_text() {
        assert_eq!(ResponseFormat::default(), ResponseFormat::Text);
        // A request built with struct-update (`..Default::default()`) — the
        // shape the main loop and the call sites use — defaults to `Text`.
        let request = ChatRequest {
            model: "m".into(),
            messages: Vec::new(),
            tools: Vec::new(),
            ..Default::default()
        };
        assert_eq!(request.response_format, ResponseFormat::Text);
    }

    /// `ResponseFormat` is provider-neutral intent, not an OpenAI wire spelling:
    /// it serializes as `text` / `json_object` (snake_case) and round-trips, so a
    /// serialized `ChatRequest` stays readable and stable.
    #[test]
    fn response_format_round_trips_through_snake_case() {
        for (variant, expected) in [
            (ResponseFormat::Text, "text"),
            (ResponseFormat::JsonObject, "json_object"),
        ] {
            let text = serde_json::to_string(&variant).expect("serializes");
            assert_eq!(text, format!("\"{expected}\""), "{variant:?}");
            let back: ResponseFormat = serde_json::from_str(&text).expect("deserializes back");
            assert_eq!(back, variant, "{variant:?}");
        }
    }

    /// The back-compat guarantee: a `ChatRequest` serialized by an older build
    /// (no `response_format` key) deserializes to the default `Text`, so old
    /// serialized requests stay valid.
    #[test]
    fn chat_request_without_response_format_key_defaults_to_text() {
        let json = r#"{"model":"m","messages":[],"tools":[]}"#;
        let request: ChatRequest = serde_json::from_str(json).expect("old form deserializes");
        assert_eq!(request.response_format, ResponseFormat::Text);
    }

    /// `ReasoningEffort::Default` is the default — it means "send nothing", so
    /// the main loop's request (built with `..Default::default()`) leaves effort
    /// to the endpoint. If this regresses, the main loop would silently ask for
    /// less thinking; the main loop keeps real reasoning.
    #[test]
    fn reasoning_effort_defaults_to_default_send_nothing() {
        assert_eq!(ReasoningEffort::default(), ReasoningEffort::Default);
        let request = ChatRequest {
            model: "m".into(),
            messages: Vec::new(),
            tools: Vec::new(),
            ..Default::default()
        };
        assert_eq!(request.reasoning_effort, ReasoningEffort::Default);
    }

    /// `ReasoningEffort` is provider-neutral intent, not an OpenAI wire spelling:
    /// it serializes as `default` / `minimal` / `low` / `medium` / `high`
    /// (snake_case) and round-trips, so a serialized `ChatRequest` stays readable
    /// and stable.
    #[test]
    fn reasoning_effort_round_trips_through_snake_case() {
        for (variant, expected) in [
            (ReasoningEffort::Default, "default"),
            (ReasoningEffort::Minimal, "minimal"),
            (ReasoningEffort::Low, "low"),
            (ReasoningEffort::Medium, "medium"),
            (ReasoningEffort::High, "high"),
        ] {
            let text = serde_json::to_string(&variant).expect("serializes");
            assert_eq!(text, format!("\"{expected}\""), "{variant:?}");
            let back: ReasoningEffort = serde_json::from_str(&text).expect("deserializes back");
            assert_eq!(back, variant, "{variant:?}");
        }
    }

    /// The back-compat guarantee: a `ChatRequest` serialized by an older build
    /// (no `reasoning_effort` key) deserializes to the default `Default`, so old
    /// serialized requests stay valid.
    #[test]
    fn chat_request_without_reasoning_effort_key_defaults_to_default() {
        let json = r#"{"model":"m","messages":[],"tools":[]}"#;
        let request: ChatRequest = serde_json::from_str(json).expect("old form deserializes");
        assert_eq!(request.reasoning_effort, ReasoningEffort::Default);
    }

    /// The back-compat guarantee: a `ToolEffect` serialized by an older build
    /// (no `local_state` key) deserializes to the default `None`.
    #[test]
    fn tool_effect_without_local_state_key_defaults_to_none() {
        let json = r#"{
            "database_data": false,
            "external_side_effect": false,
            "requires_approval": false
        }"#;
        let effect: super::ToolEffect = serde_json::from_str(json).expect("old form deserializes");
        assert_eq!(effect.local_state, LocalStateEffect::None);
    }

    /// Each variant round-trips through snake_case.
    #[test]
    fn local_state_effect_round_trips_through_snake_case() {
        for (variant, expected) in [
            (LocalStateEffect::None, "none"),
            (LocalStateEffect::Read, "read"),
            (LocalStateEffect::WriteCandidate, "write_candidate"),
        ] {
            let text = serde_json::to_string(&variant).expect("serializes");
            assert_eq!(text, format!("\"{expected}\""), "{variant:?}");
            let back: LocalStateEffect = serde_json::from_str(&text).expect("deserializes back");
            assert_eq!(back, variant, "{variant:?}");
        }
    }

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
        let fake_identity =
            "sha256:9f2a8c7b1e4d0a6f3c5b8e2d7a9f1c4b6e8a0d2f4c6b8e0a2d4f6c8b0e2d4f6";
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
}
