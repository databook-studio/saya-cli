/// The session's executor serves the shell by name — the model's `bash`
/// spelling included — while every other name keeps the shared composite's
/// own answer. Skipped where the sandbox does not prove.
#[tokio::test]
async fn the_session_executor_serves_the_shell() {
    let project = worktree("shell-executor");
    let state = temp_dir("shell-executor-state");
    let universe = compose(&session_runtime(), &project, &state);
    if !universe.shell_available(ApprovalPolicy::ReadOnly, true) {
        eprintln!("skipping `the_session_executor_serves_the_shell`: no proven sandbox");
        return;
    }
    let shell = universe
        .shell(ApprovalPolicy::Bypass, None, CancellationToken::new())
        .expect("bypass keeps a lane for the shell");
    let database = Arc::new(crate::agent::tools::DatabaseTools::new(None, 100, true));
    let executor = universe.executor(
        Arc::clone(&database),
        &CancellationToken::new(),
        Some(shell),
        None,
        None,
        Default::default(),
    );
    for name in ["shell", "bash"] {
        let ran = executor
            .execute(
                name,
                serde_json::json!({"command": "true", "sandbox": true}),
            )
            .await
            .unwrap_or_else(|error| panic!("the session serves `{name}`: {error:?}"));
        assert_eq!(ran["exit_code"], 0, "{name} ran: {ran}");
        assert_eq!(ran["sandboxed"], true, "{name} ran in the sandbox: {ran}");
    }
    let _ = (fs::remove_dir_all(&project), fs::remove_dir_all(&state));
}
