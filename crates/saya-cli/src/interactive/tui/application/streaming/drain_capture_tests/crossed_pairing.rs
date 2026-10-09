/// Pairing is by content, not arrival order: two queries' captures arrive
/// crossed, and each successful completion still promotes its own capture.
#[test]
fn capture_pairs_by_sql_and_connection_not_order() {
    let (mut app, mut state) = app_and_state();
    let b = capture_of("SELECT 2", "analytics", 2);
    let a = capture_of("SELECT 1", "analytics", 1);
    app.request.stream = Some(stream_with(vec![
        sql_request("SELECT 1", Some("analytics")),
        sql_request("SELECT 2", Some("analytics")),
        StreamMsg::QueryCaptured(b),
        StreamMsg::QueryCaptured(a),
        completed("1 row"),
    ]));
    app.drain_stream(&mut state);
    let held = app
        .captured
        .as_ref()
        .expect("A's completion pairs its capture");
    assert_eq!(
        held.result.executed_sql, "SELECT 1",
        "A's completion took A's capture, though B's arrived first"
    );
    app.request.stream = Some(stream_with(vec![completed("2 rows")]));
    app.drain_stream(&mut state);
    let held = app
        .captured
        .as_ref()
        .expect("B's completion pairs its capture");
    assert_eq!(
        held.result.executed_sql, "SELECT 2",
        "B's completion took B's capture"
    );
}
