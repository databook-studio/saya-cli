//! C1 properties: the shared grouper + shaper over `AgentEvent` streams.
//!
//! Each named test pins one slice property; the trailing extras cover what
//! the list misses and say why.

use super::{group_tool_events, is_failure_summary, shape_group};
use saya_agent::{AgentEvent, LocalStateEffect, ToolEffect};

fn write_effect() -> Option<ToolEffect> {
    Some(ToolEffect {
        database_data: false,
        external_side_effect: false,
        requires_approval: false,
        local_state: LocalStateEffect::WriteWorkspace,
    })
}

fn read_effect() -> Option<ToolEffect> {
    Some(ToolEffect {
        database_data: true,
        external_side_effect: false,
        requires_approval: false,
        local_state: LocalStateEffect::Read,
    })
}

fn run_effect() -> Option<ToolEffect> {
    Some(ToolEffect {
        database_data: false,
        external_side_effect: true,
        requires_approval: false,
        local_state: LocalStateEffect::WriteWorkspace,
    })
}

fn requested(name: &str, arguments: serde_json::Value, effect: Option<ToolEffect>) -> AgentEvent {
    AgentEvent::tool_requested(name, arguments, effect)
}

fn completed(name: &str, summary: &str) -> AgentEvent {
    AgentEvent::ToolCompleted {
        name: name.into(),
        summary: summary.into(),
    }
}

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

/// Property 6: failure detection agrees with `tool_metadata.status` — both
/// key on the summary containing "failed".
#[test]
fn failure_detection_agrees_with_tool_metadata_status() {
    for summary in ["failed pytest", "failed to complete: host command ran"] {
        assert!(
            is_failure_summary(summary),
            "the grouper must classify {summary:?} as failed"
        );
        let status = if summary.contains("failed") {
            "failed"
        } else {
            "completed"
        };
        assert_eq!(
            status, "failed",
            "tool_metadata.status derives from the same predicate"
        );
    }
    for summary in ["pytest exited 1", "notes.md written"] {
        assert!(
            !is_failure_summary(summary),
            "the grouper must not classify {summary:?} as failed"
        );
    }
}

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
