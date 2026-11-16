/// A host call under `ask` reaches the session's own approval surface exactly
/// once — the same prompt every other write-shaped call uses — and the
/// surface's answer is the call's answer.
#[tokio::test]
async fn an_ask_host_call_reaches_the_sessions_approval_surface() {
    let project = worktree("shell-ask");
    let state = temp_dir("shell-ask-state");
    let universe = compose(&session_runtime(), &project, &state);
    let definition = crate::agent::tools::shell_policy::definition();
    let host_call = serde_json::json!({"command": "true"});

    let approving = RecordingOperator::new(true);
    let surface: Arc<dyn saya_agent::ApprovalDecider> = approving.clone();
    let shell = universe
        .shell(ApprovalPolicy::Ask, Some(surface), CancellationToken::new())
        .expect("ask keeps a lane for the shell wherever a root binds");
    assert!(
        shell.approve(&definition, &host_call).await,
        "the surface allowed the call"
    );
    assert_eq!(
        approving.calls(),
        1,
        "a host call asks the session's surface exactly once"
    );

    let refusing = RecordingOperator::new(false);
    let surface: Arc<dyn saya_agent::ApprovalDecider> = refusing.clone();
    let shell = universe
        .shell(ApprovalPolicy::Ask, Some(surface), CancellationToken::new())
        .expect("ask keeps a lane for the shell wherever a root binds");
    assert!(
        !shell.approve(&definition, &host_call).await,
        "the surface's refusal is the call's refusal"
    );
    assert_eq!(refusing.calls(), 1);
    let _ = (fs::remove_dir_all(&project), fs::remove_dir_all(&state));
}

/// Under `bypass` the mode is the consent: a host call runs with no prompt at
/// all — the operator is never consulted, even where one is composed.
#[tokio::test]
async fn bypass_runs_a_host_call_without_asking() {
    let project = worktree("shell-bypass");
    let state = temp_dir("shell-bypass-state");
    let universe = compose(&session_runtime(), &project, &state);
    let operator = RecordingOperator::new(true);
    let surface: Arc<dyn saya_agent::ApprovalDecider> = operator.clone();
    let shell = universe
        .shell(
            ApprovalPolicy::Bypass,
            Some(surface),
            CancellationToken::new(),
        )
        .expect("bypass keeps a lane for the shell wherever a root binds");
    assert!(
        shell
            .approve(
                &crate::agent::tools::shell_policy::definition(),
                &serde_json::json!({"command": "true"}),
            )
            .await
    );
    assert_eq!(operator.calls(), 0, "bypass never asks");
    let _ = (fs::remove_dir_all(&project), fs::remove_dir_all(&state));
}

/// A call that names the sandbox runs there without asking: the confinement
/// is the consent, in every mode but `never`. Skipped where the sandbox does
/// not prove — there is no sandbox lane to name.
#[tokio::test]
async fn a_sandbox_call_never_asks() {
    let project = worktree("shell-sandbox");
    let state = temp_dir("shell-sandbox-state");
    let universe = compose(&session_runtime(), &project, &state);
    if !universe.shell_available(ApprovalPolicy::ReadOnly, true) {
        eprintln!("skipping `a_sandbox_call_never_asks`: the sandbox is not proven on this host");
        return;
    }
    let operator = RecordingOperator::new(true);
    let surface: Arc<dyn saya_agent::ApprovalDecider> = operator.clone();
    let shell = universe
        .shell(ApprovalPolicy::Ask, Some(surface), CancellationToken::new())
        .expect("ask keeps a lane for the shell wherever a root binds");
    assert!(
        shell
            .approve(
                &crate::agent::tools::shell_policy::definition(),
                &serde_json::json!({"command": "true", "sandbox": true}),
            )
            .await
    );
    assert_eq!(operator.calls(), 0, "the sandbox lane never asks");
    let _ = (fs::remove_dir_all(&project), fs::remove_dir_all(&state));
}
