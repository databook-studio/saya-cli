/// A run's steps run the user's hooks: the `[hooks]` table covers a `saya
/// run`, so a `PreToolUse` hook that blocks a tool blocks it for every step,
/// and the step's model reads the hook's own reason. The gate sits in front of
/// the step's composite, so a name the step advertises is refused with the
/// hook's reason rather than served.
#[cfg(any(target_os = "macos", target_os = "linux"))]
#[tokio::test]
async fn a_pre_tool_use_hook_blocks_a_run_steps_call() {
    let database = Arc::new(DatabaseTools::new(None, 100, true));
    let ws = workspace("run-hooks");
    let reason = "no reads during this run";
    let hooks = crate::agent::hooks::test_support::blocking_hooks(
        reason,
        ws.root(),
        crate::agent::hooks::HookCaller::Run("run-1".to_owned()),
    );
    let built = toolsets(
        ToolsetInputs {
            database: &database,
            fetch: None,
            shell: None,
            workspace: &ws,
            allow_query_data: true,
            tools: &saya_config::ResolvedTools::default(),
            mcp: None,
            hooks: Some(&hooks),
        },
        &[step("read the source", Capabilities::default())],
    );
    assert!(
        names(&built, 0).contains(&"read".to_string()),
        "the fixture's step must advertise the tool the hook blocks"
    );

    let error = built[0]
        .executor
        .execute("read", serde_json::json!({"path": "a.txt"}))
        .await
        .expect_err("the hook must block this call");
    assert!(
        error.to_string().contains(reason),
        "the step's model must read the hook's own reason: {error}"
    );
}
