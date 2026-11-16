use super::{ChatMessage, ChatRequest, ChatResponse, ReasoningEffort, ResponseFormat};

/// `ChatMessage` may carry reasoning for the next request's replay, but the
/// field is `#[serde(skip)]`: what session persistence and the context
/// measurement serialize is byte-identical to the shape before the field
/// existed, and a deserialized message has no reasoning.
#[test]
fn chat_message_reasoning_is_never_serialized() {
    let reasoning = "I reasoned about the row values and the time column";
    let message =
        ChatMessage::text("assistant", "the answer is 42").with_reasoning(Some(reasoning.into()));
    let json = serde_json::to_string(&message).expect("serializes");
    assert!(
        !json.contains(reasoning),
        "reasoning leaked onto ChatMessage wire form: {json}"
    );
    assert!(
        !json.contains("reasoning"),
        "a `reasoning` key appeared on ChatMessage: {json}"
    );
    // The wire form is exactly role + content + tool_calls + tool_call_id.
    assert_eq!(
        json, r#"{"role":"assistant","content":"the answer is 42"}"#,
        "ChatMessage wire form changed: {json}"
    );
    let back: ChatMessage = serde_json::from_str(&json).expect("deserializes");
    assert_eq!(back.reasoning, None);
}

///
/// a response carrying reasoning serializes the reasoning under a
/// `reasoning` key, and one with `None` omits it (`skip_serializing_if`),
/// so a response written before reasoning capture (no `reasoning` key)
/// deserializes to `None` — earlier serialized responses stay valid.
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
/// (no `response_format` key) deserializes to the default `Text`, so earlier
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
/// (no `reasoning_effort` key) deserializes to the default `Default`, so earlier
/// serialized requests stay valid.
#[test]
fn chat_request_without_reasoning_effort_key_defaults_to_default() {
    let json = r#"{"model":"m","messages":[],"tools":[]}"#;
    let request: ChatRequest = serde_json::from_str(json).expect("old form deserializes");
    assert_eq!(request.reasoning_effort, ReasoningEffort::Default);
}
