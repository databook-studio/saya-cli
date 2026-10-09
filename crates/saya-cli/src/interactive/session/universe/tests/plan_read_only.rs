/// Plan advertises exactly the read-only surface: for a fully composed
/// universe under ask-with-prompt, the Plan definition list equals the list
/// the same universe produces under read-only in Build. Compared as a set —
/// Plan is hiding, not denying: the tools are absent from the list, never
/// offered and refused.
#[test]
fn plan_advertises_exactly_the_read_only_surface() {
    let project = worktree("plan-readonly-surface");
    let state = temp_dir("plan-readonly-surface-state");
    let universe = compose(&session_runtime(None), &project, &state);
    let plan: std::collections::BTreeSet<String> =
        advertised(&universe, AgentMode::Plan, ApprovalPolicy::Ask, true)
            .into_iter()
            .collect();
    let read_only: std::collections::BTreeSet<String> =
        advertised(&universe, AgentMode::Build, ApprovalPolicy::ReadOnly, true)
            .into_iter()
            .collect();
    assert_eq!(
        plan, read_only,
        "Plan hides write-shaped tools instead of offering-then-denying them, \
         so its Ask surface equals the read-only surface: plan={plan:?} read-only={read_only:?}"
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
    let universe = compose(&session_runtime(None), &project, &state);
    let names = advertised(&universe, AgentMode::Plan, ApprovalPolicy::Bypass, false);
    for tool in PLAN_HIDDEN_TOOLS {
        assert!(
            !names.contains(&tool.to_string()),
            "Plan hides {tool} even under bypass (hiding, not denying): {names:?}"
        );
    }
    // The positive control: Build-with-bypass advertises the four the plain
    // composition carries.
    let build = advertised(&universe, AgentMode::Build, ApprovalPolicy::Bypass, false);
    for tool in [
        "workspace_write",
        "workspace_edit",
        "scratch_sql",
        "http_fetch",
        "http_download",
    ] {
        assert!(
            build.contains(&tool.to_string()),
            "Build-with-bypass still advertises {tool}: {build:?}"
        );
    }
    let _ = (fs::remove_dir_all(&project), fs::remove_dir_all(&state));
}
