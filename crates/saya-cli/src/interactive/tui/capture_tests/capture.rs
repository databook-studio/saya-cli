use super::*;

#[test]
fn sql_success_captures_typed_result_and_evidence() {
    let expected = result(3, false);
    let (_, captured) = complete_sql(expected.clone());
    let capture = captured.expect("a successful /sql result is captured");
    assert_eq!(
        capture.result, expected,
        "the capture holds the typed result, rows included"
    );
    let e = &capture.evidence;
    assert_eq!(e.returned_rows, expected.row_count, "evidence counts match");
    assert_eq!(e.max_rows, runtime().resolved.max_rows);
    assert!(!e.truncated);
    assert_eq!(e.connection_label, "analytics");
    assert_eq!(
        e.connection_identity,
        Some("test-identity".to_string()),
        "the identity is the executed binding's"
    );
    assert_eq!(e.dialect, SqlDialect::Sqlite);
    assert!(
        e.started_unix_ms <= e.finished_unix_ms,
        "the planted envelope timestamps never run backwards"
    );
    assert_eq!(e.source, EvidenceSource::DirectSql);
    assert_eq!(e.scope, ResultScope::Full);
    assert!(!e.execution_id.is_empty());
}

#[test]
fn two_tasks_keep_their_own_envelopes() {
    // Two tasks with different bindings, completed in the opposite order:
    // each completion reads its own slot, never the other's.
    let first = SqlTask::new(
        Some("analytics".to_string()),
        "SELECT 1".to_string(),
        Followup::Sql {
            connection: Some("analytics".to_string()),
        },
    );
    let second = SqlTask::new(
        Some("analytics".to_string()),
        "SELECT 2".to_string(),
        Followup::Sql {
            connection: Some("analytics".to_string()),
        },
    );
    let one = result(1, false);
    let two = result(2, false);
    let binding_one = SqlExecution::binding_for("analytics", "identity-one", SqlDialect::Sqlite, 7);
    let binding_two = SqlExecution::binding_for("analytics", "identity-two", SqlDialect::Sqlite, 9);
    first.plant_execution_for_test(&one, binding_one);
    second.plant_execution_for_test(&two, binding_two);

    // Opposite order: second completes first.
    let mut transcript = Transcript::new();
    let mut captured = None;
    complete(
        &second,
        TerminalEvent::QueryResult { result: two },
        &mut transcript,
        &mut None,
        &mut captured,
    );
    let cap = captured.expect("the second task captures");
    assert_eq!(
        cap.evidence.connection_identity,
        Some("identity-two".to_string())
    );
    assert_eq!(cap.evidence.max_rows, 9);

    let mut transcript = Transcript::new();
    let mut captured = None;
    complete(
        &first,
        TerminalEvent::QueryResult { result: one },
        &mut transcript,
        &mut None,
        &mut captured,
    );
    let cap = captured.expect("the first task captures");
    assert_eq!(
        cap.evidence.connection_identity,
        Some("identity-one".to_string())
    );
    assert_eq!(cap.evidence.max_rows, 7);
}
