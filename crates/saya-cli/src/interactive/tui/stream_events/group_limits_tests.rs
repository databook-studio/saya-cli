//! Tests for the tool-group collapse limits: failure and persistence (C3), driven through `super::apply_event` (moved verbatim
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

/// C3 property 5: a group with a failure is not collapsible into a count —
/// the failure's full pair renders with the shared mark (`✗` for the
/// failed call, `✓` for the success), never the piped text's `Using tool:`
/// rendering.
#[test]
fn group_with_a_failure_renders_the_full_pair() {
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
        AgentEvent::tool_requested(
            "run_command",
            serde_json::json!({"program": "pytest", "args": ["-q"]}),
            Some(saya_agent::ToolEffect {
                database_data: false,
                external_side_effect: true,
                requires_approval: false,
                local_state: saya_agent::LocalStateEffect::WriteWorkspace,
            }),
        ),
        AgentEvent::ToolCompleted {
            name: "run_command".into(),
            summary: "failed pytest".into(),
        },
    ];
    let groups = crate::render::tool_groups::group_tool_events(&events);
    assert_eq!(groups.len(), 1, "precondition: one run is one group");
    assert!(
        groups[0].calls.iter().any(|call| call.failed),
        "precondition: the group carries a failure"
    );

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
        vec![
            "→ workspace_write: notes.md",
            "✓ workspace_write: notes.md written",
            "→ run_command: pytest",
            "✗ run_command: failed pytest",
        ],
        "the failure's full pair renders with ✗ on the failed call, successes uncollapsed"
    );
    assert!(
        transcript.blocks().iter().all(|b| !b.is_collapsible()),
        "a group with a failure offers no toggle"
    );
    assert!(
        texts.iter().any(|line| line.contains("failed pytest")),
        "the failure text is visible: {texts:?}"
    );
}

/// C3 property 6: expansion state is never written to the session file,
/// and a resumed session does not carry it — replay renders through
/// `replay.rs`, unchanged, with no group state. (Why: the transcript is
/// never serialized — `SessionState` carries role + content only — so the
/// strongest check available is the serialized session plus the replay
/// path both showing no group text.)
#[test]
fn expansion_state_is_not_persisted_and_resume_carries_none_of_it() {
    use crate::interactive::session_state::SessionState;

    let mut transcript = Transcript::new();
    for event in two_write_calls() {
        apply_event(&mut transcript, event, false);
    }
    apply_event(&mut transcript, AgentEvent::complete(), false);
    assert!(transcript.toggle_latest_group(), "expand the group");

    let mut session = SessionState::new("s1", Some(String::from("analytics")), String::from("m"));
    session.record_turn("do the writes", "wrote both files", false, Vec::new());
    let json = serde_json::to_string(&session).expect("serializes");
    assert!(
        !json.contains("expanded"),
        "expansion state leaked into the session file: {json}"
    );

    let replayed = crate::interactive::tui::replay::history_blocks(&session);
    assert!(
        replayed.iter().all(|(_, text)| !text.starts_with('▸')),
        "replay renders through replay.rs, not the grouper: {replayed:?}"
    );
}
