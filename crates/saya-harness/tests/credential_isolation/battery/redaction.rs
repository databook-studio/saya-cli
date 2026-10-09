use super::*;

// -- 2: a secret in captured stdout is redacted to the model and to disk --

/// A child that echoes the credential it legitimately received — under
/// the env name the generated config binds, next to the recognised
/// secret shapes — is the credential leaving through captured output.
/// The model lane (the `tool` message the loop hands the provider) and
/// the disk lane (the record the runner persists into the run
/// workspace) are asserted separately: different code paths, and one has
/// been wrong before. This test exposed the hole the capture-boundary
/// fix closes: pattern redaction alone cannot recognise a bare value.
#[tokio::test]
async fn a_secret_in_captured_stdout_is_redacted_before_the_model_and_before_disk() {
    let cell = cell(
        "exfiltration",
        &[orchestrator_endpoint()],
        &EndpointBindings::new([(ORCHESTRATOR_ROLE, "primary")]).expect("shaped bindings"),
    )
    .await;
    let var = endpoint_env_var(ORCHESTRATOR_ROLE);
    let provider = ScriptProvider::new(vec![
        Turn::Tools(vec![run_program_call(&["leak", &var])]),
        Turn::Answer("done"),
    ]);
    let tool = cell.tool(
        vec![orchestrator_credential()],
        source_with(&[(ORCHESTRATOR_SOURCE, ORCHESTRATOR_VALUE)]),
    );
    let plan = RunPlan::new(vec![runner_step("echo the environment")]).expect("well-shaped");
    let sink = sink(&cell);
    drive_step(&cell, &sink, tool, &provider, &plan, 0)
        .await
        .expect("the step completes");
    drop(sink);

    let forbidden = [
        ORCHESTRATOR_VALUE,
        "hunter2-marker-shape",
        "sk-live-shape-marker",
        "hunter2-url-marker",
    ];
    // The model lane: the tool message the provider actually received.
    let model_lane = tool_lane(&provider.requests());
    assert!(
        model_lane.contains("SAYA_RUN_EP_ORCHESTRATOR=[redacted]"),
        "the env-name shape must survive with its value scrubbed — this is what proves \
             the child really echoed the credential: {model_lane}"
    );
    for secret in &forbidden {
        assert!(
            !model_lane.contains(secret),
            "the model lane carries a planted secret: {model_lane}"
        );
    }
    assert!(
        model_lane.contains("[redacted]"),
        "the model lane must show the redaction happened: {model_lane}"
    );

    // The disk lane: the record files the runner persisted, read from
    // disk — not from memory.
    let records = record_files(&cell);
    assert!(
        !records.is_empty(),
        "the runner's disk records must exist for the disk lane to be honest"
    );
    let disk_lane = records
        .iter()
        .map(|path| fs::read_to_string(path).expect("the record reads"))
        .collect::<Vec<_>>()
        .join("\n");
    for secret in &forbidden {
        assert!(
            !disk_lane.contains(secret),
            "the disk lane carries a planted secret"
        );
    }
    assert!(
        disk_lane.contains("[redacted]"),
        "the disk lane must show the redaction happened"
    );

    // The failure of either lane would also show in the full sweep.
    sweep_run(&cell, ORCHESTRATOR_VALUE);
    for secret in [
        "hunter2-marker-shape",
        "sk-live-shape-marker",
        "hunter2-url-marker",
    ] {
        sweep_run(&cell, secret);
    }

    cleanup(&cell);
}

// -- 5: the failure path scrubs too ----------------------------------------

/// The child ran three times (one per bounded attempt), each time
/// echoing the live credential; the provider failed after every attempt;
/// the bound was spent and the run paused mid-flight. The journal and
/// the store — the two records a resume reads — carry no credential
/// material, in their bytes or their rows.
#[tokio::test]
async fn a_failed_run_s_journal_and_store_carry_no_credential_material() {
    let cell = cell(
        "failed",
        &[orchestrator_endpoint()],
        &EndpointBindings::new([(ORCHESTRATOR_ROLE, "primary")]).expect("shaped bindings"),
    )
    .await;
    let var = endpoint_env_var(ORCHESTRATOR_ROLE);
    let leak_call = run_program_call(&["leak", &var]);
    let provider = ScriptProvider::new(vec![
        Turn::Tools(vec![leak_call.clone()]),
        Turn::Fail,
        Turn::Tools(vec![leak_call.clone()]),
        Turn::Fail,
        Turn::Tools(vec![leak_call]),
        Turn::Fail,
    ]);
    let tool = cell.tool(
        vec![orchestrator_credential()],
        source_with(&[(ORCHESTRATOR_SOURCE, ORCHESTRATOR_VALUE)]),
    );
    let plan = RunPlan::new(vec![runner_step("keep failing")]).expect("well-shaped plan");
    let sink = sink(&cell);
    let error = drive_step(&cell, &sink, tool, &provider, &plan, 0)
        .await
        .expect_err("the step exhausts its bounded attempts");
    assert!(
        matches!(
            &error,
            EpisodeError::StepExhausted {
                step: 0,
                attempts: 3,
                ..
            }
        ),
        "the spent bound must surface as the typed exhaustion: {error:?}"
    );
    assert_eq!(
        sink.state(),
        RunState::Paused,
        "a spent bound pauses the run"
    );
    drop(sink);

    // The journal tells the bounded-retry story, paused — with no
    // credential material anywhere in it.
    let events = Journal::open(cell.run.root())
        .read()
        .expect("the journal parses");
    assert_eq!(
        events,
        vec![
            RunEvent::RunStarted,
            RunEvent::PlanApproved { scopes: None },
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
        "the journal must carry exactly the bounded-retry story"
    );

    // The store's rows exist — and carry no credential material.
    let run = RunStore::get_run(&*cell.store, &cell.run_id)
        .await
        .expect("the store reads")
        .expect("the run row exists");
    assert_eq!(run.status, RunStatus::Paused, "the run paused");
    assert!(
        !format!("{run:?}").contains(ORCHESTRATOR_VALUE),
        "the store's run row carries credential material: {run:?}"
    );
    let steps = RunStore::list_steps(&*cell.store, &cell.run_id)
        .await
        .expect("the store reads");
    assert_eq!(steps.len(), 1, "one step row");
    assert_eq!(steps[0].status, RunStepStatus::Failed, "the step failed");
    assert!(
        !format!("{:?}", steps[0]).contains(ORCHESTRATOR_VALUE),
        "the store's step row carries credential material: {:?}",
        steps[0]
    );

    // The byte sweep over the store, the journal, and the whole run
    // directory — the failure path is exactly where scrubbing is skipped.
    sweep_run(&cell, ORCHESTRATOR_VALUE);
    for secret in [
        "hunter2-marker-shape",
        "sk-live-shape-marker",
        "hunter2-url-marker",
    ] {
        sweep_run(&cell, secret);
    }

    // Non-vacuity: the credential was live in all three attempts, and
    // every attempt's captured output was scrubbed on its way to disk.
    let records = record_files(&cell);
    assert_eq!(records.len(), 3, "the three attempts each ran the child");
    for record in &records {
        let text = fs::read_to_string(record).expect("the record reads");
        assert!(
            !text.contains(ORCHESTRATOR_VALUE),
            "the failed run's record carries the credential: {record:?}"
        );
        assert!(
            text.contains("[redacted]"),
            "the failed attempt's record must show the redaction: {record:?}"
        );
    }

    cleanup(&cell);
}
