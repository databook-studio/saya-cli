use super::*;

// -- 1: the planted secret, full sweep ------------------------------------

/// A resolved credential must appear in exactly one place: the child's
/// environment. The child reports a digest of its own env value — the
/// test compares it against the digest of the planted value, proving the
/// environment held exactly that value — and everything else is swept:
/// the whole run directory recursively, the journal (bytes and parsed
/// events), the store database with its WAL sidecars, the disk records,
/// and the tool result the model received. The generated config is
/// asserted to hold the reference only.
#[tokio::test]
async fn a_resolved_credential_lives_only_in_the_child_s_environment() {
    let cell = cell(
        "sweep",
        &[orchestrator_endpoint()],
        &EndpointBindings::new([(ORCHESTRATOR_ROLE, "primary")]).expect("shaped bindings"),
    )
    .await;
    let var = endpoint_env_var(ORCHESTRATOR_ROLE);
    let provider = ScriptProvider::new(vec![
        Turn::Tools(vec![run_program_call(&["digest", &var])]),
        Turn::Answer("done"),
    ]);
    let tool = cell.tool(
        vec![orchestrator_credential()],
        source_with(&[(ORCHESTRATOR_SOURCE, ORCHESTRATOR_VALUE)]),
    );
    let plan = RunPlan::new(vec![runner_step("measure the child's environment")])
        .expect("well-shaped plan");
    let sink = sink(&cell);
    drive_step(&cell, &sink, tool, &provider, &plan, 0)
        .await
        .expect("the step completes");
    drop(sink);

    // The child's environment held exactly the planted credential — and
    // the raw value rode no captured byte.
    let model_lane = tool_lane(&provider.requests());
    let expected_digest = format!("DIGEST {var} {:016x}", fnv1a(ORCHESTRATOR_VALUE.as_bytes()));
    assert!(
        model_lane.contains(&expected_digest),
        "the child's environment must have held exactly the planted value (digest match) \
             — tool result: {model_lane}"
    );
    assert!(
        !model_lane.contains(ORCHESTRATOR_VALUE),
        "the raw value must not ride the tool result: {model_lane}"
    );

    // Exactly one place: everything else is swept.
    sweep_run(&cell, ORCHESTRATOR_VALUE);
    assert_absent(&record_files(&cell), ORCHESTRATOR_VALUE, "the disk record");

    // Non-vacuity: the child really ran and its record landed, the store
    // really recorded the completed run.
    assert!(
        !record_files(&cell).is_empty(),
        "the disk record must exist for the sweep to be honest"
    );
    let run = RunStore::get_run(&*cell.store, &cell.run_id)
        .await
        .expect("the store reads")
        .expect("the run row exists");
    assert_eq!(run.status, RunStatus::Completed, "the run completed");

    // The generated config holds the reference, never a resolved value.
    let config = fs::read_to_string(cell.run.state().join("config/config.toml"))
        .expect("the generated config reads");
    assert!(
        config.contains(&format!("api_key = {{ env = \"{var}\" }}")),
        "the config binds the run-scoped env reference: {config}"
    );
    assert!(
        !config.contains(ORCHESTRATOR_SOURCE),
        "the original reference must not be echoed into the config: {config}"
    );
    assert!(
        !config.contains(ORCHESTRATOR_VALUE),
        "the resolved value must not reach the config: {config}"
    );

    cleanup(&cell);
}

// -- 3: the four conditions, end to end -----------------------------------

/// Condition "declared" removed: the step's credential list is empty, so
/// the variable never reaches the child's environment — proven through
/// the episode loop, not at the spawn call (the escape battery's
/// `an_undeclared_credential_never_reaches_the_child` owns that shape).
#[tokio::test]
async fn an_undeclared_credential_never_reaches_a_child_through_the_episode_loop() {
    let cell = cell(
        "undeclared",
        &[orchestrator_endpoint()],
        &EndpointBindings::new([(ORCHESTRATOR_ROLE, "primary")]).expect("shaped bindings"),
    )
    .await;
    let var = endpoint_env_var(ORCHESTRATOR_ROLE);
    let provider = ScriptProvider::new(vec![
        Turn::Tools(vec![run_program_call(&["which", &var])]),
        Turn::Answer("done"),
    ]);
    let tool = cell.tool(Vec::new(), source_with(&[]));
    let plan = RunPlan::new(vec![runner_step("probe the environment")]).expect("well-shaped");
    let sink = sink(&cell);
    drive_step(&cell, &sink, tool, &provider, &plan, 0)
        .await
        .expect("the step completes");
    drop(sink);

    let model_lane = tool_lane(&provider.requests());
    assert!(
        model_lane.contains(&format!("UNSET {var}")),
        "an undeclared credential must be absent from the child's environment — the \
             helper reports one line per variable, either SET or UNSET: {model_lane}"
    );
    assert!(
        !model_lane.contains(ORCHESTRATOR_SOURCE),
        "the reference must not ride the tool result: {model_lane}"
    );

    cleanup(&cell);
}

/// Condition "references-only" removed: a declared credential whose
/// reference cannot be resolved fails the call before anything spawns —
/// the model is told exactly that, and no record exists because no child
/// ever ran. The generated-config half of the condition (references
/// only, never values) is asserted in test 1; the escape battery's
/// `an_unresolvable_reference_injects_nothing_and_spawns_nothing` owns
/// the typed-error-at-spawn shape.
#[tokio::test]
async fn an_unresolvable_reference_spawns_no_child_and_names_the_failure_to_the_model() {
    let cell = cell(
        "unresolved",
        &[orchestrator_endpoint()],
        &EndpointBindings::new([(ORCHESTRATOR_ROLE, "primary")]).expect("shaped bindings"),
    )
    .await;
    let var = endpoint_env_var(ORCHESTRATOR_ROLE);
    let dangling = Credential::new(
        "orchestrator-key",
        &var,
        SecretRef::Env {
            env: "SAYA_E2E_MISSING_SOURCE".to_owned(),
        },
    )
    .expect("the declared credential is well-shaped");
    let provider = ScriptProvider::new(vec![
        Turn::Tools(vec![run_program_call(&["digest", &var])]),
        Turn::Answer("done"),
    ]);
    let tool = cell.tool(vec![dangling], source_with(&[]));
    let plan = RunPlan::new(vec![runner_step("measure the environment")]).expect("well-shaped");
    let sink = sink(&cell);
    drive_step(&cell, &sink, tool, &provider, &plan, 0)
        .await
        .expect("the loop survives the refusal and finishes the step");
    drop(sink);

    let model_lane = tool_lane(&provider.requests());
    assert!(
        model_lane.contains("could not be resolved"),
        "the unresolved reference must be fed back to the model as the tool result: \
             {model_lane}"
    );
    assert!(
        model_lane.contains("orchestrator-key"),
        "the failure must name the credential: {model_lane}"
    );
    assert!(
        record_files(&cell).is_empty(),
        "nothing spawned — the runner never produced a record"
    );

    cleanup(&cell);
}
