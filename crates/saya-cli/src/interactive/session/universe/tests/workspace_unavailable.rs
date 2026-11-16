/// No-PATH property 2 — the missing-PATH notice names the fact, the
/// consequence, and the remedy, in the unbound-workspace register.
#[cfg(not(windows))]
#[test]
fn the_missing_path_notice_names_fact_consequence_and_remedy() {
    use crate::interactive::session_host::NO_PATH_NOTICE;
    assert!(
        NO_PATH_NOTICE.contains("No PATH is set"),
        "the notice states the fact: {NO_PATH_NOTICE:?}"
    );
    assert!(
        NO_PATH_NOTICE.contains("run_command is unavailable"),
        "the notice names the consequence: {NO_PATH_NOTICE:?}"
    );
    assert!(
        NO_PATH_NOTICE.contains("set PATH"),
        "the notice names the remedy: {NO_PATH_NOTICE:?}"
    );
}

/// Property 2 — no workspace root, no lane. The tool stays hidden, not
/// advertised: the pin is the no-root shape, and there is no flag that could
/// state a root instead.
#[test]
fn no_root_still_no_lane() {
    let plain = std::env::temp_dir();
    let state = temp_dir("host-no-worktree-state");
    let runtime = session_runtime();
    let launch = crate::interactive::session_host::HostLaunch::for_tests_stated(&runtime);
    let universe = SessionUniverse::compose_with_launch(
        &runtime,
        None,
        None,
        true,
        &plain,
        Some(&launch),
    )
    .expect("composition succeeds without a root");
    assert!(
        universe.host_composed_for_tests().is_none(),
        "no workspace root: the lane does not compose"
    );
    let names = advertised(&universe, AgentMode::Build, ApprovalPolicy::Ask, true);
    assert!(
        !names.contains(&"run_command".to_string()),
        "hidden, not advertised: {names:?}"
    );
    let bypass_names = advertised(&universe, AgentMode::Build, ApprovalPolicy::Bypass, false);
    assert!(
        !bypass_names.contains(&"run_command".to_string()),
        "hidden under bypass too, not advertised: {bypass_names:?}"
    );
    // The unstated shape composes the same way, retaining the live
    // workspace-independent members from the first universe.
    let unstated = SessionUniverse::compose_with_launch_and_path(
        crate::interactive::session::universe::SessionComposition {
            runtime: &runtime,
            explicit: None,
            pinned_root: None,
            walk_when_unpinned: true,
            cwd: &plain,
            launch: None,
            path: std::env::var_os("PATH").map(|value| value.to_string_lossy().into_owned()),
            recovery: None,
        mcp: None,
        },
    )
    .expect("the unstated unbound session composes");
    assert!(
        unstated.host_composed_for_tests().is_none(),
        "unstated: no lane either"
    );
    let _ = fs::remove_dir_all(&state);
}
