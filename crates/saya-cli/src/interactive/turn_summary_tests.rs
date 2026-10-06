use super::*;
use crate::render::TerminalEvent;

fn requested(name: &str, arguments: serde_json::Value) -> AgentEvent {
    AgentEvent::ToolRequested {
        name: name.to_owned(),
        arguments,
        effect: None,
    }
}

fn completed(name: &str) -> AgentEvent {
    AgentEvent::ToolCompleted {
        name: name.to_owned(),
        summary: "did it".to_owned(),
    }
}

fn denied(name: &str) -> AgentEvent {
    AgentEvent::ToolDenied {
        name: name.to_owned(),
        reason: "refused".to_owned(),
    }
}

fn tally(events: &[AgentEvent]) -> ToolTally {
    let mut tally = ToolTally::default();
    for event in events {
        tally.record(event);
    }
    tally
}

/// The `turn_result` wire shape is pinned per status: the serde tag, the
/// snake_case status, and the field order a script reads.
#[test]
fn the_turn_result_json_shape_is_pinned_for_each_status() {
    let shape = |status| {
        serde_json::to_string(&TerminalEvent::TurnResult {
            status,
            exit_code: 0,
            tools_completed: 2,
            tools_unfinished: Vec::new(),
        })
        .unwrap()
    };
    assert_eq!(
        shape(TurnStatus::Completed),
        "{\"event\":\"turn_result\",\"status\":\"completed\",\"exit_code\":0,\"tools_completed\":2,\"tools_unfinished\":[]}"
    );
    assert_eq!(
        shape(TurnStatus::Failed),
        "{\"event\":\"turn_result\",\"status\":\"failed\",\"exit_code\":0,\"tools_completed\":2,\"tools_unfinished\":[]}"
    );
    assert_eq!(
        shape(TurnStatus::Cancelled),
        "{\"event\":\"turn_result\",\"status\":\"cancelled\",\"exit_code\":0,\"tools_completed\":2,\"tools_unfinished\":[]}"
    );
    assert_eq!(
        shape(TurnStatus::BudgetStopped),
        "{\"event\":\"turn_result\",\"status\":\"budget_stopped\",\"exit_code\":0,\"tools_completed\":2,\"tools_unfinished\":[]}"
    );
}

/// Twelve requested-never-completed calls: seven real entries, then the one
/// overflow line `… and 5 more` — at most eight entries, each within the
/// character bound, and a hostile long path truncated on a char boundary
/// (the entry is a whole-char prefix of the original).
#[test]
fn unfinished_entries_are_bounded_and_truncated_on_char_boundaries() {
    let mut counted = ToolTally::default();
    for index in 0..12 {
        counted.record(&requested(
            "workspace_write",
            serde_json::json!({"path": format!("dir/file-{index}.txt"), "content": "x"}),
        ));
    }
    let entries = counted.unfinished_entries();
    assert_eq!(
        entries.len(),
        8,
        "seven real entries plus the overflow line"
    );
    assert_eq!(entries[7], "… and 5 more");
    for entry in &entries[..7] {
        assert!(entry.chars().count() <= 120, "{entry}");
        assert!(entry.contains("workspace_write"), "{entry}");
    }
    let mut hostile = ToolTally::default();
    let path: String = "é".repeat(200);
    hostile.record(&requested(
        "workspace_edit",
        serde_json::json!({"path": path}),
    ));
    let entries = hostile.unfinished_entries();
    assert_eq!(entries.len(), 1);
    assert!(
        entries[0].chars().count() <= 120,
        "{}",
        entries[0].chars().count()
    );
    assert!(
        "é".repeat(200)
            .starts_with(entries[0].split_once(' ').unwrap().1),
        "the truncated path is a whole-char prefix"
    );
}

/// Requests and completions pair by order and name: a completed event with
/// no pending request is ignored entirely; a requested call with no
/// completion is listed; a file-writing call's entry carries name + path and
/// NEVER the call's content; a non-file tool is named alone; a denied call
/// settled (its effect is known: none); same-name nesting pairs oldest
/// first.
#[test]
fn the_tally_pairs_requests_and_completions_and_lists_unfinished_calls() {
    let counted = tally(&[
        requested("workspace_read", serde_json::json!({"path": "a.txt"})),
        completed("workspace_read"),
        requested("workspace_edit", serde_json::json!({"path": "b.txt"})),
    ]);
    assert_eq!(
        counted.completed(),
        1,
        "the read completed; the edit did not"
    );
    assert_eq!(
        counted.unfinished_entries(),
        vec!["workspace_edit b.txt".to_owned()]
    );
    let orphan = tally(&[completed("ghost")]);
    assert_eq!(orphan.completed(), 0, "an unmatched completion is ignored");
    assert!(orphan.unfinished_entries().is_empty());
    let content = tally(&[requested(
        "workspace_write",
        serde_json::json!({"path": "c.txt", "content": "SECRET-FILE-CONTENT"}),
    )]);
    let entries = content.unfinished_entries();
    assert_eq!(entries, vec!["workspace_write c.txt".to_owned()]);
    assert!(!entries[0].contains("SECRET-FILE-CONTENT"));
    let sql = tally(&[requested(
        "bounded_sql_query",
        serde_json::json!({"sql": "SELECT token FROM secrets"}),
    )]);
    assert_eq!(
        sql.unfinished_entries(),
        vec!["bounded_sql_query".to_owned()],
        "a non-file tool is named alone"
    );
    let refused = tally(&[
        requested("workspace_write", serde_json::json!({"path": "d.txt"})),
        denied("workspace_write"),
    ]);
    assert_eq!(refused.completed(), 0);
    assert!(
        refused.unfinished_entries().is_empty(),
        "a denied call's effect is known: none"
    );
    let nested = tally(&[
        requested("workspace_read", serde_json::json!({})),
        requested("workspace_read", serde_json::json!({})),
        completed("workspace_read"),
    ]);
    assert_eq!(nested.completed(), 1);
    assert_eq!(
        nested.unfinished_entries(),
        vec!["workspace_read".to_owned()]
    );
}

/// The prose settlement maps to the terminal status, a latched budget stop
/// reclassifying Failed/Cancelled.
#[test]
fn prose_status_maps_the_settlement_and_the_latched_budget() {
    assert_eq!(
        prose_status(ProseOutcome::Completed, false),
        TurnStatus::Completed
    );
    assert_eq!(
        prose_status(ProseOutcome::Completed, true),
        TurnStatus::Completed
    );
    assert_eq!(
        prose_status(ProseOutcome::Failed, false),
        TurnStatus::Failed
    );
    assert_eq!(
        prose_status(ProseOutcome::Failed, true),
        TurnStatus::BudgetStopped
    );
    assert_eq!(
        prose_status(ProseOutcome::Cancelled, false),
        TurnStatus::Cancelled
    );
    assert_eq!(
        prose_status(ProseOutcome::Cancelled, true),
        TurnStatus::BudgetStopped
    );
}

/// Exit code 0 belongs to a completed turn only.
#[test]
fn exit_code_is_zero_only_for_a_completed_turn() {
    assert_eq!(exit_code(TurnStatus::Completed), 0);
    assert_eq!(exit_code(TurnStatus::Failed), 5);
    assert_eq!(exit_code(TurnStatus::Cancelled), 5);
    assert_eq!(exit_code(TurnStatus::BudgetStopped), 5);
}
