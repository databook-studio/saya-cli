/// The lane's advertisement follows the mode rule — read-only and never
/// sessions never see the tool — and a composed lane advertises under ask
/// (with a prompt) and under bypass. The universe helper composes the lane
/// here through the stated-launch helper below.
#[test]
fn the_lane_s_advertisement_follows_the_mode_rule() {
    let project = worktree("host-advertise");
    let state = temp_dir("host-advertise-state");
    let runtime = session_runtime();
    let launch = crate::interactive::session_host::HostLaunch::for_tests_stated(&runtime);
    let universe = SessionUniverse::compose_with_launch(
        &runtime,
        None,
        None,
        true,
        &project,
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
/// approvals). Unix-only: a Windows process always has a system-defined
/// search path, so the same seam composes the lane there instead (see
/// `a_windows_session_without_path_uses_the_system_search_path`).
#[cfg(not(windows))]
#[test]
fn a_session_without_path_still_starts() {
    let project = worktree("no-path-starts");
    let state = temp_dir("no-path-starts-state");
    let runtime = session_runtime();
    // The full composer with the PATH seam explicit: `None` is the no-PATH
    // environment (no process-environment mutation — the seam carries it).
    let universe = SessionUniverse::compose_with_launch_and_path(
        crate::interactive::session::universe::SessionComposition {
            runtime: &runtime,
            explicit: None,
            pinned_root: None,
            walk_when_unpinned: true,
            cwd: &project,
            launch: None,
            path: None,
            recovery: None,
        mcp: None,
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
    assert!(
        notice.contains("No PATH is set"),
        "the notice states the fact: {notice:?}"
    );
    // Everything else works: the session's own surface is intact — SQL,
    // schema reads, workspace file tools, and approvals.
    let ask_names = advertised(&universe, AgentMode::Build, ApprovalPolicy::Ask, true);
    for tool in [
        "http_fetch",
        "write",
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
            launch: Some(&launch),
            path: None,
            recovery: None,
        mcp: None,
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

/// Windows composes the same host lane Unix does. The lane is unsandboxed
/// there — no OS confinement is claimed — but it is composed, advertised
/// under the same mode rule, and contributes the same approval facts.
#[cfg(windows)]
#[test]
fn a_windows_session_composes_the_unsandboxed_host_lane() {
    let project = worktree("windows-host-on");
    let state = temp_dir("windows-host-on-state");
    let runtime = session_runtime();
    let universe = SessionUniverse::compose_with_launch_and_path(
        crate::interactive::session::universe::SessionComposition {
            runtime: &runtime,
            explicit: None,
            pinned_root: None,
            walk_when_unpinned: true,
            cwd: &project,
            launch: None,
            path: Some("C:\\Windows\\System32".to_owned()),
            recovery: None,
            mcp: None,
        },
    )
    .expect("Windows composes the host lane");

    assert!(universe.root().is_some(), "the workspace binds");
    assert!(
        universe.host_composed_for_tests().is_some(),
        "the host lane composes on Windows"
    );
    for (mode, can_obtain_approval, label) in [
        (ApprovalPolicy::Ask, true, "ask"),
        (ApprovalPolicy::Bypass, false, "bypass"),
    ] {
        let names = advertised(&universe, AgentMode::Build, mode, can_obtain_approval);
        assert!(
            names.contains(&"run_command".to_string()),
            "Windows {label} advertises the composed lane: {names:?}"
        );
    }
    for (mode, label) in [
        (ApprovalPolicy::ReadOnly, "read-only"),
        (ApprovalPolicy::Never, "never"),
    ] {
        let names = advertised(&universe, AgentMode::Build, mode, true);
        assert!(
            !names.contains(&"run_command".to_string()),
            "Windows {label} never advertises the lane: {names:?}"
        );
    }
    assert!(
        universe.approval_facts(&runtime).host.is_some(),
        "a composed host lane contributes its approval facts"
    );
    let notice = universe.notice.as_deref().unwrap_or_default();
    assert!(
        !notice.contains("unavailable on Windows") && !notice.contains("lane is not composed"),
        "a composed lane rides no unavailable notice: {notice:?}"
    );
    assert!(
        matches!(
            crate::interactive::session_host::compose_host(
                &crate::interactive::session_host::HostLaunch::unstated(&runtime),
                Some(&project),
                "C:\\Windows\\System32".to_owned(),
            ),
            Ok(Some(_))
        ),
        "the unstated launch composes on Windows"
    );
    let _ = (fs::remove_dir_all(&project), fs::remove_dir_all(&state));
}

/// The no-PATH seam on Windows: the platform always has a system-defined
/// search path, so the same composition that drops the lane on Unix composes
/// it here over the process and system directories. The session still binds
/// its root, the lane advertises under the mode rule, and the deny list
/// still rides the session.
#[cfg(windows)]
#[test]
fn a_windows_session_without_path_uses_the_system_search_path() {
    let project = worktree("windows-no-path");
    let state = temp_dir("windows-no-path-state");
    let runtime = session_runtime();
    let universe = SessionUniverse::compose_with_launch_and_path(
        crate::interactive::session::universe::SessionComposition {
            runtime: &runtime,
            explicit: None,
            pinned_root: None,
            walk_when_unpinned: true,
            cwd: &project,
            launch: None,
            path: None,
            recovery: None,
            mcp: None,
        },
    )
    .expect("a session without PATH still starts");
    assert!(
        universe.root().is_some(),
        "the root still binds without PATH"
    );
    assert!(
        universe.host_composed_for_tests().is_some(),
        "Windows composes the lane over its system search path"
    );
    let notice = universe.notice.as_deref().unwrap_or_default();
    assert!(
        !notice.contains("No PATH is set") && !notice.contains("unavailable on Windows"),
        "a composed lane rides no no-PATH notice: {notice:?}"
    );
    let ask_names = advertised(&universe, AgentMode::Build, ApprovalPolicy::Ask, true);
    for tool in ["run_command", "http_fetch", "write", "http_download"] {
        assert!(
            ask_names.contains(&tool.to_string()),
            "no PATH keeps {tool} on Windows: {ask_names:?}"
        );
    }
    assert!(
        universe.workspace().is_some(),
        "workspace file tools still compose without PATH"
    );
    assert!(
        universe.approval_facts(&runtime).host.is_some(),
        "the lane contributes approval facts without PATH"
    );
    let launch =
        crate::interactive::session_host::HostLaunch::from_deny_for_tests(vec!["curl".to_owned()]);
    let denied = SessionUniverse::compose_with_launch_and_path(
        crate::interactive::session::universe::SessionComposition {
            runtime: &runtime,
            explicit: None,
            pinned_root: None,
            walk_when_unpinned: true,
            cwd: &project,
            launch: Some(&launch),
            path: None,
            recovery: None,
            mcp: None,
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
