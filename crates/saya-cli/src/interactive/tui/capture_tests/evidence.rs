use super::*;

#[test]
fn evidence_line_follows_the_scope_line() {
    let (transcript, _) = complete_sql(result(3, false));
    let block = last_block(&transcript, BlockKind::Table).expect("the result renders as a table");
    let last = block.lines().last().expect("the block has lines");
    assert!(
        last.starts_with("direct sql: analytics · 3 rows"),
        "the evidence line is the block's final line: {last:?}"
    );
    assert!(
        block.contains("from analytics · SELECT id FROM t"),
        "the scope line is still present above it:\n{block}"
    );
    assert!(
        !last.contains("SELECT"),
        "the evidence line carries no SQL text: {last:?}"
    );
}

#[test]
fn truncated_result_evidence_says_truncated() {
    // The test runtime caps at 100 rows (unused_runtime's max_rows).
    let (transcript, captured) = complete_sql(result(100, true));
    let cap = captured.expect("a truncated success is still captured");
    assert!(cap.evidence.truncated);
    let last = last_block(&transcript, BlockKind::Table)
        .expect("table")
        .lines()
        .last()
        .expect("lines")
        .to_string();
    assert!(last.contains("100 rows (truncated at 100)"), "{last}");
}

#[test]
fn connection_label_falls_back_to_the_task_profile() {
    // No dispatch-time connection: the label falls back to the profile the
    // query ran on.
    let mut transcript = Transcript::new();
    let mut captured = None;
    let task = SqlTask::new(
        Some("analytics".to_string()),
        "SELECT id FROM t".to_string(),
        Followup::Sql { connection: None },
    );
    let r = result(2, false);
    plant_envelope(&task, &r, runtime().resolved.max_rows);
    complete(
        &task,
        TerminalEvent::QueryResult { result: r },
        &mut transcript,
        &mut None,
        &mut captured,
    );
    let cap = captured.expect("a successful /sql without a captured connection is still captured");
    assert_eq!(cap.evidence.connection_label, "analytics");
}
