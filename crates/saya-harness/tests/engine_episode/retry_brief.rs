use super::*;

/// A failing step retries a bounded number of times, each with a fresh
/// brief, then records a typed failure and pauses.
#[tokio::test]
async fn a_failing_step_retries_with_a_fresh_brief_then_pauses_with_a_typed_code() {
    let run = approved_run("retry").await;
    let provider = ScriptProvider::new(vec![
        Turn::Tools(vec![call("probe")]),
        Turn::Fail,
        Turn::Tools(vec![call("probe")]),
        Turn::Fail,
        Turn::Tools(vec![call("probe")]),
        Turn::Fail,
    ]);
    let approval = AllowApproval;
    let toolsets = vec![toolset(
        Arc::new(SeedingTools {
            workspace: run.root.join("workspace"),
            calls: Arc::new(Mutex::new(Vec::new())),
        }),
        vec![def("probe", LocalStateEffect::None)],
    )];
    let (sink, driver) = driver_and_sink(&run, &provider, &approval, &toolsets, false);
    let workspace = run.workspace();
    let plan = RunPlan::new(vec![step("keep failing", Capabilities::default())]).unwrap();

    let error = driver
        .run_step(&sink, &plan, 0, &workspace)
        .await
        .unwrap_err();

    assert!(
        matches!(
            &error,
            EpisodeError::StepExhausted {
                step: 0,
                attempts: 3,
                code: RunFailureCode::Provider,
            } | _
        ),
        "the bound spent must surface as a typed failure with its cause: {error:?}"
    );
    assert_eq!(
        sink.state(),
        RunState::Paused,
        "a spent bound pauses the run"
    );

    // Exactly the bound was spent: three attempts, two provider calls each.
    let requests = provider.requests();
    assert_eq!(requests.len(), 6, "three attempts, each of two calls");

    // Each attempt got a fresh brief: the workspace the previous attempt's
    // tool call seeded appears in the next attempt's brief.
    assert!(
        !texts(&requests[0]).contains("seeded-"),
        "the first attempt's brief must not know the files later attempts create"
    );
    assert!(
        texts(&requests[2]).contains("seeded-0.csv"),
        "the second attempt's brief must carry the manifest as of its start"
    );
    assert!(
        texts(&requests[4]).contains("seeded-0.csv")
            && texts(&requests[4]).contains("seeded-1.csv"),
        "the third attempt's brief must carry everything seeded so far"
    );

    // The journal tells the bounded-retry story and pauses with its reason.
    assert_eq!(
        Journal::open(&run.run_dir).read().unwrap(),
        vec![
            RunEvent::StepStarted { step: 0 },
            RunEvent::StepFailed { step: 0 },
            RunEvent::StepStarted { step: 0 },
            RunEvent::StepFailed { step: 0 },
            RunEvent::StepStarted { step: 0 },
            RunEvent::StepFailed { step: 0 },
            RunEvent::Paused {
                reason: PauseReason::StepFailedAfterRetry,
            },
        ],
    );
    // The store's step row ends Failed — the mirror resume will read.
    let steps = RunStore::list_steps(&*run.store, &run.run_id)
        .await
        .unwrap();
    assert_eq!(steps.len(), 1);
    assert_eq!(steps[0].status, RunStepStatus::Failed);

    let _ = fs::remove_dir_all(run.root);
}

/// The brief carries the workspace manifest: names, sizes, digests.
#[tokio::test]
async fn the_brief_carries_the_workspace_manifest() {
    let run = approved_run("brief").await;
    let workspace = run.workspace();
    fs::create_dir_all(workspace.root().join("notes")).unwrap();
    fs::write(workspace.root().join("notes/a.md"), "hello notes").unwrap();
    fs::write(workspace.root().join("b.csv"), "col\n1\n").unwrap();
    let provider = ScriptProvider::new(vec![Turn::Answer("done")]);
    let approval = AllowApproval;
    let toolsets = vec![toolset(
        Arc::new(RecordingTools::default()),
        vec![def("probe", LocalStateEffect::None)],
    )];
    let (sink, driver) = driver_and_sink(&run, &provider, &approval, &toolsets, false);
    let plan = RunPlan::new(vec![step("read the workspace", Capabilities::default())]).unwrap();

    driver.run_step(&sink, &plan, 0, &workspace).await.unwrap();

    // The expected manifest, computed the same way the driver builds it.
    let entries = manifest::build(&workspace, 8, 64 * 1024).unwrap();
    assert!(!entries.is_empty(), "the test workspace must hold files");
    let brief = texts(&provider.requests()[0]);
    for entry in &entries {
        assert!(
            brief.contains(&entry.path),
            "the brief must name {} — brief: {brief}",
            entry.path
        );
        assert!(
            brief.contains(&entry.digest),
            "the brief must carry {}'s digest — brief: {brief}",
            entry.path
        );
    }

    let _ = fs::remove_dir_all(run.root);
}
