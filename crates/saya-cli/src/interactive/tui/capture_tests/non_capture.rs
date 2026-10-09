use super::*;

#[test]
fn failed_sql_keeps_previous_capture_untouched() {
    let seed = seed_capture();
    let mut transcript = Transcript::new();
    let mut captured = Some(seed);
    complete(
        &sql_task(),
        TerminalEvent::Error {
            message: "connection refused".into(),
        },
        &mut transcript,
        &mut None,
        &mut captured,
    );
    let kept = captured.expect("a failed /sql never clears the previous capture");
    assert_eq!(
        kept.result.row_count, 1,
        "the previous result is still held"
    );
    assert!(
        last_block(&transcript, BlockKind::Error).is_some(),
        "the failure still rendered as an error block"
    );
}

#[test]
fn chart_and_explain_do_not_capture() {
    // /explain renders a plan table; the capture must not move.
    let mut transcript = Transcript::new();
    let mut captured = Some(seed_capture());
    let explain = SqlTask::new(
        Some("analytics".to_string()),
        "EXPLAIN SELECT id FROM t".to_string(),
        Followup::Explain,
    );
    complete(
        &explain,
        TerminalEvent::QueryResult {
            result: result(2, false),
        },
        &mut transcript,
        &mut None,
        &mut captured,
    );
    assert_eq!(
        captured
            .as_ref()
            .expect("explain leaves the capture")
            .result
            .row_count,
        1,
        "the previous capture survives /explain"
    );

    // /chart with a chartable result, written to a path that cannot exist so
    // no temp chart is reserved and no browser is opened. Even this
    // successful-render follow-up must leave the capture alone.
    let mut transcript = Transcript::new();
    let mut captured = Some(seed_capture());
    let chart = SqlTask::new(
        Some("analytics".to_string()),
        "SELECT id FROM t".to_string(),
        Followup::Chart {
            kind: None,
            path: Some("/nonexistent-saya-test-dir/chart.html".into()),
        },
    );
    complete(
        &chart,
        TerminalEvent::QueryResult {
            result: result(2, false),
        },
        &mut transcript,
        &mut None,
        &mut captured,
    );
    assert_eq!(
        captured
            .as_ref()
            .expect("chart leaves the capture")
            .result
            .row_count,
        1,
        "the previous capture survives /chart"
    );
}

#[test]
fn dialect_unresolvable_does_not_capture_or_say_anything() {
    // An empty slot — no envelope was written — means no evidence, no
    // capture, and nothing extra said: a dialect is never guessed.
    let mut transcript = Transcript::new();
    let mut captured = Some(seed_capture());
    let mut task = sql_task();
    task.profile = Some("not-a-profile".to_string());
    complete(
        &task,
        TerminalEvent::QueryResult {
            result: result(3, false),
        },
        &mut transcript,
        &mut None,
        &mut captured,
    );
    let kept = captured.expect("an unresolvable dialect leaves the previous capture");
    assert_eq!(kept.result.row_count, 1);
    let table = last_block(&transcript, BlockKind::Table).expect("the result still renders");
    assert!(
        !table.contains("direct sql"),
        "nothing extra is said: {table:?}"
    );
    assert!(
        last_block(&transcript, BlockKind::System).is_none(),
        "no system note either"
    );
}

#[test]
fn a_successful_completion_without_an_envelope_produces_no_evidence() {
    // An empty slot — the worker never wrote an envelope — renders the
    // table with NO evidence line and captures nothing: `complete` never
    // derives provenance from the live runtime.
    let mut transcript = Transcript::new();
    let mut captured = Some(seed_capture());
    let task = sql_task();
    let rows = result(3, false);
    complete(
        &task,
        TerminalEvent::QueryResult {
            result: rows.clone(),
        },
        &mut transcript,
        &mut None,
        &mut captured,
    );
    assert!(
        captured.is_some(),
        "the previous capture survives an envelope-less success"
    );
    assert_eq!(
        captured.as_ref().expect("kept").result.row_count,
        1,
        "no new capture replaces it"
    );
    let table = last_block(&transcript, BlockKind::Table).expect("the result still renders");
    assert!(
        !table.contains("direct sql"),
        "no evidence line is synthesized: {table:?}"
    );
    assert!(
        table.contains("from analytics · SELECT id FROM t"),
        "the scope line still renders: {table:?}"
    );
}
