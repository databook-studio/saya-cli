/// A turn that carried chain-of-thought must leave none of it in a
/// persisted session. The reasoning lives on `ChatResponse.reasoning`
/// (transport for a single call) and on `AgentEvent::ReasoningText` (an
/// in-memory event); neither has a field on the persisted types, and
/// `record_turn` — the only write path into a session — takes `user`,
/// `assistant`, `database_derived` and `tools`, so there is no argument a
/// caller could pass the reasoning through.
///
/// What this pins is the shape of the persisted form: no `reasoning` key
/// appears on a serialized session. The companion test beside `apply_event`
/// covers the stronger property — reasoning genuinely on screen and still
/// absent from disk — which needs the transcript, unreachable from here.
#[test]
fn a_session_persisted_after_a_reasoning_turn_contains_none_of_it() {
    let reasoning = "the secret chain-of-thought about row values 9f3a";
    let mut session = SessionState::new("s1", Some(String::from("analytics")), String::from("m"));
    session.record_turn("what is the answer", "the answer is 42", false, Vec::new());

    let json = serde_json::to_string(&session).expect("serializes");
    assert!(
        !json.contains(reasoning),
        "reasoning leaked into the persisted session: {json}"
    );
    assert!(
        !json.contains("reasoning"),
        "a `reasoning` key appeared on a persisted session: {json}"
    );

    let replayed = session.provider_history();
    assert!(
        replayed.iter().all(|m| !m.content.contains(reasoning)),
        "reasoning leaked into replayed history: {replayed:?}"
    );
}

/// A turn may carry arguments and a result shape while the session is
/// live, but neither is persisted. Session files retain only the minimal
/// tool name/status audit metadata.
#[test]
fn a_session_that_ran_sql_does_not_persist_arguments_or_shape() {
    let mut session = SessionState::new("s1", None, "m");
    session.record_turn(
        "count customers",
        "42 customers",
        true,
        vec![ToolMetadata {
            name: "bounded_sql_query".into(),
            status: "completed".into(),
            arguments: r#"{"sql":"SELECT customer, age FROM users"}"#.into(),
            result_shape: Some(ToolResultShape {
                row_count: 1,
                columns: vec!["customer".into(), "age".into()],
            }),
        }],
    );

    let saved = session.redacted();
    let tool = &saved.turns[0].tools[0];
    assert_eq!(tool.name, "bounded_sql_query");
    assert_eq!(tool.status, "completed");
    let json = serde_json::to_string(&saved).expect("serializes");
    assert!(!json.contains("SELECT customer, age FROM users"));
    assert!(!json.contains("result_shape"));
}

/// A cell value present in a live query result never reaches the session
/// file. Tool arguments and result shapes are live-only, so neither the
/// cell nor its surrounding query metadata is serialized.
#[test]
fn a_result_cell_value_never_reaches_the_session_file() {
    use saya_store::{FsSessionStore, SessionStore};
    // A live query result carrying a cell value. The agent reads only its
    // row_count and columns (pinned in saya-agent's `result_shape_of`
    // test); the record below mirrors that shape, with no rows.
    let live_result = serde_json::json!({
        "columns": ["customer"],
        "rows": [["CELL_SENTINEL_9f3a"]],
        "row_count": 1,
        "truncated": false
    });
    let shape = ToolResultShape {
        row_count: live_result["row_count"].as_u64().unwrap(),
        columns: live_result["columns"]
            .as_array()
            .unwrap()
            .iter()
            .map(|name| name.as_str().unwrap().to_owned())
            .collect(),
    };
    let mut session = SessionState::new("cell-test", None, "m");
    session.record_turn(
        "show customers",
        "here they are",
        true,
        vec![ToolMetadata {
            name: "bounded_sql_query".into(),
            status: "completed".into(),
            arguments: r#"{"sql":"SELECT customer FROM users"}"#.into(),
            result_shape: Some(shape),
        }],
    );
    let root = std::env::temp_dir().join(format!("saya-cell-{}", std::process::id()));
    let store = FsSessionStore::new(&root);
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(store.save(session.redacted())).unwrap();
    let file = std::fs::read_to_string(root.join("cell-test.json")).unwrap();
    assert!(
        !file.contains("CELL_SENTINEL_9f3a"),
        "a cell value reached the session file: {file}"
    );
    assert!(
        !file.contains("\"rows\""),
        "a `rows` key appeared in a tool record: {file}"
    );
    // Round-trip through the store and confirm the persisted record keeps
    // only the minimal tool audit fields.
    let loaded = runtime
        .block_on(store.load("cell-test"))
        .unwrap()
        .expect("the session file loads");
    let tool = &loaded.turns[0].tools[0];
    assert!(tool.arguments.is_empty());
    assert!(tool.result_shape.is_none());
    let _ = std::fs::remove_dir_all(root);
}
