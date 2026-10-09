use super::*;

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
