#[cfg(any(target_os = "macos", target_os = "linux"))]
use saya_agent::{ApprovalDecider, ToolExecutor};

/// The run's command door is the `shell` tool — the same tool the coding
/// root advertises, composed over the run's workspace — and never
/// `run_program`: every step's universe carries `shell`, and the step's
/// composite dispatches the name to the shell member rather than the
/// database fall-through, which refuses unknown names.
#[cfg(any(target_os = "macos", target_os = "linux"))]
#[tokio::test]
async fn every_step_carries_shell_and_never_run_program() {
    let database = Arc::new(DatabaseTools::new(None, 100, true));
    let ws = workspace("shell-member");
    let Some(shell) = run_shell_over(&ws) else {
        return;
    };
    let built = toolsets(
        ToolsetInputs {
            database: &database,
            fetch: None,
            shell: Some(&shell),
            workspace: &ws,
            allow_query_data: true,
            tools: &saya_config::ResolvedTools::default(),
            mcp: None,
            hooks: None,
        },
        &[step("run the tests", Capabilities::default())],
    );
    let names = names(&built, 0);
    assert!(
        names.contains(&"shell".to_string()),
        "every step's universe carries the shell: {names:?}"
    );
    assert!(
        !names.contains(&"run_program".to_string()),
        "the runner door is gone from every step: {names:?}"
    );

    // The composite dispatches the name to the shell member: a sandboxed
    // call runs (the confinement is the consent) instead of refusing as an
    // unknown tool the way the database fall-through would. Where the host
    // proves no sandbox, the member still serves the name and refuses the
    // unapprovable host call with its documented reason.
    let outcome = built[0]
        .executor
        .execute(
            "shell",
            serde_json::json!({"command": "true", "sandbox": true}),
        )
        .await;
    if crate::agent::tools::shell_policy::prove(ws.root(), saya_agent::ApprovalPolicy::Ask)
        .is_some()
    {
        let ran = outcome.expect("the shell member serves the call");
        assert_eq!(ran["exit_code"], 0, "{ran}");
        assert_eq!(ran["sandboxed"], true, "{ran}");
    } else {
        let error = outcome.expect_err("a host-only shell cannot honour `sandbox: true`");
        assert!(
            error.to_string().contains("no operator is available"),
            "the shell member refuses the unapprovable host call: {error}"
        );
    }
}

/// Headless Ask: the run has no shell operator, so a host call is refused
/// with the reason the refusal names, while a call that names the sandbox
/// runs there unasked — the confinement is the consent, exactly as the
/// coding root's shell decides it.
#[cfg(any(target_os = "macos", target_os = "linux"))]
#[tokio::test]
async fn a_headless_host_call_is_refused_and_a_sandboxed_call_runs() {
    // No proven sandbox: the host lane is all this host has, and with no
    // operator to answer a host call the call is refused.
    let ws = workspace("shell-headless-refused");
    let Some(host_only) = run_shell_host_only(&ws) else {
        return;
    };
    let definition = crate::agent::tools::shell_policy::definition();
    assert!(!host_only.approve(&definition, &call("true")).await);
    assert_eq!(
        host_only
            .refusal_detail(&definition, &call("true"))
            .as_deref(),
        Some(crate::agent::tools::shell_policy::HOST_APPROVAL_UNAVAILABLE)
    );

    // A proven sandbox: the sandbox lane needs nobody's answer. Without one,
    // the same call is a host call with no operator, and the documented
    // refusal stands — naming the sandbox cannot conjure a lane.
    let ws = workspace("shell-headless-sandboxed");
    let Some(shell) = run_shell_over(&ws) else {
        return;
    };
    if crate::agent::tools::shell_policy::prove(ws.root(), saya_agent::ApprovalPolicy::Ask)
        .is_some()
    {
        assert!(shell.approve(&definition, &sandbox_call("true")).await);
        let ran = shell
            .execute("shell", sandbox_call("true"))
            .await
            .expect("the sandboxed call runs");
        assert_eq!(ran["sandboxed"], true, "{ran}");
    } else {
        assert!(!shell.approve(&definition, &sandbox_call("true")).await);
        assert_eq!(
            shell
                .refusal_detail(&definition, &sandbox_call("true"))
                .as_deref(),
            Some(crate::agent::tools::shell_policy::HOST_APPROVAL_UNAVAILABLE)
        );
    }
}

/// Headless Ask with a trusted allow rule: the stated prefix is the answer,
/// so the host call runs without an operator. The same call without the rule
/// is refused by the test above — the rule is the only difference.
#[cfg(any(target_os = "macos", target_os = "linux"))]
#[tokio::test]
async fn a_headless_host_call_under_an_allowed_prefix_runs_on_the_host() {
    let ws = workspace("shell-headless-allowed");
    let rules = saya_config::CommandRules {
        allow: vec![
            saya_config::CommandRule::parse("true").expect("the test rule parses"),
        ],
        deny: Vec::new(),
        diagnostics: Vec::new(),
    };
    let Some(shell) = run_shell_host_only_with_rules(&ws, rules.clone()) else {
        return;
    };
    let definition = crate::agent::tools::shell_policy::definition();
    assert!(
        shell.approve(&definition, &call("true")).await,
        "the allow rule answers the host call"
    );
    let ran = shell
        .execute("shell", call("true"))
        .await
        .expect("the rule-allowed host call runs");
    assert_eq!(ran["exit_code"], 0, "{ran}");
    assert_eq!(ran["sandboxed"], false, "the rule reaches the host: {ran}");

    // A prefix is not a wildcard: a different command is still refused.
    assert!(
        !shell.approve(&definition, &call("false")).await,
        "an uncovered command still needs an operator"
    );
}

/// The run's shell over a private workspace, with the sandbox lane when
/// this host proves one — the `assembly` composition for a headless run
/// (no operator: a run has no shell operator to answer a host call).
#[cfg(any(target_os = "macos", target_os = "linux"))]
fn run_shell_over(
    workspace: &Arc<Workspace>,
) -> Option<Arc<crate::agent::tools::shell_policy::ShellTool>> {
    run_shell(
        workspace,
        saya_agent::ApprovalPolicy::Ask,
        None,
        saya_harness::runner::HostPolicy::default(),
        cancellation(),
        saya_config::CommandRules::default(),
    )
}

/// The run's shell over the host lane alone, as a host with no proven
/// sandbox composes one.
#[cfg(any(target_os = "macos", target_os = "linux"))]
fn run_shell_host_only(
    workspace: &Arc<Workspace>,
) -> Option<Arc<crate::agent::tools::shell_policy::ShellTool>> {
    run_shell_host_only_with_rules(workspace, saya_config::CommandRules::default())
}

/// The same host-only composition with the trusted command rules stated.
#[cfg(any(target_os = "macos", target_os = "linux"))]
fn run_shell_host_only_with_rules(
    workspace: &Arc<Workspace>,
    rules: saya_config::CommandRules,
) -> Option<Arc<crate::agent::tools::shell_policy::ShellTool>> {
    crate::agent::tools::shell_policy::compose_host_only(
        workspace.root(),
        saya_agent::ApprovalPolicy::Ask,
        None,
        saya_harness::runner::HostPolicy::default(),
        cancellation(),
        // A run's shell lives for one step, so it owns no background run.
        crate::agent::tools::shell_policy::Background::Off,
        None,
        rules,
    )
    .map(Arc::new)
}

/// A call that names no lane: `sandbox` is omitted, so the schema default —
/// the host lane — applies.
#[cfg(any(target_os = "macos", target_os = "linux"))]
fn call(command: &str) -> serde_json::Value {
    serde_json::json!({ "command": command, "description": "a probe" })
}

/// A call that names the sandbox lane explicitly.
#[cfg(any(target_os = "macos", target_os = "linux"))]
fn sandbox_call(command: &str) -> serde_json::Value {
    serde_json::json!({ "command": command, "description": "a probe", "sandbox": true })
}
