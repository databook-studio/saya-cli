/// End to end through a real turn: a `shell` call the model issues is decided
/// by the shell's own lane, with the host lane under `ask` put to this turn's
/// approval surface — the decider the session's surfaces build — and the
/// surface's refusal is the call's refusal. This is the wiring the executor
/// and the approver share: if the two ever disagreed about the lane, the call
/// would run where it was not approved.
#[tokio::test]
async fn a_shell_call_in_a_turn_is_decided_by_the_sessions_approval_surface() {
    use crate::agent::runtime::run_prompt_with_inputs;
    use crate::agent::turn_inputs::TurnInputs;
    use saya_agent::AgentEvent;

    let project = worktree("shell-turn");
    let state = temp_dir("shell-turn-state");
    let runtime = session_runtime();
    let universe = Arc::new(compose(&runtime, &project, &state));
    let operator = RecordingOperator::new(false);
    let decider: Arc<dyn ApprovalDecider> = operator.clone();
    let inputs = TurnInputs {
        ai: runtime.resolved.ai.clone(),
        provider: Box::new(ScriptedShell::new(serde_json::json!({"command": "true"}))),
        registry: crate::connection::ConnectionRegistry::new(""),
        failures: Vec::new(),
    };
    let sink = ShellTurnSink(std::sync::Mutex::new(Vec::new()));
    let output = run_prompt_with_inputs(
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
        AgentMode::Build,
        None,
        None,
    )
    .await;
    output.expect("the turn completes: the refused call was denied, not run");
    assert_eq!(
        operator.calls(),
        1,
        "the host shell call was put to this turn's approval surface exactly once"
    );
    {
        let events = sink.0.lock().unwrap();
        assert!(
            events.iter().any(|event| matches!(
                event,
                AgentEvent::ToolDenied { name, .. } if name.as_str() == "shell"
            )),
            "the surface's refusal denied the shell call at the call boundary: {events:?}"
        );
    }
    let _ = (fs::remove_dir_all(&project), fs::remove_dir_all(&state));
}

/// The same turn, with the call naming the sandbox: the confinement is the
/// consent, so the surface is never asked and the call runs. Skipped where the
/// sandbox does not prove.
#[tokio::test]
async fn a_sandboxed_shell_call_in_a_turn_runs_without_asking() {
    use crate::agent::runtime::run_prompt_with_inputs;
    use crate::agent::turn_inputs::TurnInputs;
    use saya_agent::AgentEvent;

    let project = worktree("shell-turn-sandbox");
    let state = temp_dir("shell-turn-sandbox-state");
    let runtime = session_runtime();
    let universe = Arc::new(compose(&runtime, &project, &state));
    if !universe.shell_available(ApprovalPolicy::ReadOnly, true) {
        eprintln!("skipping `a_sandboxed_shell_call_in_a_turn_runs_without_asking`: no sandbox");
        return;
    }
    let operator = RecordingOperator::new(true);
    let decider: Arc<dyn ApprovalDecider> = operator.clone();
    let inputs = TurnInputs {
        ai: runtime.resolved.ai.clone(),
        provider: Box::new(ScriptedShell::new(
            serde_json::json!({"command": "true", "sandbox": true}),
        )),
        registry: crate::connection::ConnectionRegistry::new(""),
        failures: Vec::new(),
    };
    let sink = ShellTurnSink(std::sync::Mutex::new(Vec::new()));
    let output = run_prompt_with_inputs(
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
        AgentMode::Build,
        None,
        None,
    )
    .await;
    output.expect("the turn completes");
    assert_eq!(
        operator.calls(),
        0,
        "the sandbox lane needs no approval surface"
    );
    {
        let events = sink.0.lock().unwrap();
        assert!(
            events.iter().any(|event| matches!(
                event,
                AgentEvent::ToolCompleted { name, .. } if name.as_str() == "shell"
            )),
            "the sandboxed shell call ran: {events:?}"
        );
    }
    let _ = (fs::remove_dir_all(&project), fs::remove_dir_all(&state));
}
