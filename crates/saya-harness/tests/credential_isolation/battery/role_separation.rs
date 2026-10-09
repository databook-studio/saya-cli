use super::*;

// -- 4: a role's credential is not visible to another role's child --------

/// One run, two steps: step 0 runs as the reviewer role (only the
/// reviewer credential declared), step 1 as the orchestrator role (only
/// the orchestrator credential declared). Each child probes both env
/// variables by name: its own is SET, the other role's is UNSET — and
/// the generated config binds each role to its own run-scoped env
/// variable.
#[tokio::test]
async fn a_role_a_credential_is_not_visible_to_a_child_running_as_role_b() {
    let bindings =
        EndpointBindings::new([(ORCHESTRATOR_ROLE, "primary"), ("reviewer", "reviewer-ep")])
            .expect("shaped bindings");
    let cell = cell(
        "roles",
        &[orchestrator_endpoint(), reviewer_endpoint()],
        &bindings,
    )
    .await;
    let reviewer_var = endpoint_env_var("reviewer");
    let orchestrator_var = endpoint_env_var(ORCHESTRATOR_ROLE);
    let source = source_with(&[
        (ORCHESTRATOR_SOURCE, ORCHESTRATOR_VALUE),
        (REVIEWER_SOURCE, REVIEWER_VALUE),
    ]);
    let plan = RunPlan::new(vec![
        runner_step("act as the reviewer"),
        runner_step("act as the orchestrator"),
    ])
    .expect("well-shaped plan");
    let sink = sink(&cell);

    let reviewer_tool = cell.tool(vec![reviewer_credential()], source.clone());
    let reviewer_provider = ScriptProvider::new(vec![
        Turn::Tools(vec![run_program_call(&[
            "which",
            &reviewer_var,
            &orchestrator_var,
        ])]),
        Turn::Answer("done"),
    ]);
    drive_step(&cell, &sink, reviewer_tool, &reviewer_provider, &plan, 0)
        .await
        .expect("the reviewer step completes");

    let orchestrator_tool = cell.tool(vec![orchestrator_credential()], source.clone());
    let orchestrator_provider = ScriptProvider::new(vec![
        Turn::Tools(vec![run_program_call(&[
            "which",
            &reviewer_var,
            &orchestrator_var,
        ])]),
        Turn::Answer("done"),
    ]);
    drive_step(
        &cell,
        &sink,
        orchestrator_tool,
        &orchestrator_provider,
        &plan,
        1,
    )
    .await
    .expect("the orchestrator step completes");
    drop(sink);

    let reviewer_lane = tool_lane(&reviewer_provider.requests());
    assert!(
        reviewer_lane.contains(&format!("SET {reviewer_var}")),
        "the reviewer child must see its own role's variable: {reviewer_lane}"
    );
    assert!(
        reviewer_lane.contains(&format!("UNSET {orchestrator_var}")),
        "the reviewer child must not see the orchestrator's variable: {reviewer_lane}"
    );
    let orchestrator_lane = tool_lane(&orchestrator_provider.requests());
    assert!(
        orchestrator_lane.contains(&format!("SET {orchestrator_var}")),
        "the orchestrator child must see its own role's variable: {orchestrator_lane}"
    );
    assert!(
        orchestrator_lane.contains(&format!("UNSET {reviewer_var}")),
        "the orchestrator child must not see the reviewer's variable: {orchestrator_lane}"
    );

    // Neither raw value reached anything the run wrote down.
    sweep_run(&cell, ORCHESTRATOR_VALUE);
    sweep_run(&cell, REVIEWER_VALUE);

    // The generated config binds each role to its own env variable.
    let config = fs::read_to_string(cell.run.state().join("config/config.toml"))
        .expect("the generated config reads");
    assert!(
        config.contains(&format!("api_key = {{ env = \"{reviewer_var}\" }}")),
        "the reviewer role binds its own env reference: {config}"
    );
    assert!(
        config.contains(&format!("api_key = {{ env = \"{orchestrator_var}\" }}")),
        "the orchestrator role binds its own env reference: {config}"
    );

    cleanup(&cell);
}
