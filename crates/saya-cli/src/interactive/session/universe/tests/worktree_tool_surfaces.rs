/// Outside any worktree with no `--workspace`: the write-shaped file tools
/// and `run_program` are absent from the definitions, `scratch_sql` and
/// `http_fetch` still work, and the workspace reads stay advertised with
/// their typed no-workspace refusal at dispatch.
#[test]
fn outside_a_worktree_the_write_shaped_tools_are_hidden_and_scratch_and_fetch_work() {
    let plain = temp_dir("no-worktree-universe");
    let state = temp_dir("no-worktree-state");
    let universe =
        SessionUniverse::compose(&session_runtime(None), None, None, true, &plain, &state)
            .expect("composition succeeds without a root");
    let names = advertised(&universe, AgentMode::Build, ApprovalPolicy::Ask, true);
    assert!(
        !names.contains(&"workspace_write".to_string()),
        "no root, no write tool: {names:?}"
    );
    assert!(
        !names.contains(&"http_download".to_string()),
        "no root, no download destination: {names:?}"
    );
    assert!(
        !names.contains(&"run_program".to_string()),
        "no root, no runner: {names:?}"
    );
    assert!(
        !names.contains(&"scratch_import".to_string()),
        "no root, no contained import: {names:?}"
    );
    assert!(
        names.contains(&"scratch_sql".to_string()),
        "scratch needs no root: {names:?}"
    );
    assert!(
        names.contains(&"http_fetch".to_string()),
        "fetch needs no root: {names:?}"
    );
    assert!(universe.root().is_none());
    let _ = (fs::remove_dir_all(&plain), fs::remove_dir_all(&state));
}

/// Inside a worktree with prompts possible: the write-shaped tools are
/// advertised, each ask-gated — one engine, no scope flags. `run_program`
/// joins them only where the probe proved the host, so its advertisement is
/// pinned with the proven-host test below; its absence here (no
/// `program_dir` configured — nothing composes a runner) is the honest
/// shape, not a hidden capability.
#[test]
fn a_worktree_session_advertises_every_write_shaped_tool_ask_gated() {
    let project = worktree("ask-universe");
    let state = temp_dir("ask-universe-state");
    let universe = compose(&session_runtime(None), &project, &state);
    let names = advertised(&universe, AgentMode::Build, ApprovalPolicy::Ask, true);
    for tool in [
        "workspace_write",
        "workspace_edit",
        "scratch_sql",
        "scratch_import",
        "http_fetch",
        "http_download",
    ] {
        assert!(
            names.contains(&tool.to_string()),
            "an ask session advertises {tool}: {names:?}"
        );
    }
    let definitions = universe.definitions(
        saya_agent::AgentMode::Build,
        ApprovalPolicy::Ask,
        true,
        true,
        false,
        false,
    );
    for tool in [
        "workspace_write",
        "workspace_edit",
        "scratch_sql",
        "scratch_import",
        "http_fetch",
        "http_download",
    ] {
        let definition = definitions
            .iter()
            .find(|definition| definition.name == tool)
            .expect("the write-shaped tool is advertised");
        assert!(
            definition.effect.requires_approval,
            "{tool} is ask-gated: the engine decides every call"
        );
    }
    let _ = (fs::remove_dir_all(&project), fs::remove_dir_all(&state));
}

#[tokio::test]
async fn advertised_scratch_import_routes_through_the_session_executor() {
    let project = worktree("scratch-import-executor");
    let state = temp_dir("scratch-import-executor-state");
    std::fs::write(project.join("data.csv"), b"name\nAda\n").unwrap();
    let universe = compose(&session_runtime(None), &project, &state);
    assert!(
        advertised(&universe, AgentMode::Build, ApprovalPolicy::Bypass, false)
            .contains(&"scratch_import".to_owned())
    );
    let database = Arc::new(
        crate::agent::tools::DatabaseTools::new(None, 100, true)
            .with_workspace(universe.workspace()),
    );
    let result = universe
        .executor(database, &CancellationToken::new(), false)
        .execute(
            "scratch_import",
            serde_json::json!({"path":"data.csv","table":"people"}),
        )
        .await
        .expect("advertised import routes to scratch");
    assert_eq!(result["rows_imported"], 1);
    let _ = (fs::remove_dir_all(project), fs::remove_dir_all(state));
}
