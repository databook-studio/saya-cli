/// The headline: a successful agent query promotes its pending query AND
/// pairs its own capture — the slot holds exactly the result the model saw,
/// with agent evidence at the model row cap, and one transcript line names
/// the provenance.
#[test]
fn agent_success_captures_exactly_that_result() {
    let (mut app, mut state) = app_and_state();
    let capture = capture_of("SELECT 1", "analytics", 3);
    let expected = capture.result.clone();
    app.request.stream = Some(stream_with(vec![
        sql_request("SELECT 1", Some("analytics")),
        StreamMsg::QueryCaptured(capture),
        completed("3 rows"),
    ]));
    app.drain_stream(&mut state);
    let held = app
        .captured
        .as_ref()
        .expect("the successful agent query's result is captured");
    assert_eq!(
        held.result, expected,
        "the slot holds exactly the result the model saw"
    );
    let evidence = &held.evidence;
    assert_eq!(evidence.source, EvidenceSource::Agent);
    assert_eq!(
        evidence.scope,
        ResultScope::ModelLimited { row_cap: 100 },
        "the agent saw at most the model row cap"
    );
    assert_eq!(evidence.connection_label, "analytics");
    assert_eq!(
        evidence.connection_identity,
        Some("prof-identity-1".to_string()),
        "the profile identity is the capture's"
    );
    assert_eq!(evidence.dialect, SqlDialect::Sqlite);
    assert_eq!(evidence.started_unix_ms, 1_700_000_000_000);
    assert_eq!(evidence.finished_unix_ms, 1_700_000_000_005);
    assert_eq!(evidence.max_rows, 100);
    assert_eq!(evidence.returned_rows, 3);
    let line = last_system_line(&app);
    assert!(
        line.contains("agent: analytics · 3 rows"),
        "the provenance line names source and rows: {line}"
    );
    assert!(
        line.contains("model-limited (first 100 rows)"),
        "the provenance line names the scope: {line}"
    );
}
