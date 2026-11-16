#[test]
fn query_results_render_as_text_and_json_without_diagnostics() {
    let result = saya_types::QueryResult {
        columns: vec!["id".into()],
        rows: vec![serde_json::json!([1])],
        row_count: 1,
        truncated: true,
        executed_sql: "SELECT id".into(),
    };
    let text = render_event(
        &TerminalEvent::QueryResult {
            result: result.clone(),
        },
        RenderFormat::Text,
    );
    assert_eq!(text.stdout, "id\n1\n[truncated]\n");
    assert!(text.stderr.is_empty());
    let json = render_event(&TerminalEvent::QueryResult { result }, RenderFormat::Json);
    assert!(json.stdout.contains("\"event\":\"query_result\""));
    assert!(json.stderr.is_empty());
}
