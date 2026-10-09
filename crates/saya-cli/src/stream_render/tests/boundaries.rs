/// C2 property 5: text ordering is preserved — the summary appears where
/// the group's first line would have, relative to surrounding text.
#[test]
fn piped_text_summary_keeps_position_relative_to_surrounding_text() {
    let events = vec![
        AgentEvent::assistant_text("before"),
        AgentEvent::tool_requested(
            "workspace_write",
            serde_json::json!({"path": "notes.md", "content": "hi"}),
            Some(write_effect()),
        ),
        AgentEvent::ToolCompleted {
            name: "workspace_write".into(),
            summary: "notes.md written".into(),
        },
        AgentEvent::tool_requested(
            "workspace_write",
            serde_json::json!({"path": "other.md", "content": "hi"}),
            Some(write_effect()),
        ),
        AgentEvent::ToolCompleted {
            name: "workspace_write".into(),
            summary: "other.md written".into(),
        },
        AgentEvent::assistant_text("after"),
        AgentEvent::complete(),
    ];
    let stdout = drain_text_stream(events);
    let before = stdout.find("before").expect("leading text renders");
    let summary = stdout.find("2 tool calls · ok").expect("summary renders");
    let after = stdout.find("after").expect("trailing text renders");
    assert!(
        before < summary && summary < after,
        "the flush happens where the first line would have printed: {stdout:?}"
    );
}

/// C2 property 6: a group exceeding the bound flushes rather than
/// growing — 40 completed calls degrade into chunk summaries.
#[test]
fn piped_text_group_exceeding_the_bound_flushes_in_chunks() {
    let mut events = Vec::new();
    for index in 0..40 {
        events.push(AgentEvent::tool_requested(
            "schema_discovery",
            serde_json::json!({"n": index}),
            None,
        ));
        events.push(AgentEvent::ToolCompleted {
            name: "schema_discovery".into(),
            summary: "discovered".into(),
        });
    }
    events.push(AgentEvent::complete());
    let stdout = drain_text_stream(events);
    assert!(
        stdout.contains("32 tool calls · ok"),
        "the bound force-flushes a full chunk: {stdout:?}"
    );
    assert!(
        stdout.contains("8 tool calls · ok"),
        "the remainder flushes as its own chunk: {stdout:?}"
    );
    assert_eq!(
        stdout.lines().count(),
        2,
        "a runaway group degrades into chunk summaries: {stdout:?}"
    );
}
