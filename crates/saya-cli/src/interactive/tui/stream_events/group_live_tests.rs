//! Tests for the live tail and approval-denial group boundaries, driven through `super::apply_event` (moved verbatim
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

/// Extra: while the group is still streaming, the tail shows the per-call
/// lines live — collapsing mid-stream would rewrite history the user just
/// watched. (Why: the boundary rule says a group closes at the next
/// content event; until then the calls are still arriving.)
#[test]
fn open_group_shows_per_call_lines_live_until_the_boundary() {
    let mut transcript = Transcript::new();
    for event in two_write_calls() {
        apply_event(&mut transcript, event, false);
    }
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
            "→ workspace_write: other.md",
            "✓ workspace_write: other.md written",
        ],
        "before the boundary the stream shows today's lines live"
    );
    apply_event(&mut transcript, AgentEvent::assistant_text("done"), false);
    let texts: Vec<&str> = transcript
        .blocks()
        .iter()
        .map(|b| b.text.as_str())
        .collect();
    assert_eq!(
        texts,
        vec![
            "▸ 2 tool calls · ok — workspace_write notes.md, other.md",
            "",
            "done",
        ],
        "the boundary collapses the run and the text follows"
    );
}

/// Property 2 (TUI half): `approvals_are_untouched` — the denial is a
/// boundary that splits the surrounding calls into one-member groups
/// (today's `→` / `✓` lines, no toggle), and the denial itself renders
/// as today's `✗` line. The key half of the property lives beside the
/// keys (`keys::approval_modal_tests::grouping_leaves_approval_keys_untouched`):
/// Enter still toggles a group and never approves. Prompt bytes and the
/// bypass line live beside their seams (see the piped half for the
/// pointers); the transcript path never renders either string.
#[test]
fn approvals_are_untouched() {
    let mut transcript = Transcript::new();
    apply_event(
        &mut transcript,
        AgentEvent::tool_requested(
            "workspace_write",
            serde_json::json!({"path": "a.md", "content": "hi"}),
            write_effect(),
        ),
        false,
    );
    apply_event(
        &mut transcript,
        AgentEvent::ToolCompleted {
            name: "workspace_write".into(),
            summary: "a.md written".into(),
        },
        false,
    );
    apply_event(
        &mut transcript,
        AgentEvent::ToolDenied {
            name: "run_command".into(),
            reason: "denied".into(),
        },
        false,
    );
    let texts: Vec<&str> = transcript
        .blocks()
        .iter()
        .map(|b| b.text.as_str())
        .collect();
    assert_eq!(
        texts,
        vec![
            "→ workspace_write: a.md",
            "✓ workspace_write: a.md written",
            "✗ run_command denied: denied",
        ],
        "one-member groups stay verbatim and the denial is its own line"
    );
    assert!(
        transcript.blocks().iter().all(|b| !b.is_collapsible()),
        "the denial splits the run: no collapsible group may span it"
    );
}

fn last_block_text(transcript: &Transcript) -> Option<&str> {
    transcript.blocks().last().map(|b| b.text.as_str())
}

/// C0 property 4 (TUI half): the `→` request block names the same file
/// the piped half pins beside the shared seam — both adapters render
/// from `tool_call_detail`, so they cannot drift.
#[test]
fn tui_request_block_names_the_same_file_as_the_piped_line() {
    use saya_agent::{LocalStateEffect, ToolEffect};

    let mut transcript = Transcript::new();
    apply_event(
        &mut transcript,
        AgentEvent::tool_requested(
            "workspace_write",
            serde_json::json!({"path": "notes.md", "content": "hi"}),
            Some(ToolEffect {
                database_data: false,
                external_side_effect: false,
                requires_approval: false,
                local_state: LocalStateEffect::WriteWorkspace,
            }),
        ),
        false,
    );
    let block = last_block_text(&transcript).expect("a block was pushed");
    assert!(
        block.contains("notes.md"),
        "the TUI request block must name the file: {block:?}"
    );
}
