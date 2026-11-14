// The session's background shell family: the same start/read/kill the
// coding root advertises, owned by the session rather than by the turn that
// started it. A detached command outlives the turn's composed shell and dies
// with the session; the two readers are advertised only after a run exists.
//
// Every case here takes the host lane under `bypass` — the lane needs no
// sandbox proof — and uses only commands `/bin/sh` runs on macOS.

fn session_executor(
    universe: &SessionUniverse,
    shell: Arc<crate::agent::tools::shell_policy::ShellTool>,
) -> Arc<dyn ToolExecutor> {
    let database = Arc::new(crate::agent::tools::DatabaseTools::new(None, 100, true));
    universe.executor(
        database,
        &CancellationToken::new(),
        Some(shell),
        None,
        None,
        Default::default(),
    )
}

/// A background start made through the session's executor answers with an id,
/// the read reaches the run, and the kill ends one that is still going. The
/// readers are unknown tools before the first start and advertised after it.
#[tokio::test]
async fn a_session_background_run_is_started_read_and_killed() {
    let project = worktree("shell-background");
    let state = temp_dir("shell-background-state");
    let universe = compose(&session_runtime(), &project, &state);
    let before = advertised(&universe, AgentMode::Build, ApprovalPolicy::Bypass, false);
    assert!(
        before.contains(&"shell".to_string()),
        "the start is advertised with the shell: {before:?}"
    );
    for name in ["shell_output", "shell_kill"] {
        assert!(
            !before.contains(&name.to_string()),
            "{name} is not advertised before a background run exists: {before:?}"
        );
    }

    let shell = universe
        .shell(ApprovalPolicy::Bypass, None, CancellationToken::new())
        .expect("bypass keeps a lane for the shell wherever a root binds");
    let executor = session_executor(&universe, shell);
    for name in ["shell_output", "shell_kill"] {
        assert!(
            matches!(
                executor.execute(name, serde_json::json!({"id": 1})).await,
                Err(saya_agent::ToolError::UnsupportedTool)
            ),
            "a call to {name} before a run exists is an unknown tool"
        );
    }

    let started = executor
        .execute(
            "shell",
            serde_json::json!({"command": "sleep 5; echo late", "run_in_background": true}),
        )
        .await
        .expect("a background start runs");
    let id = started["id"]
        .as_u64()
        .expect("the start answers with an id");
    assert_eq!(started["status"], "running", "the start does not wait");

    let after = advertised(&universe, AgentMode::Build, ApprovalPolicy::Bypass, false);
    for name in ["shell_output", "shell_kill"] {
        assert!(
            after.contains(&name.to_string()),
            "{name} joins the list once a run exists: {after:?}"
        );
    }

    let read = {
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(20);
        loop {
            let read = executor
                .execute("shell_output", serde_json::json!({"id": id}))
                .await
                .expect("the read reaches the run");
            if read["status"] != "running" {
                break read;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "the run did not end: {read}"
            );
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
    };
    assert_eq!(read["status"], "exited");
    assert_eq!(read["exit_code"], 0);
    assert_eq!(read["stdout"], "late\n");

    let second = executor
        .execute(
            "shell",
            serde_json::json!({"command": "sleep 30", "run_in_background": true}),
        )
        .await
        .expect("a second background start runs");
    let second_id = second["id"].as_u64().expect("an id");
    let killed = executor
        .execute("shell_kill", serde_json::json!({"id": second_id}))
        .await
        .expect("the kill reaches the run");
    assert_eq!(killed["status"], "killed");

    let _ = (fs::remove_dir_all(&project), fs::remove_dir_all(&state));
}

/// A run started by one turn's shell is reachable from the next turn's: the
/// session owns the registry, not the turn, so a detached command survives the
/// shell that started it.
#[tokio::test]
async fn a_session_background_run_outlives_the_turn_that_started_it() {
    let project = worktree("shell-background-turn");
    let state = temp_dir("shell-background-turn-state");
    let universe = compose(&session_runtime(), &project, &state);

    let first = universe
        .shell(ApprovalPolicy::Bypass, None, CancellationToken::new())
        .expect("bypass keeps a lane for the shell wherever a root binds");
    let started = first
        .call(
            "shell",
            &serde_json::json!({"command": "sleep 5; echo late", "run_in_background": true}),
        )
        .await
        .expect("the turn's shell starts the run");
    let id = started["id"]
        .as_u64()
        .expect("the start answers with an id");
    // The turn ends: its composed shell goes away, and the next turn composes
    // its own.
    drop(first);

    let second = universe
        .shell(ApprovalPolicy::Bypass, None, CancellationToken::new())
        .expect("the next turn composes a shell");
    let read = {
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(20);
        loop {
            let read = second
                .call("shell_output", &serde_json::json!({"id": id}))
                .await
                .expect("the next turn reaches the run the session still holds");
            if read["status"] != "running" {
                break read;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "the run did not end: {read}"
            );
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
    };
    assert_eq!(read["status"], "exited");
    assert_eq!(read["stdout"], "late\n");

    let _ = (fs::remove_dir_all(&project), fs::remove_dir_all(&state));
}

/// `plan` hides the shell with the rest of the write-shaped surface, so a
/// background start is refused like any other shell call — the loop never
/// reaches the tool, and no run is recorded for a later turn to read.
#[tokio::test]
async fn plan_mode_refuses_a_background_start() {
    use crate::agent::runtime::run_prompt_with_inputs;
    use crate::agent::turn_inputs::TurnInputs;
    use saya_agent::AgentEvent;

    let project = worktree("shell-background-plan");
    let state = temp_dir("shell-background-plan-state");
    let runtime = session_runtime();
    let universe = Arc::new(compose(&runtime, &project, &state));
    let operator = RecordingOperator::new(true);
    let decider: Arc<dyn ApprovalDecider> = operator.clone();
    let inputs = TurnInputs {
        ai: runtime.resolved.ai.clone(),
        provider: Box::new(ScriptedShell::new(serde_json::json!({
            "command": "sleep 5; echo late",
            "run_in_background": true,
        }))),
        registry: crate::connection::ConnectionRegistry::new(""),
        failures: Vec::new(),
    };
    let sink = ShellTurnSink(std::sync::Mutex::new(Vec::new()));
    run_prompt_with_inputs(
        &runtime,
        inputs,
        "run it",
        ApprovalPolicy::Ask,
        false,
        true,
        Vec::new(),
        &sink,
        CancellationToken::new(),
        None,
        Some(decider),
        None,
        Some(Arc::clone(&universe)),
        AgentMode::Plan,
        None,
        None,
    )
    .await
    .expect("the turn completes: the shell call was refused, not run");
    assert_eq!(
        operator.calls(),
        0,
        "plan refuses the shell before any approval surface is consulted"
    );
    {
        let events = sink.0.lock().unwrap();
        assert!(
            events.iter().any(|event| matches!(
                event,
                AgentEvent::ToolCompleted { name, .. } if name.as_str() == "shell"
            )),
            "the shell call was answered as a refusal: {events:?}"
        );
    }
    let after = advertised(&universe, AgentMode::Build, ApprovalPolicy::Bypass, false);
    for name in ["shell_output", "shell_kill"] {
        assert!(
            !after.contains(&name.to_string()),
            "no run was recorded by a refused plan-mode start: {after:?}"
        );
    }
    let _ = (fs::remove_dir_all(&project), fs::remove_dir_all(&state));
}
