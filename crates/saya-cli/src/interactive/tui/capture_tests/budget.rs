use super::super::capture;
use super::*;

#[test]
fn capture_budget_has_visible_failure() {
    let big = saya_types::QueryResult {
        columns: vec!["text".to_string()],
        rows: vec![serde_json::json!(["x".repeat(200)])],
        row_count: 1,
        truncated: false,
        executed_sql: "SELECT text FROM t".to_string(),
    };
    // The parameterised walk the production capture uses: a small budget
    // refuses the big result and admits the small one.
    assert!(
        capture::accounted_bytes_within(&big, 64).is_none(),
        "over a small budget the walk refuses"
    );
    assert!(
        capture::accounted_bytes_within(&result(1, false), 64).is_some(),
        "a small result stays within a small budget"
    );

    let mut transcript = Transcript::new();
    let mut captured = Some(seed_capture());
    let envelope_task = sql_task();
    plant_envelope(&envelope_task, &big, runtime().resolved.max_rows);
    let evidence =
        capture::direct_sql_evidence_from_execution(Some("analytics"), &take(&envelope_task));
    let accounted = capture::accounted_bytes_within(&big, 64);
    capture::capture_within(&mut captured, big, evidence, accounted, &mut transcript);
    assert!(
        captured.is_none(),
        "over budget clears the previous capture"
    );
    let note = last_block(&transcript, BlockKind::System).expect("the refusal is visible");
    assert!(note.contains("Result not captured"), "{note}");
    assert!(
        note.contains("/export --refresh will re-run it"),
        "the refusal names the way out: {note}"
    );
}

#[test]
fn accounting_stops_at_the_budget_boundary() {
    // One 60-char cell over a 100-byte budget fits (60 + column "text" = 64);
    // a second identical row passes it — the walk refuses there, not after
    // measuring everything.
    let row = serde_json::json!(["x".repeat(60)]);
    let one = saya_types::QueryResult {
        columns: vec!["text".to_string()],
        rows: vec![row.clone()],
        row_count: 1,
        truncated: false,
        executed_sql: "SELECT text FROM t".to_string(),
    };
    assert_eq!(capture::accounted_bytes_within(&one, 100), Some(64));
    let two = saya_types::QueryResult {
        rows: vec![row.clone(), row],
        row_count: 2,
        ..one.clone()
    };
    assert_eq!(capture::accounted_bytes_within(&two, 100), None);
    // Arrays and objects are walked: a nested 200-char string refuses a
    // 64-byte budget.
    let nested = saya_types::QueryResult {
        columns: vec!["j".to_string()],
        rows: vec![serde_json::json!({"k": ["x".repeat(200)]})],
        row_count: 1,
        truncated: false,
        executed_sql: "SELECT j FROM t".to_string(),
    };
    assert_eq!(capture::accounted_bytes_within(&nested, 64), None);
    assert!(
        capture::accounted_bytes(&one).is_some(),
        "the production budget admits a small result"
    );
}
