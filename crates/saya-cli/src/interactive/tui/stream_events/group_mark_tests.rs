//! Phase 7 packet 1: a tool line's mark matches its outcome — `✓` for a
//! completion that succeeded, `✗` for one that failed — at both render
//! sites (the live tail in `tool_buffer.rs` and the flushed group detail
//! in `stream_events/group.rs`). Driven through `super::apply_event`, the
//! same entry point the stream uses.

use super::{Transcript, apply_event};
use saya_agent::AgentEvent;

fn read_only_effect() -> Option<saya_agent::ToolEffect> {
    Some(saya_agent::ToolEffect {
        database_data: true,
        external_side_effect: false,
        requires_approval: false,
        local_state: saya_agent::LocalStateEffect::None,
    })
}

fn failed_completion(name: &str) -> Vec<AgentEvent> {
    vec![
        AgentEvent::tool_requested(
            name,
            serde_json::json!({"sql": "DROP TABLE t"}),
            read_only_effect(),
        ),
        AgentEvent::ToolCompleted {
            name: name.into(),
            summary: "read-only database tool failed".into(),
        },
    ]
}

fn succeeded_completion(name: &str) -> Vec<AgentEvent> {
    vec![
        AgentEvent::tool_requested(
            name,
            serde_json::json!({"sql": "SELECT 1"}),
            read_only_effect(),
        ),
        AgentEvent::ToolCompleted {
            name: name.into(),
            summary: "1 row".into(),
        },
    ]
}

fn texts(transcript: &Transcript) -> Vec<String> {
    transcript.blocks().iter().map(|b| b.text.clone()).collect()
}

/// A refused/failed read-only query must not wear the success mark: today
/// both render sites format every completion as `✓ {name}: {summary}`.
#[test]
fn a_failed_tool_line_is_not_marked_with_a_checkmark() {
    let mut transcript = Transcript::new();
    for event in failed_completion("bounded_sql_query") {
        apply_event(&mut transcript, event, false);
    }
    let texts = texts(&transcript);
    assert!(
        texts
            .iter()
            .any(|line| line == "✗ bounded_sql_query: read-only database tool failed"),
        "the failed completion must render with ✗, not ✓: {texts:?}"
    );
    assert!(
        texts
            .iter()
            .all(|line| !line.starts_with("✓ bounded_sql_query")),
        "no line for the failed call may carry a checkmark: {texts:?}"
    );
}

/// The success path is unchanged: a completed call keeps its checkmark.
#[test]
fn a_successful_tool_line_keeps_its_checkmark() {
    let mut transcript = Transcript::new();
    for event in succeeded_completion("bounded_sql_query") {
        apply_event(&mut transcript, event, false);
    }
    let texts = texts(&transcript);
    assert!(
        texts
            .iter()
            .any(|line| line == "✓ bounded_sql_query: 1 row"),
        "the successful completion must keep its ✓: {texts:?}"
    );
}

/// The anti-drift test: the same failing completion must render with the
/// same mark on the live tail (before the boundary) and in the flushed
/// group (after the boundary). A failure group keeps its live lines
/// verbatim, so the two must be identical.
#[test]
fn the_live_tail_and_the_flushed_group_agree_on_the_mark() {
    let mut transcript = Transcript::new();
    for event in failed_completion("bounded_sql_query") {
        apply_event(&mut transcript, event, false);
    }
    let live: Vec<String> = texts(&transcript);
    apply_event(&mut transcript, AgentEvent::assistant_text("done"), false);
    let flushed: Vec<String> = texts(&transcript);
    let live_completion = live
        .iter()
        .find(|line| line.contains("read-only database tool failed"))
        .expect("the live tail shows the completion");
    let flushed_completion = flushed
        .iter()
        .find(|line| line.contains("read-only database tool failed"))
        .expect("the flushed group shows the completion");
    assert_eq!(
        live_completion, flushed_completion,
        "live tail and flushed group must agree on the mark"
    );
    assert!(
        live_completion.starts_with("✗ "),
        "both must use ✗ for a failure: {live_completion:?}"
    );
}

/// `ToolDenied` is a different thing from a failure and already renders
/// differently (`✗ {name} denied: {reason}` on a System block). This
/// packet must not merge or move it.
#[test]
fn a_denied_tool_still_renders_as_denied() {
    let mut transcript = Transcript::new();
    apply_event(
        &mut transcript,
        AgentEvent::ToolDenied {
            name: "run_command".into(),
            reason: "needs approval".into(),
        },
        false,
    );
    let blocks = transcript.blocks();
    assert_eq!(blocks.len(), 1, "one denial block: {blocks:?}");
    assert_eq!(blocks[0].text, "✗ run_command denied: needs approval");
    assert_eq!(
        blocks[0].kind,
        super::BlockKind::System,
        "a denial stays a System block, not a Tool block"
    );
}

/// The collapsed group header already names the failure count honestly
/// (`▸ 2 tool calls · 1 failed (…) — details below`); this packet leaves
/// it exactly as it is.
#[test]
fn the_group_header_still_names_the_failure_count() {
    let events = vec![
        AgentEvent::tool_requested(
            "bounded_sql_query",
            serde_json::json!({"sql": "SELECT 1"}),
            read_only_effect(),
        ),
        AgentEvent::ToolCompleted {
            name: "bounded_sql_query".into(),
            summary: "1 row".into(),
        },
        AgentEvent::tool_requested(
            "bounded_sql_query",
            serde_json::json!({"sql": "DROP TABLE t"}),
            read_only_effect(),
        ),
        AgentEvent::ToolCompleted {
            name: "bounded_sql_query".into(),
            summary: "read-only database tool failed".into(),
        },
    ];
    let groups = crate::render::tool_groups::group_tool_events(&events);
    assert_eq!(groups.len(), 1, "precondition: one run is one group");
    let shaped = crate::render::tool_groups::shape_group(&groups[0]);
    assert!(
        shaped[0].starts_with("▸ 2 tool calls · 1 failed ("),
        "the header must still name the failure count: {shaped:?}"
    );
}

/// The mark keys on the one shared predicate
/// (`render::tool_groups::is_failure_summary`), never a second definition:
/// for a spread of summaries the rendered mark and the predicate agree.
#[test]
fn the_mark_uses_the_shared_failure_predicate() {
    for summary in [
        "read-only database tool failed",
        "local-state write failed",
        "failed pytest",
        "1 row",
        "notes.md written",
        "query exited 1",
    ] {
        let mut transcript = Transcript::new();
        apply_event(
            &mut transcript,
            AgentEvent::tool_requested("probe", serde_json::json!({}), read_only_effect()),
            false,
        );
        apply_event(
            &mut transcript,
            AgentEvent::ToolCompleted {
                name: "probe".into(),
                summary: summary.into(),
            },
            false,
        );
        let expected = if crate::render::tool_groups::is_failure_summary(summary) {
            "✗"
        } else {
            "✓"
        };
        let texts = texts(&transcript);
        assert!(
            texts
                .iter()
                .any(|line| line == &format!("{expected} probe: {summary}")),
            "summary {summary:?} (predicate={expected}) must render with {expected}: {texts:?}"
        );
    }
}
