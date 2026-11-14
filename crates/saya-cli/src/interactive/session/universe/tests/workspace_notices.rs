/// Property 1 — a fresh session launched in a dangerous root (the
/// temporary directory) with no `--workspace` binds nothing, and the
/// composition says so at startup: the notice names the fact, why the file
/// tools are absent, and the remedy. Both session loops already print
/// `SessionRuntime::notice`, so carrying it here is what makes an unbound
/// session say so.
#[test]
fn fresh_unbound_composition_carries_the_no_workspace_notice() {
    let plain = std::env::temp_dir();
    let state = temp_dir("unbound-notice-state");
    let universe =
        SessionUniverse::compose(&session_runtime(), None, None, true, &plain)
            .expect("composition succeeds without a root");
    assert!(
        universe.root().is_none(),
        "outside a worktree with no --workspace, nothing binds"
    );
    let notice = universe
        .notice
        .as_deref()
        .expect("an unbound session says so at startup");
    assert!(
        notice.contains("No workspace is bound"),
        "the notice states the fact: {notice:?}"
    );
    assert!(
        notice.contains("file tools are unavailable"),
        "the notice names why the file tools are absent: {notice:?}"
    );
    assert!(
        notice.contains("--workspace <dir>"),
        "the notice names the explicit-bind remedy: {notice:?}"
    );
    assert!(
        notice.contains("project folder"),
        "the notice names the plain-folder remedy: {notice:?}"
    );
    let _ = fs::remove_dir_all(&state);
}

/// Property 2 — a bound session carries no binding notice. The default
/// in-place coding root adds its own one-line fact, and the Windows host
/// lane its refusal; nothing says the workspace is missing.
#[test]
fn a_bound_session_carries_no_binding_notice() {
    let project = worktree("bound-silent");
    let state = temp_dir("bound-silent-state");
    let universe =
        SessionUniverse::compose(&session_runtime(), None, None, true, &project)
            .expect("composition succeeds on a worktree");
    assert!(
        universe.root().is_some(),
        "inside a worktree, the root binds"
    );
    let notice = universe.notice.as_deref().unwrap_or_default();
    assert!(
        !notice.contains("No workspace is bound") && !notice.contains("No Git work tree found"),
        "a bound session names no missing workspace: {:?}",
        universe.notice
    );
    assert!(
        notice.contains("edits land in this folder"),
        "the default in-place coding root says where edits land: {:?}",
        universe.notice
    );
    assert!(
        notice.contains("`/job rewind` undoes tool edits"),
        "the notice names the rewind remedy: {:?}",
        universe.notice
    );
    if cfg!(windows) {
        assert!(
            notice.contains("Host commands are unavailable on Windows"),
            "Windows names the unavailable host lane: {:?}",
            universe.notice
        );
    }
    let _ = (fs::remove_dir_all(&project), fs::remove_dir_all(&state));
}

/// A managed default leaves the worktree untouched, so the session stays
/// silent about in-place edits.
#[test]
fn a_managed_default_carries_no_in_place_notice() {
    let project = worktree("managed-silent");
    let state = temp_dir("managed-silent-state");
    let mut runtime = session_runtime();
    runtime.resolved.coding.root_mode = saya_config::JobRootMode::Managed;
    let universe = SessionUniverse::compose(&runtime, None, None, true, &project)
        .expect("composition succeeds on a worktree");
    assert!(
        universe
            .notice
            .as_deref()
            .is_none_or(|notice| !notice.contains("edits land in this folder")),
        "a managed default says nothing about in-place edits: {:?}",
        universe.notice
    );
    let _ = (fs::remove_dir_all(&project), fs::remove_dir_all(&state));
}

/// Property 1 — the unstated lane composes wherever a
/// workspace root binds: the plain `compose` path (no launch statement)
/// over a worktree carries the lane, and advertises `run_command` under
/// ask-with-prompt and under bypass.
#[cfg(not(windows))]
#[test]
fn unstated_lane_composes_where_a_root_binds() {
    let project = worktree("unstated-lane");
    let state = temp_dir("unstated-lane-state");
    let universe =
        SessionUniverse::compose(&session_runtime(), None, None, true, &project)
            .expect("composition succeeds on a worktree");
    assert!(
        universe.host_composed_for_tests().is_some(),
        "a bound root composes the lane with no statement"
    );
    let ask_names = advertised(&universe, AgentMode::Build, ApprovalPolicy::Ask, true);
    assert!(
        ask_names.contains(&"run_command".to_string()),
        "the unstated lane advertises under ask with a prompt: {ask_names:?}"
    );
    let bypass_names = advertised(&universe, AgentMode::Build, ApprovalPolicy::Bypass, false);
    assert!(
        bypass_names.contains(&"run_command".to_string()),
        "the unstated lane advertises under bypass: {bypass_names:?}"
    );
    let _ = (fs::remove_dir_all(&project), fs::remove_dir_all(&state));
}

/// An extra pin the list misses — a resumed session whose record predates the
/// workspace root stays silent: `walk_when_unpinned` is false there, so the
/// fresh-unbound notice must not fire on a path that is silent by design.
#[test]
fn a_pre_workspace_resume_stays_silent_when_unbound() {
    let plain = temp_dir("resume-silent");
    let state = temp_dir("resume-silent-state");
    let universe =
        SessionUniverse::compose(&session_runtime(), None, None, false, &plain)
            .expect("composition succeeds without a root");
    assert!(
        universe.root().is_none(),
        "no pin and no walk: nothing binds"
    );
    assert!(
        universe.notice.is_none(),
        "a pre-workspace resume keeps its old silence: {:?}",
        universe.notice
    );
    let _ = (fs::remove_dir_all(&plain), fs::remove_dir_all(&state));
}
