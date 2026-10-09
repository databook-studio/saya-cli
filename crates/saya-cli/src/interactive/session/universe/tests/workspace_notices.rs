/// Slice 1 (G1 property 1) — a fresh session launched outside any worktree
/// with no `--workspace` binds nothing, and the composition says so at
/// startup: the notice names the fact, why the file tools are absent, and
/// the remedy. Both session loops already print `SessionRuntime::notice`,
/// so carrying it here is what makes an unbound session say so.
#[test]
fn fresh_unbound_composition_carries_the_no_workspace_notice() {
    let plain = temp_dir("unbound-notice");
    let state = temp_dir("unbound-notice-state");
    let universe =
        SessionUniverse::compose(&session_runtime(None), None, None, true, &plain, &state)
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
        notice.contains("git worktree"),
        "the notice names the worktree remedy: {notice:?}"
    );
    let _ = (fs::remove_dir_all(&plain), fs::remove_dir_all(&state));
}

/// Slice 1 (G1 property 2) — a bound session carries no such notice: the
/// worktree case composes silently, byte-identical to today's output.
#[test]
fn a_bound_session_carries_no_such_notice() {
    let project = worktree("bound-silent");
    let state = temp_dir("bound-silent-state");
    let universe =
        SessionUniverse::compose(&session_runtime(None), None, None, true, &project, &state)
            .expect("composition succeeds on a worktree");
    assert!(
        universe.root().is_some(),
        "inside a worktree, the root binds"
    );
    if cfg!(windows) {
        assert!(
            universe
                .notice
                .as_deref()
                .is_some_and(|notice| notice.contains("Host commands are unavailable on Windows")),
            "Windows names the unavailable host lane: {:?}",
            universe.notice
        );
    } else {
        assert!(
            universe.notice.is_none(),
            "a bound session stays silent: {:?}",
            universe.notice
        );
    }
    let _ = (fs::remove_dir_all(&project), fs::remove_dir_all(&state));
}

/// G2 property 1 (slices 3-5) — the unstated lane composes wherever a
/// workspace root binds: the plain `compose` path (no launch statement)
/// over a worktree carries the lane, and advertises `run_command` under
/// ask-with-prompt and under bypass.
#[cfg(not(windows))]
#[test]
fn unstated_lane_composes_where_a_root_binds() {
    let project = worktree("unstated-lane");
    let state = temp_dir("unstated-lane-state");
    let universe =
        SessionUniverse::compose(&session_runtime(None), None, None, true, &project, &state)
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

/// Slice 1, extra pin the list misses — a resumed session whose record
/// predates the workspace keeps its old silence: `walk_when_unpinned` is
/// false there, so the fresh-unbound notice must not fire on a path that
/// was silent by design.
#[test]
fn a_pre_workspace_resume_stays_silent_when_unbound() {
    let plain = temp_dir("resume-silent");
    let state = temp_dir("resume-silent-state");
    let universe =
        SessionUniverse::compose(&session_runtime(None), None, None, false, &plain, &state)
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
