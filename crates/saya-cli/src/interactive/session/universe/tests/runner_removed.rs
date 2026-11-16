/// The session's toolset carries no `run_program`: it has no runner or
/// interpreter door, so no mode, policy, or grant can
/// advertise it — and `[jobs.runner]` is not a setting, so no config can name
/// a program for a session to compose from either.
#[test]
fn the_session_never_advertises_run_program() {
    let project = worktree("runner-removed");
    let state = temp_dir("runner-removed-state");
    let tools = temp_dir("runner-removed-tools");
    let runtime = session_runtime();
    let universe = compose(&runtime, &project, &state);
    for (agent_mode, policy, can_obtain_approval) in [
        (AgentMode::Build, ApprovalPolicy::Ask, true),
        (AgentMode::Build, ApprovalPolicy::Ask, false),
        (AgentMode::Build, ApprovalPolicy::Bypass, false),
        (AgentMode::Build, ApprovalPolicy::ReadOnly, true),
        (AgentMode::Build, ApprovalPolicy::Never, true),
        (AgentMode::Plan, ApprovalPolicy::Ask, true),
        (AgentMode::Plan, ApprovalPolicy::Bypass, false),
    ] {
        let names = advertised(&universe, agent_mode, policy, can_obtain_approval);
        assert!(
            !names.contains(&"run_program".to_string()),
            "run_program left the session toolset: {agent_mode:?} {policy:?} \
             advertises {names:?}"
        );
    }
    // No grant conjures it either: one token opens one family, and this
    // family is gone.
    granted(&universe, &["runner:touch", "interpreter:python3"]);
    let names = advertised(&universe, AgentMode::Build, ApprovalPolicy::Ask, false);
    assert!(
        !names.contains(&"run_program".to_string()),
        "no grant conjures a removed tool: {names:?}"
    );
    let _ = (
        fs::remove_dir_all(&project),
        fs::remove_dir_all(&state),
        fs::remove_dir_all(&tools),
    );
}

/// The executor refuses `run_program` as an unsupported tool: nothing in the
/// session's composition answers that name — no member claims it, so the
/// composite's own unknown-name refusal is the answer.
#[tokio::test]
async fn the_session_executor_refuses_run_program() {
    let project = worktree("runner-removed-exec");
    let state = temp_dir("runner-removed-exec-state");
    let tools = temp_dir("runner-removed-exec-tools");
    let runtime = session_runtime();
    let universe = compose(&runtime, &project, &state);
    let database = Arc::new(crate::agent::tools::DatabaseTools::new(None, 100, true));
    let executor = universe.executor(
            Arc::clone(&database),
            &CancellationToken::new(),
            None,
            None,
            None,
            Default::default(),
        );
    let refused = executor
        .execute("run_program", serde_json::json!({"program": "touch"}))
        .await
        .expect_err("run_program is not a session tool");
    let refusal = format!("{refused:?}");
    assert!(
        refusal.contains("UnsupportedTool"),
        "no member answers the removed name: {refusal}"
    );
    let _ = (
        fs::remove_dir_all(&project),
        fs::remove_dir_all(&state),
        fs::remove_dir_all(&tools),
    );
}
