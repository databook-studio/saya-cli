/// A refusal over the capture budget clears the slot — the previous capture
/// is never silently re-exported as if it were this query's result — and the
/// gap outlives the turn, so a later snapshot still names the reason.
#[test]
fn refused_capture_clears_previous_capture() {
    let (mut app, mut state) = app_and_state();
    app.captured = Some(prev_capture());
    app.request.stream = Some(stream_with(vec![
        sql_request("SELECT 2", Some("analytics")),
        StreamMsg::QueryCaptureRefused {
            sql: "SELECT 2".into(),
            connection: "analytics".into(),
            reason: CaptureRefusalReason::OverBudget,
        },
        completed("1 row"),
    ]));
    app.drain_stream(&mut state);
    assert_eq!(
        selectable_sql(&app),
        Some("SELECT 2"),
        "the successful completion still promotes the selectable query"
    );
    assert!(
        app.captured.is_none(),
        "the refusal clears the slot: no fallback to the older result"
    );
    assert_eq!(app.agent_captures.gap, Some(CaptureGap::OverBudget));
    app.request.stream = Some(stream_with(vec![done()]));
    assert!(app.drain_stream(&mut state), "the turn finished");
    assert_eq!(
        app.agent_captures.gap,
        Some(CaptureGap::OverBudget),
        "the gap outlives the turn: the snapshot may still be asked"
    );
}

/// A failed completion never touches the capture slot: the previous capture
/// survives, nothing is promoted, no gap is recorded.
#[test]
fn failed_agent_query_keeps_previous_capture() {
    let (mut app, mut state) = app_and_state();
    app.captured = Some(prev_capture());
    app.last_query = Some(LastQuery {
        sql: "SELECT 1".into(),
        connection: Some("analytics".into()),
    });
    app.request.stream = Some(stream_with(vec![
        sql_request("SELECT 2", Some("analytics")),
        completed("read-only database tool failed"),
    ]));
    app.drain_stream(&mut state);
    let kept = app
        .captured
        .as_ref()
        .expect("a failed completion never touches the capture slot");
    assert_eq!(kept.result.executed_sql, "SELECT 0", "the previous capture");
    assert_eq!(
        selectable_sql(&app),
        Some("SELECT 1"),
        "a failure never promotes"
    );
    assert_eq!(app.agent_captures.gap, None, "no gap without a promotion");
}
