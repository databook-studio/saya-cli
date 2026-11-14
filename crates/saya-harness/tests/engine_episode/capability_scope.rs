use super::*;

/// A step whose capabilities exclude a tool gets definitions without that
/// tool — the tool is hidden, never advertised-and-refused.
#[tokio::test]
async fn a_tool_outside_the_step_s_capabilities_is_absent_from_definitions() {
    let run = approved_run("narrow").await;
    let provider = ScriptProvider::new(vec![Turn::Answer("done")]);
    let approval = AllowApproval;
    let toolsets = vec![toolset(
        Arc::new(RecordingTools::default()),
        vec![
            def("probe", LocalStateEffect::None),
            def("workspace_writer", LocalStateEffect::WriteWorkspace),
            def("remember", LocalStateEffect::WriteCandidate),
        ],
    )];
    let (sink, driver) = driver_and_sink(&run, &provider, &approval, &toolsets, false);
    let workspace = run.workspace();
    let plan = RunPlan::new(vec![step("read the schema", Capabilities::default())]).unwrap();

    driver.run_step(&sink, &plan, 0, &workspace).await.unwrap();

    // The loop received the definitions, not a refusal: a step without the
    // workspace-write capability never sees the workspace-write tool.
    let received: Vec<String> = provider.requests()[0]
        .tools
        .iter()
        .map(|tool| tool.name.clone())
        .collect();
    assert_eq!(
        received,
        vec!["probe", "remember"],
        "the workspace-write tool must be absent from the step's definitions"
    );
    assert_eq!(sink.state(), RunState::Completed);

    let _ = fs::remove_dir_all(run.root);
}
