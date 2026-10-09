/// A resumed session's replayed history carries no tool calls or
/// tool-call ids, so the agent's history validator (which rejects both)
/// accepts it on every provider — the persisted tool record lives beside
/// the conversation, not inside the replayed messages. This is why
/// `validate()` is left unchanged.
#[test]
fn a_resumed_session_with_tool_calls_produces_valid_history_for_every_provider() {
    let mut session = SessionState::new("s1", None, "m");
    session.record_turn(
        "q",
        "a",
        true,
        vec![ToolMetadata {
            name: "bounded_sql_query".into(),
            status: "completed".into(),
            arguments: r#"{"sql":"SELECT 1"}"#.into(),
            result_shape: Some(ToolResultShape {
                row_count: 1,
                columns: vec!["c".into()],
            }),
        }],
    );
    let history = session.provider_history();
    assert_eq!(history.len(), 2);
    assert_eq!(history[0].role, "user");
    assert_eq!(history[1].role, "assistant");
    for message in &history {
        assert!(
            message.tool_calls.is_empty(),
            "replayed history carried tool_calls: {message:?}"
        );
        assert!(
            message.tool_call_id.is_none(),
            "replayed history carried a tool_call_id: {message:?}"
        );
    }
}

/// The persisted mode carries the task posture and nothing secret: `plan`
/// round-trips through `redacted()`, and the serialized form carries the
/// mode word with no secret-shaped keys beside the ones the redaction
/// tests already pin. The task list rides the same record: it round-trips
/// too, and its titles and notes — plain conversation metadata — add no
/// new secret-shaped key.
#[test]
fn redacted_carries_the_mode_and_no_secret_shaped_data() {
    use saya_agent::AgentMode;
    use saya_types::{SessionTask, SessionTaskList, TaskStatus};
    let mut planned = SessionState::new("s-mode", None, "m");
    planned.agent_mode = AgentMode::Plan.as_str().into();
    planned.task_list = SessionTaskList::new(vec![
        SessionTask::with_note(
            "profile the tables",
            TaskStatus::InProgress,
            Some("halfway"),
        )
        .unwrap(),
    ])
    .unwrap();
    let saved = planned.redacted();
    assert_eq!(saved.agent_mode, "plan");
    assert_eq!(saved.approval_mode, "ask");
    assert_eq!(saved.task_list, planned.task_list);
    let json = serde_json::to_string(&saved).unwrap();
    assert!(
        json.contains(r#""agent_mode":"plan""#),
        "mode missing: {json}"
    );
    assert!(
        json.contains("profile the tables"),
        "the task list must ride the record: {json}"
    );
    for key in ["password", "api_key", "rows", "reasoning"] {
        assert!(
            !json.contains(key),
            "`{key}` leaked into the record: {json}"
        );
    }

    let built = SessionState::new("s-build", None, "m");
    assert_eq!(built.redacted().agent_mode, "build");
    assert!(
        built.redacted().task_list.is_empty(),
        "a fresh session persists an empty list"
    );
}

/// A session with no tool calls serializes to today's shape: no
/// `arguments` or `result_shape` keys appear, and the turn is just user +
/// assistant. A tool record with empty arguments and no shape is also
/// byte-compatible with a file written before this feature (the new keys
/// are `skip_serializing_if`-omitted), so old and new files interoperate.
#[test]
fn a_session_with_no_tool_calls_is_unchanged_from_today() {
    let mut session = SessionState::new("s1", None, "m");
    session.record_turn("hello", "hi there", false, vec![]);
    let json = serde_json::to_string(&session.redacted()).unwrap();
    assert!(!json.contains("arguments"), "new key leaked: {json}");
    assert!(!json.contains("result_shape"), "new key leaked: {json}");
    assert!(
        json.contains(r#""user":"hello""#),
        "user turn missing: {json}"
    );
    assert!(
        json.contains(r#""assistant":"hi there""#),
        "assistant turn missing: {json}"
    );

    // A tool record with no extra data serializes as name + status only.
    let mut with_tool = SessionState::new("s2", None, "m");
    with_tool.record_turn(
        "q",
        "a",
        true,
        vec![ToolMetadata {
            name: "schema_discovery".into(),
            status: "completed".into(),
            ..Default::default()
        }],
    );
    let tool_json = serde_json::to_string(&with_tool.redacted()).unwrap();
    assert!(
        tool_json.contains(r#""tools":[{"name":"schema_discovery","status":"completed"}]"#),
        "a tool record with no extra data should be name + status only: {tool_json}"
    );
    assert!(
        !tool_json.contains("arguments") && !tool_json.contains("result_shape"),
        "empty optional fields leaked into the wire form: {tool_json}"
    );
}
