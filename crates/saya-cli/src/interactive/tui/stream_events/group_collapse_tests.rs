//! Tests for the tool-group collapse/expand properties (C3), driven through `super::apply_event` (moved verbatim
//! from the inline `tests` module in `stream_events/apply.rs`).

use super::{Transcript, apply_event};
use saya_agent::AgentEvent;

fn write_effect() -> Option<saya_agent::ToolEffect> {
    Some(saya_agent::ToolEffect {
        database_data: false,
        external_side_effect: false,
        requires_approval: false,
        local_state: saya_agent::LocalStateEffect::WriteWorkspace,
    })
}

/// Two successful workspace writes: the shared fixture every C3 property
/// builds its group from.
fn two_write_calls() -> Vec<AgentEvent> {
    vec![
        AgentEvent::tool_requested(
            "workspace_write",
            serde_json::json!({"path": "notes.md", "content": "hi"}),
            write_effect(),
        ),
        AgentEvent::ToolCompleted {
            name: "workspace_write".into(),
            summary: "notes.md written".into(),
        },
        AgentEvent::tool_requested(
            "workspace_write",
            serde_json::json!({"path": "other.md", "content": "hi"}),
            write_effect(),
        ),
        AgentEvent::ToolCompleted {
            name: "workspace_write".into(),
            summary: "other.md written".into(),
        },
    ]
}

/// C3 property 1: a collapsed group renders one block whose text equals
/// the shaper's output — the same string the piped surface would emit, so
/// the two surfaces cannot drift.
#[test]
fn collapsed_group_renders_one_block_equal_to_the_shaper_output() {
    let events = two_write_calls();
    let groups = crate::render::tool_groups::group_tool_events(&events);
    assert_eq!(groups.len(), 1, "precondition: one run is one group");
    let shaped = crate::render::tool_groups::shape_group(&groups[0]);
    assert_eq!(shaped.len(), 1, "precondition: the shaper collapses it");

    let mut transcript = Transcript::new();
    for event in events {
        apply_event(&mut transcript, event, false);
    }
    apply_event(&mut transcript, AgentEvent::complete(), false);
    let texts: Vec<&str> = transcript
        .blocks()
        .iter()
        .map(|b| b.text.as_str())
        .collect();
    assert_eq!(
        texts,
        vec![shaped[0].as_str()],
        "a collapsed group must render one block equal to the shaper's output"
    );
}

/// C3 property 2: toggling expands to the per-call `→` / `✓` lines and
/// toggling again collapses back to the one-line summary.
#[test]
fn toggling_expands_to_the_per_call_lines_and_back() {
    let mut transcript = Transcript::new();
    for event in two_write_calls() {
        apply_event(&mut transcript, event, false);
    }
    apply_event(&mut transcript, AgentEvent::complete(), false);
    assert_eq!(transcript.blocks().len(), 1, "collapsed to one block");

    assert!(
        transcript.toggle_latest_group(),
        "a group is there to expand"
    );
    let wrapped = transcript.wrapped(200);
    let shown: Vec<&str> = wrapped
        .iter()
        .filter(|row| !row.is_label)
        .map(|row| row.text.as_str())
        .collect();
    assert_eq!(
        shown,
        vec![
            "▾ 2 tool calls · ok — workspace_write notes.md, other.md",
            "→ workspace_write: notes.md",
            "✓ workspace_write: notes.md written",
            "→ workspace_write: other.md",
            "✓ workspace_write: other.md written",
        ],
        "expanded shows today's per-call lines verbatim under a ▾ header"
    );

    assert!(transcript.toggle_latest_group(), "toggling again collapses");
    let wrapped = transcript.wrapped(200);
    let shown: Vec<&str> = wrapped
        .iter()
        .filter(|row| !row.is_label)
        .map(|row| row.text.as_str())
        .collect();
    assert_eq!(
        shown,
        vec!["▸ 2 tool calls · ok — workspace_write notes.md, other.md"],
        "collapsed again to the one-line summary"
    );
}

/// C3 property 3: expansion survives a re-render and a newly streamed
/// event — it is view state on the block, not a render-time flag.
#[test]
fn expansion_survives_rerender_and_a_newly_streamed_event() {
    let mut transcript = Transcript::new();
    for event in two_write_calls() {
        apply_event(&mut transcript, event, false);
    }
    apply_event(&mut transcript, AgentEvent::complete(), false);
    assert!(transcript.toggle_latest_group(), "expand the group");

    // A re-render: the same wrapped lines, still expanded.
    let first: Vec<String> = transcript
        .wrapped(200)
        .iter()
        .filter(|row| !row.is_label)
        .map(|row| row.text.clone())
        .collect();
    let second: Vec<String> = transcript
        .wrapped(200)
        .iter()
        .filter(|row| !row.is_label)
        .map(|row| row.text.clone())
        .collect();
    assert_eq!(first, second, "re-render must not reset expansion");
    assert!(first[0].starts_with('▾'), "still expanded: {first:?}");

    // A newly streamed event lands after the group without resetting it.
    apply_event(&mut transcript, AgentEvent::assistant_text("done"), false);
    let wrapped = transcript.wrapped(200);
    let shown: Vec<&str> = wrapped
        .iter()
        .filter(|row| !row.is_label)
        .map(|row| row.text.as_str())
        .collect();
    assert!(shown[0].starts_with('▾'), "expansion survives: {shown:?}");
    assert_eq!(
        shown.last(),
        Some(&"done"),
        "the new event lands: {shown:?}"
    );
}

/// C3 property 4: a one-member group renders exactly as today — the same
/// blocks, byte for byte — with no toggle affordance.
#[test]
fn one_member_group_renders_as_today_with_no_toggle() {
    let events = vec![
        AgentEvent::tool_requested(
            "workspace_write",
            serde_json::json!({"path": "notes.md", "content": "hi"}),
            write_effect(),
        ),
        AgentEvent::ToolCompleted {
            name: "workspace_write".into(),
            summary: "notes.md written".into(),
        },
    ];
    let mut grouped = Transcript::new();
    for event in events {
        apply_event(&mut grouped, event, false);
    }
    apply_event(&mut grouped, AgentEvent::complete(), false);

    assert_eq!(grouped.blocks().len(), 2, "two per-call blocks, not one");
    assert_eq!(grouped.blocks()[0].text, "→ workspace_write: notes.md");
    assert_eq!(
        grouped.blocks()[1].text,
        "✓ workspace_write: notes.md written"
    );
    assert!(
        grouped.blocks().iter().all(|b| !b.is_collapsible()),
        "no block may offer a toggle"
    );
    assert!(
        !grouped.toggle_latest_group(),
        "toggling with no group is a no-op"
    );
    assert_eq!(grouped.blocks().len(), 2, "the no-op toggled nothing");
}
