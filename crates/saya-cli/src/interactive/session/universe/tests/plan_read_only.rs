/// Plan advertises exactly the read-only surface minus `shell`, plus the
/// gate's own `submit_plan`: for a fully composed universe under
/// ask-with-prompt, the Plan definition list equals the list the same universe
/// produces under read-only in Build, with the shell the one difference — and
/// the gate's tool the one addition. Compared as a set — Plan is hiding, not
/// denying: the tools are absent from the list, never offered and refused.
///
/// The shell is the exception because its advertisement is the *mode's*, not
/// the posture's: `read-only` keeps a lane for it (the sandbox, where nothing
/// in the checkout is writable), while Plan denies an external side effect
/// outright, so the shell hides with the rest of the write-shaped surface
/// there — the same rule `plan_hides_write_shaped_tools_under_bypass_too`
/// pins. `submit_plan` is the other way round: it is the closed posture's one
/// door, so it is advertised exactly where the write family is not.
///
/// The read-only lane is the write-denied sandbox, which only a host that can
/// express a denial proves: macOS does, Linux's allow-only Landlock does not,
/// so on Linux both surfaces lack the shell and the set equality still holds.
#[test]
fn plan_advertises_exactly_the_read_only_surface_minus_the_shell() {
    let project = worktree("plan-readonly-surface");
    let state = temp_dir("plan-readonly-surface-state");
    let universe = compose(&session_runtime(), &project, &state);
    let mut plan: std::collections::BTreeSet<String> =
        advertised(&universe, AgentMode::Plan, ApprovalPolicy::Ask, true)
            .into_iter()
            .collect();
    let mut read_only: std::collections::BTreeSet<String> =
        advertised(&universe, AgentMode::Build, ApprovalPolicy::ReadOnly, true)
            .into_iter()
            .collect();
    assert!(
        plan.remove("submit_plan"),
        "the closed plan posture advertises its one door: {plan:?}"
    );
    if read_only_sandbox_proves(&project) {
        assert!(
            read_only.remove("shell"),
            "read-only keeps a lane for the shell: {read_only:?}"
        );
    }
    assert_eq!(
        plan, read_only,
        "Plan hides write-shaped tools instead of offering-then-denying them, \
         so its Ask surface equals the read-only surface minus the shell and \
         plus the gate: plan={plan:?} read-only={read_only:?}"
    );
    assert!(
        !plan.contains("shell"),
        "Plan denies an external side effect, so the shell hides there: {plan:?}"
    );
    let _ = (fs::remove_dir_all(&project), fs::remove_dir_all(&state));
}

/// Plan hides under bypass too: bypass-with-Plan advertises no write-shaped
/// tool — the case a reader will doubt, because Build-with-bypass advertises
/// them all.
#[test]
fn plan_hides_write_shaped_tools_under_bypass_too() {
    let project = worktree("plan-bypass");
    let state = temp_dir("plan-bypass-state");
    let universe = compose(&session_runtime(), &project, &state);
    let names = advertised(&universe, AgentMode::Plan, ApprovalPolicy::Bypass, false);
    for tool in PLAN_HIDDEN_TOOLS {
        assert!(
            !names.contains(&tool.to_string()),
            "Plan hides {tool} even under bypass (hiding, not denying): {names:?}"
        );
    }
    // The positive control: Build-with-bypass advertises the members the
    // plain composition carries, the shell among them.
    let build = advertised(&universe, AgentMode::Build, ApprovalPolicy::Bypass, false);
    for tool in [
        "write",
        "edit",
        "http_fetch",
        "http_download",
        "shell",
    ] {
        assert!(
            build.contains(&tool.to_string()),
            "Build-with-bypass still advertises {tool}: {build:?}"
        );
    }
    let _ = (fs::remove_dir_all(&project), fs::remove_dir_all(&state));
}
