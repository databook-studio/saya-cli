/// The host-command lane: a held `command:` token advertises `run_command`
/// where the lane composed — and nothing where it did not (no workspace
/// root, no lane, structural). On Windows the lane is unavailable and stays
/// hidden, grant or no grant.
#[test]
fn a_headless_ask_session_advertises_run_command_only_where_the_lane_composed() {
    let project = worktree("grant-command");
    let state = temp_dir("grant-command-state");
    let universe = compose(&session_runtime(None), &project, &state);
    granted(&universe, &["command:cargo"]);
    let names = advertised(&universe, AgentMode::Build, ApprovalPolicy::Ask, false);
    if cfg!(windows) {
        assert!(
            !names.contains(&"run_command".to_string()),
            "Windows keeps the unavailable lane hidden under a grant: {names:?}"
        );
    } else {
        assert!(
            names.contains(&"run_command".to_string()),
            "the held command grant advertises the composed lane under headless ask: {names:?}"
        );
    }
    for tool in [
        "workspace_write",
        "workspace_edit",
        "scratch_sql",
        "scratch_import",
        "http_fetch",
        "http_download",
        "run_program",
        "render_chart",
    ] {
        assert!(
            !names.contains(&tool.to_string()),
            "a command grant widens no other family: {tool} stays hidden: {names:?}"
        );
    }
    // No root, no lane: the grant gates nothing.
    let plain = temp_dir("grant-command-plain");
    let plain_state = temp_dir("grant-command-plain-state");
    let unbound = SessionUniverse::compose(
        &session_runtime(None),
        None,
        None,
        true,
        &plain,
        &plain_state,
    )
    .expect("composition succeeds without a root");
    granted(&unbound, &["command:cargo"]);
    let names = advertised(&unbound, AgentMode::Build, ApprovalPolicy::Ask, false);
    assert!(
        !names.contains(&"run_command".to_string()),
        "no root, no lane, grant or no grant: {names:?}"
    );
    let _ = (
        fs::remove_dir_all(&project),
        fs::remove_dir_all(&state),
        fs::remove_dir_all(&plain),
        fs::remove_dir_all(&plain_state),
    );
}

/// The runner family, where a runner composed: a `runner:` token for an
/// allowlisted program — or an `interpreter:` token for a staged one —
/// advertises `run_program` under headless ask. Anywhere the runner did
/// not compose (any platform, no program_dir), the same grants advertise
/// nothing: a capability the composition could not construct is never
/// conjured by a grant.
#[cfg(target_os = "macos")]
#[test]
fn a_headless_ask_session_advertises_run_program_under_runner_and_interpreter_grants() {
    let project = worktree("grant-runner");
    let state = temp_dir("grant-runner-state");
    let mut runtime = session_runtime(Some((
        vec!["touch".to_string()],
        Some(PathBuf::from("/usr/bin")),
    )));
    runtime.resolved.jobs.interpreter.allow = vec!["python3".to_string()];
    let universe = compose(&runtime, &project, &state);
    assert!(
        universe.runner.is_some(),
        "the probe proves this host (macOS): the runner composes"
    );
    granted(&universe, &["runner:touch"]);
    let names = advertised(&universe, AgentMode::Build, ApprovalPolicy::Ask, false);
    assert!(
        names.contains(&"run_program".to_string()),
        "the held runner grant advertises the composed runner under headless ask: {names:?}"
    );
    for tool in [
        "workspace_write",
        "workspace_edit",
        "scratch_sql",
        "scratch_import",
        "http_fetch",
        "http_download",
        "run_command",
        "render_chart",
    ] {
        assert!(
            !names.contains(&tool.to_string()),
            "a runner grant widens no other family: {tool} stays hidden: {names:?}"
        );
    }
    // The interpreter door rides `run_program`'s advertisement: a staged
    // interpreter's token opens the same single tool.
    granted(&universe, &["interpreter:python3"]);
    let names = advertised(&universe, AgentMode::Build, ApprovalPolicy::Ask, false);
    assert!(
        names.contains(&"run_program".to_string()),
        "the interpreter token opens the same composed runner tool: {names:?}"
    );
    let _ = (fs::remove_dir_all(&project), fs::remove_dir_all(&state));
}

#[test]
fn a_runner_grant_advertises_nothing_where_no_runner_composed() {
    let project = worktree("grant-runner-absent");
    let state = temp_dir("grant-runner-absent-state");
    let universe = compose(&session_runtime(None), &project, &state);
    granted(&universe, &["runner:touch", "interpreter:python3"]);
    let names = advertised(&universe, AgentMode::Build, ApprovalPolicy::Ask, false);
    assert!(
        !names.contains(&"run_program".to_string()),
        "no composed runner, no run_program — grant or no grant: {names:?}"
    );
    let _ = (fs::remove_dir_all(&project), fs::remove_dir_all(&state));
}
