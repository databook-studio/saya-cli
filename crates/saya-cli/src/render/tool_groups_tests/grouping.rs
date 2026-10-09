use super::*;

/// Property 1: a one-member group shapes to today's lines, byte for byte.
#[test]
fn single_call_shapes_to_todays_lines_byte_for_byte() {
    let arguments = serde_json::json!({"path": "notes.md", "content": "hi"});
    let events = vec![
        requested("workspace_write", arguments, write_effect()),
        completed("workspace_write", "notes.md written"),
    ];
    let groups = group_tool_events(&events);
    assert_eq!(groups.len(), 1, "one run of tool events is one group");
    let shaped = shape_group(&groups[0]);
    assert_eq!(
        shaped,
        vec![
            "Using tool: workspace_write\n  notes.md\n".to_owned(),
            "workspace_write: notes.md written\n".to_owned(),
        ],
        "a one-member group must shape to today's lines verbatim"
    );
}

/// Property 2: a run of successful calls shapes to one header naming the
/// tools and their key arguments, not a bare count.
#[test]
fn successful_run_names_tools_and_key_arguments() {
    let events = vec![
        requested(
            "workspace_read",
            serde_json::json!({"path": "a.md"}),
            read_effect(),
        ),
        completed("workspace_read", "a.md"),
        requested(
            "workspace_read",
            serde_json::json!({"path": "b.md"}),
            read_effect(),
        ),
        completed("workspace_read", "b.md"),
        requested(
            "workspace_write",
            serde_json::json!({"path": "notes.md"}),
            write_effect(),
        ),
        completed("workspace_write", "notes.md written"),
        requested(
            "run_command",
            serde_json::json!({"program": "pytest", "args": ["-q"]}),
            run_effect(),
        ),
        completed("run_command", "pytest exited 0"),
        requested("schema_discovery", serde_json::json!({}), read_effect()),
        completed("schema_discovery", "discovered"),
    ];
    let groups = group_tool_events(&events);
    assert_eq!(groups.len(), 1, "one uninterrupted run is one group");
    let shaped = shape_group(&groups[0]);
    assert_eq!(
        shaped,
        vec!["▸ 5 tool calls · ok — workspace_read ×2, workspace_write notes.md, run_command [pytest -q], schema_discovery".to_owned()],
        "the header must name what ran, not just how many"
    );
}

/// Property 3: a group containing a failure prints that failure's full pair,
/// uncollapsed — header first, then the verbatim request/completion lines,
/// with successes staying collapsed.
#[test]
fn failure_prints_its_full_pair_uncollapsed() {
    let events = vec![
        requested(
            "workspace_write",
            serde_json::json!({"path": "notes.md"}),
            write_effect(),
        ),
        completed("workspace_write", "notes.md written"),
        requested(
            "run_command",
            serde_json::json!({"program": "pytest", "args": ["-q"]}),
            run_effect(),
        ),
        completed("run_command", "failed pytest"),
    ];
    let groups = group_tool_events(&events);
    assert_eq!(groups.len(), 1);
    let shaped = shape_group(&groups[0]);
    assert_eq!(
        shaped,
        vec![
            "▸ 2 tool calls · 1 failed (run_command [pytest -q]) — details below".to_owned(),
            "Using tool: run_command\n  [pytest -q]\n".to_owned(),
            "run_command: failed pytest\n".to_owned(),
        ],
        "failures are never collapsed: header plus the full verbatim pair"
    );
}

/// Property 4: a boundary event closes a group — calls on either side are
/// never merged.
#[test]
fn boundary_event_closes_the_group() {
    let events = vec![
        requested(
            "workspace_write",
            serde_json::json!({"path": "a.md"}),
            write_effect(),
        ),
        completed("workspace_write", "a.md written"),
        AgentEvent::assistant_text("between"),
        requested(
            "workspace_write",
            serde_json::json!({"path": "b.md"}),
            write_effect(),
        ),
        completed("workspace_write", "b.md written"),
    ];
    let groups = group_tool_events(&events);
    assert_eq!(groups.len(), 2, "assistant text closes the group");
    assert_eq!(
        shape_group(&groups[0]),
        vec![
            "Using tool: workspace_write\n  a.md\n".to_owned(),
            "workspace_write: a.md written\n".to_owned(),
        ]
    );
    assert_eq!(
        shape_group(&groups[1]),
        vec![
            "Using tool: workspace_write\n  b.md\n".to_owned(),
            "workspace_write: b.md written\n".to_owned(),
        ]
    );
}

/// Property 5: approval request and resolution are boundaries, so consent
/// can never sit inside a collapsed group. The TUI carries approvals outside
/// `AgentEvent`; the grouper treats every non-member event — including the
/// denial that records the resolution — as a boundary.
#[test]
fn approval_request_and_resolution_are_boundaries() {
    let events = vec![
        requested(
            "workspace_write",
            serde_json::json!({"path": "a.md"}),
            write_effect(),
        ),
        completed("workspace_write", "a.md written"),
        AgentEvent::ToolDenied {
            name: "run_command".into(),
            reason: "denied".into(),
        },
        requested(
            "workspace_write",
            serde_json::json!({"path": "b.md"}),
            write_effect(),
        ),
        completed("workspace_write", "b.md written"),
    ];
    let groups = group_tool_events(&events);
    assert_eq!(
        groups.len(),
        2,
        "a denial between two calls splits them into two groups"
    );
    assert_eq!(groups[0].calls.len(), 1);
    assert_eq!(groups[1].calls.len(), 1);
}
