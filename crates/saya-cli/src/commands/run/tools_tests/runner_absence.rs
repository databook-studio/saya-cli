/// A step whose capabilities ask for the runner gets nothing when the run's
/// runner wiring is absent — an unproven host, or a run that approved the
/// scope on a host the probe refused. The tool has no definition and the
/// composite refuses the name as an unknown tool: the capability is absent,
/// never degraded. (The definitions half needs no proven spawn.)
#[tokio::test]
async fn run_program_is_absent_where_no_runner_wiring_exists() {
    let database = Arc::new(DatabaseTools::new(None, 100, true));
    let workspace = workspace("runner-absent");
    let mut runner_caps = Capabilities::default();
    runner_caps.runner = Some(RunnerScope::new(vec!["bench".to_owned()]).expect("shaped"));
    let built = toolsets(
        ToolsetInputs {
            database: &database,
            scratch: None,
            fetch: None,
            runner: None,
            workspace: &workspace,
            allow_query_data: true,
            cancellation: &cancellation(),
        },
        &[step("run the harness", runner_caps.clone())],
    );
    assert_eq!(
        names(&built, 0),
        OPEN_GATE_WITH_CHART,
        "without wiring the runner tool must be absent from the universe"
    );
    let error = built[0]
        .executor
        .execute("run_program", serde_json::json!({}))
        .await
        .expect_err("no wiring means no member behind the composite");
    assert_eq!(error, ToolError::UnsupportedTool);
}
