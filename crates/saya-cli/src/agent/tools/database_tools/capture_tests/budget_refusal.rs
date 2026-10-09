use super::*;

/// Over the accounted budget the hook sends one whole refusal — sql and
/// connection only — and the model still receives its full result; the seam
/// (`capture_event` at a test-sized budget) shows the same decision whole.
#[tokio::test]
async fn over_budget_sends_refusal_not_partial() {
    let (hook, observed) = observing_hook();
    let tools = DatabaseTools::new(Some(Box::new(BigCellConnector)), 100, true)
        .with_capture(Some(hook), loop_budget());
    let value = tools
        .execute(
            "bounded_sql_query",
            serde_json::json!({"sql": "SELECT big", "connection": "primary"}),
        )
        .await
        .expect("the model still receives its result");
    assert_eq!(value["columns"][0], "big", "the full value is unchanged");

    // The seam: the same decision at a test-sized budget — 5 bytes for the
    // cell plus 1 for the column name, under the loop's budget so the
    // model-view gate passes.
    let entry = ConnectionEntry {
        connector: Box::new(FailingConnector),
        dialect: SqlDialect::Sqlite,
        profile_id: None,
    };
    let executed = ExecutedQuery {
        value: serde_json::json!({}),
        result: QueryResult {
            columns: vec!["c".into()],
            rows: vec![serde_json::json!(["hello"])],
            row_count: 1,
            truncated: false,
            executed_sql: "SELECT 'hello'".into(),
        },
        row_cap: 50,
        started_unix_ms: 1,
        finished_unix_ms: 2,
    };
    assert!(matches!(
        capture_event(
            "SELECT 'hello'",
            "primary",
            &entry,
            &executed,
            6,
            usize::MAX
        ),
        CaptureEvent::Captured(_)
    ));
    assert!(matches!(
        capture_event("SELECT 'hello'", "primary", &entry, &executed, 5, usize::MAX),
        CaptureEvent::Refused { sql, connection, reason: CaptureRefusalReason::OverBudget }
            if sql == "SELECT 'hello'" && connection == "primary"
    ));

    let events = observed.lock().unwrap();
    assert_eq!(events.len(), 1, "exactly one event: {events:?}");
    // A 32 MiB+ result also crosses the loop's 65,536-byte message cap, and
    // the model-view gate (R3) refuses FIRST with that reason: the honest
    // reason a capture of it is not the model's evidence.
    assert!(matches!(
        &events[0],
        CaptureEvent::Refused { sql, connection, reason: CaptureRefusalReason::ModelViewTruncated }
            if sql == "SELECT big" && connection == "primary"
    ));
}
