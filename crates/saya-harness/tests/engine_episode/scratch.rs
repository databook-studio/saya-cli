use super::*;

/// The scratch scope's end-to-end gate: a step that asked for scratch — and
/// only scratch, no `workspace_write` anywhere in its capabilities — runs
/// DDL through `scratch_sql` in a real episode. The definition rides the
/// toolset's universe, the write permit arrives through the scope→permit
/// union rather than `workspace_write`, and the statement lands in the
/// run's scratch file. If this fails, the union or the write-shaped filter
/// is wrong, not the test.
#[tokio::test]
async fn a_scratch_only_step_runs_ddl_through_scratch_sql() {
    use saya_harness::scratch::ScratchSql;

    let run = approved_run("scratch").await;
    let mut scratch_caps = Capabilities::default();
    scratch_caps.scratch = true;
    let scratch = Arc::new(
        ScratchSql::admit(&run.run_dir, &scratch_caps)
            .unwrap()
            .expect("the run approved scratch, so admission admits"),
    );
    let provider = ScriptProvider::new(vec![
        Turn::Tools(vec![ToolCall {
            id: "c-create".into(),
            name: "scratch_sql".into(),
            arguments: serde_json::json!({"sql": "CREATE TABLE staged (a INTEGER)"}),
        }]),
        Turn::Tools(vec![ToolCall {
            id: "c-insert".into(),
            name: "scratch_sql".into(),
            arguments: serde_json::json!({"sql": "INSERT INTO staged VALUES (42)"}),
        }]),
        Turn::Answer("done"),
    ]);
    let approval = AllowApproval;
    let toolsets = vec![toolset(
        scratch.clone(),
        ScratchSql::definitions(&scratch_caps),
    )];
    let (sink, driver) = driver_and_sink(&run, &provider, &approval, &toolsets, false);
    let workspace = run.workspace();
    let plan = RunPlan::new(vec![step("stage results", scratch_caps.clone())]).unwrap();

    driver.run_step(&sink, &plan, 0, &workspace).await.unwrap();

    assert_eq!(sink.state(), RunState::Completed);

    // The DDL and the DML landed in the run's scratch file: the write was
    // auto-run — no workspace-write scope anywhere, so only the permit
    // union carried it — and the staged row reads back.
    let read_back = scratch
        .run("SELECT a FROM staged")
        .await
        .expect("the table the DDL created must exist in the run's scratch file");
    assert!(
        serde_json::to_string(&read_back).unwrap().contains("42"),
        "the inserted row must be staged in the scratch file, got: {read_back:?}"
    );

    let _ = fs::remove_dir_all(run.root);
}
