use super::*;

/// Candidate writes are pinned off even when the caller carries the user's
/// memory-mode permission to write candidates (DESIGN §5.8): the tool is
/// advertised (so the pin, not hiding, is what stops it) and the loop
/// refuses it.
#[tokio::test]
async fn candidate_writes_are_pinned_off_even_when_memory_would_permit_them() {
    let run = approved_run("pin").await;
    let provider = ScriptProvider::new(vec![
        Turn::Tools(vec![call("remember")]),
        Turn::Answer("noted"),
    ]);
    let tools = Arc::new(RecordingTools::default());
    let approval = AllowApproval;
    let toolsets = vec![toolset(
        tools.clone(),
        vec![
            def("probe", LocalStateEffect::None),
            def("remember", LocalStateEffect::WriteCandidate),
        ],
    )];
    let (sink, driver) = driver_and_sink(&run, &provider, &approval, &toolsets, true);
    let workspace = run.workspace();
    let plan = RunPlan::new(vec![step("record a claim", Capabilities::default())]).unwrap();

    driver.run_step(&sink, &plan, 0, &workspace).await.unwrap();

    // The tool was advertised, so the refusal came from the pinned limit —
    // not from the definitions hiding it.
    let received: Vec<String> = provider.requests()[0]
        .tools
        .iter()
        .map(|tool| tool.name.clone())
        .collect();
    assert!(
        received.contains(&"remember".to_string()),
        "a candidate-write tool is not capability-scoped; it must be advertised: {received:?}"
    );
    // The loop refused it: the executor never ran.
    assert_eq!(
        tools.calls(),
        Vec::<String>::new(),
        "a candidate-write tool must never execute in an episode"
    );
    // The model was told the call was denied, not left guessing.
    assert!(
        texts(&provider.requests()[1]).contains("denied by approval policy"),
        "the denial must reach the model as the tool result"
    );
    assert_eq!(sink.state(), RunState::Completed);

    let _ = fs::remove_dir_all(run.root);
}
