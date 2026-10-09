use super::*;

/// Extra: no time window and no same-tool requirement — interleaved tools in
/// one run still form one group (why: ordering defines the run).
#[test]
fn interleaved_tools_stay_in_one_group() {
    let events = vec![
        requested(
            "workspace_read",
            serde_json::json!({"path": "a.md"}),
            read_effect(),
        ),
        requested(
            "run_command",
            serde_json::json!({"program": "pytest"}),
            run_effect(),
        ),
        completed("workspace_read", "a.md"),
        completed("run_command", "pytest exited 0"),
    ];
    let groups = group_tool_events(&events);
    assert_eq!(
        groups.len(),
        1,
        "ordering defines the run, not tool identity"
    );
    assert_eq!(groups[0].calls.len(), 2);
}

/// Extra: a reasoning-text boundary closes a group even though the piped
/// adapter never renders it (why: reasoning is content, and content
/// boundaries are where groups end).
#[test]
fn reasoning_text_closes_the_group() {
    let events = vec![
        requested(
            "workspace_write",
            serde_json::json!({"path": "a.md"}),
            write_effect(),
        ),
        completed("workspace_write", "a.md written"),
        AgentEvent::reasoning_text("thinking"),
        requested(
            "workspace_write",
            serde_json::json!({"path": "b.md"}),
            write_effect(),
        ),
        completed("workspace_write", "b.md written"),
    ];
    let groups = group_tool_events(&events);
    assert_eq!(groups.len(), 2, "reasoning text is a content boundary");
}
