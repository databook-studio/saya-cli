/// The lane's advertisement follows the mode rule — read-only and never
/// sessions never see the tool — and a composed lane advertises under ask
/// (with a prompt) and under bypass. The universe helper composes the lane
/// here through the stated-launch helper below.
#[cfg(not(windows))]
#[test]
fn the_lane_s_advertisement_follows_the_mode_rule() {
    let project = worktree("host-advertise");
    let state = temp_dir("host-advertise-state");
    let runtime = session_runtime(None);
    let launch = crate::interactive::session_host::HostLaunch::for_tests_stated(&runtime);
    let universe = SessionUniverse::compose_with_launch(
        &runtime,
        None,
        None,
        true,
        &project,
        &state,
        Some(&launch),
    )
    .expect("composition succeeds on a plain worktree");
    for (mode, can_obtain_approval, label) in [
        (ApprovalPolicy::ReadOnly, true, "read-only"),
        (ApprovalPolicy::Never, true, "never"),
        (ApprovalPolicy::Ask, false, "no prompt surface"),
    ] {
        let names = advertised(&universe, AgentMode::Build, mode, can_obtain_approval);
        assert!(
            !names.contains(&"run_command".to_string()),
            "{label} never sees the tool: {names:?}"
        );
    }
    let ask_names = advertised(&universe, AgentMode::Build, ApprovalPolicy::Ask, true);
    assert!(
        ask_names.contains(&"run_command".to_string()),
        "a composed lane advertises under ask with a prompt: {ask_names:?}"
    );
    let bypass_names = advertised(&universe, AgentMode::Build, ApprovalPolicy::Bypass, false);
    assert!(
        bypass_names.contains(&"run_command".to_string()),
        "a composed lane advertises under bypass: {bypass_names:?}"
    );
    let _ = (fs::remove_dir_all(&project), fs::remove_dir_all(&state));
}

/// No-PATH property 1 — a session without PATH still starts: no host
/// lane, and everything else works (SQL, schema, workspace file tools,
/// approvals).
#[test]
fn a_session_without_path_still_starts() {
    let project = worktree("no-path-starts");
    let state = temp_dir("no-path-starts-state");
    let runtime = session_runtime(None);
    // The full composer with the PATH seam explicit: `None` is the no-PATH
    // environment (no process-environment mutation — the seam carries it).
    let universe = SessionUniverse::compose_with_launch_and_path(
        crate::interactive::session::universe::SessionComposition {
            runtime: &runtime,
            explicit: None,
            pinned_root: None,
            walk_when_unpinned: true,
            cwd: &project,
            state_dir: &state,
            launch: None,
            path: None,
            scratch: None,
            recovery: None,
        },
    )
    .expect("a session without PATH still starts");
    assert!(
        universe.host_composed_for_tests().is_none(),
        "no PATH: the lane composes nothing"
    );
    assert!(
        universe.root().is_some(),
        "the root still binds without PATH"
    );
    let notice = universe
        .notice
        .as_deref()
        .expect("no PATH says so at startup");
    if cfg!(windows) {
        assert!(notice.contains("unavailable on Windows"), "{notice:?}");
    } else {
        assert!(
            notice.contains("No PATH is set"),
            "the notice states the fact: {notice:?}"
        );
    }
    // Everything else works: the session's own surface is intact — SQL,
    // schema reads, workspace file tools, and approvals.
    let ask_names = advertised(&universe, AgentMode::Build, ApprovalPolicy::Ask, true);
    for tool in [
        "scratch_sql",
        "scratch_import",
        "http_fetch",
        "workspace_write",
        "http_download",
    ] {
        assert!(
            ask_names.contains(&tool.to_string()),
            "no PATH keeps {tool}: {ask_names:?}"
        );
    }
    assert!(
        !ask_names.contains(&"run_command".to_string()),
        "only the lane is gone: {ask_names:?}"
    );
    assert!(
        universe.workspace().is_some(),
        "workspace file tools still compose without PATH"
    );
    assert!(
        universe.approval_facts(&runtime).host.is_none(),
        "the host lane contributes no approval facts without PATH"
    );
    // Deny still composes: the lane-blind list gates every session's own
    // doors even with the lane off.
    let launch =
        crate::interactive::session_host::HostLaunch::from_deny_for_tests(vec!["curl".to_owned()]);
    let denied = SessionUniverse::compose_with_launch_and_path(
        crate::interactive::session::universe::SessionComposition {
            runtime: &runtime,
            explicit: None,
            pinned_root: None,
            walk_when_unpinned: true,
            cwd: &project,
            state_dir: &state,
            launch: Some(&launch),
            path: None,
            scratch: universe.scratch(),
            recovery: None,
        },
    )
    .expect("deny composes without PATH too");
    assert!(
        denied.deny_programs().contains(&"curl".to_string()),
        "the deny list rides the no-PATH session: {:?}",
        denied.deny_programs()
    );
    let _ = (fs::remove_dir_all(&project), fs::remove_dir_all(&state));
}

/// Windows keeps the session available while the Unix-only host runner is
/// deliberately absent. The unavailable lane must contribute neither a tool
/// nor approval facts, including under bypass where it would otherwise run.
#[cfg(windows)]
#[test]
fn a_windows_session_starts_with_the_host_lane_off() {
    let project = worktree("windows-host-off");
    let state = temp_dir("windows-host-off-state");
    let runtime = session_runtime(None);
    let universe = SessionUniverse::compose_with_launch_and_path(
        crate::interactive::session::universe::SessionComposition {
            runtime: &runtime,
            explicit: None,
            pinned_root: None,
            walk_when_unpinned: true,
            cwd: &project,
            state_dir: &state,
            launch: None,
            path: Some("C:\\Windows\\System32".to_owned()),
            scratch: None,
            recovery: None,
        },
    )
    .expect("Windows starts the session with host commands unavailable");

    assert!(universe.root().is_some(), "the workspace still binds");
    assert!(
        universe.host_composed_for_tests().is_none(),
        "host lane is off"
    );
    for (mode, can_obtain_approval, label) in [
        (ApprovalPolicy::Ask, true, "ask"),
        (ApprovalPolicy::Bypass, false, "bypass"),
    ] {
        let names = advertised(&universe, AgentMode::Build, mode, can_obtain_approval);
        assert!(
            !names.contains(&"run_command".to_string()),
            "Windows {label} does not advertise an unavailable tool: {names:?}"
        );
    }
    assert!(
        universe.approval_facts(&runtime).host.is_none(),
        "an unavailable host lane has no approval facts"
    );
    let notice = universe
        .notice
        .as_deref()
        .expect("startup names the unavailable lane");
    assert!(notice.contains("unavailable on Windows"), "{notice:?}");
    assert!(notice.contains("run_command is unavailable"), "{notice:?}");
    assert!(matches!(
        crate::interactive::session_host::compose_host(
            &crate::interactive::session_host::HostLaunch::unstated(&runtime),
            Some(&project),
            "C:\\Windows\\System32".to_owned(),
        ),
        Err(error) if error.contains("unavailable on Windows")
    ));
    let _ = (fs::remove_dir_all(&project), fs::remove_dir_all(&state));
}
