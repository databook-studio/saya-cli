/// Build is pinned: for each of the four approval policies the Build
/// definition list is the chart-less database surface — `render_chart` is
/// advertised nowhere — plus `checklist`, the
/// session-metadata write every policy but `never` carries, and `shell`, the
/// shared command door every mode but `never` keeps a lane for. Pinned
/// by name on the plain worktree composition (the unstated host lane
/// composes, so `run_command` rides).
///
/// The flag is the approval-surface flag: `ask` with an approval surface
/// advertises the write-shaped set; `bypass` needs no surface; `read-only`
/// and `never` hide whatever the surface. The `(Ask, false)` row — a
/// surface that can obtain no approval — keeps the anti-pattern pinned; the
/// `shell` rides there because its sandbox lane is advertised exactly where
/// that lane exists, and needs no approval surface to be usable. Under
/// `read-only` that lane is the write-denied sandbox, which only a host that
/// can express a denial proves: macOS does, Linux's allow-only Landlock does
/// not, so the `shell` row follows this host's own answer.
#[cfg(not(windows))]
#[test]
fn build_advertisement_is_pinned_for_every_approval_policy() {
    let project = worktree("build-pinned");
    let state = temp_dir("build-pinned-state");
    let universe = compose(&session_runtime(), &project, &state);
    let mut read_only_names = vec![
        "schema_discovery",
        "request_clarification",
        "read",
        "list",
        "glob",
        "grep",
        "bounded_sql_query",
        "bounded_sql_query_all",
        "result_shape",
        "column_health",
        "join_check",
        "designate_answer",
        "checklist",
    ];
    if read_only_sandbox_proves(&project) {
        read_only_names.push("shell");
    }
    // A headless `ask` session keeps the sandbox lane only where the host
    // proves it; without one there is no lane to advertise, so the shell is
    // absent rather than advertised-and-always-refused.
    let mut ask_headless_names = vec![
        "schema_discovery",
        "request_clarification",
        "read",
        "list",
        "glob",
        "grep",
        "bounded_sql_query",
        "bounded_sql_query_all",
        "result_shape",
        "column_health",
        "join_check",
        "designate_answer",
        "checklist",
    ];
    if ask_sandbox_proves(&project) {
        ask_headless_names.push("shell");
    }
    for (mode, can_obtain_approval, names) in [
        (ApprovalPolicy::Ask, false, ask_headless_names),
        (
            ApprovalPolicy::Ask,
            true,
            vec![
                "schema_discovery",
                "request_clarification",
                "read",
                "list",
                "glob",
                "grep",
                "bounded_sql_query",
                "bounded_sql_query_all",
                "result_shape",
                "column_health",
                "join_check",
                "designate_answer",
                "checklist",
                "write",
                "edit",
                "http_fetch",
                "http_download",
                "run_command",
                "shell",
            ],
        ),
        (
            ApprovalPolicy::Bypass,
            false,
            vec![
                "schema_discovery",
                "request_clarification",
                "read",
                "list",
                "glob",
                "grep",
                "bounded_sql_query",
                "bounded_sql_query_all",
                "result_shape",
                "column_health",
                "join_check",
                "designate_answer",
                "checklist",
                "write",
                "edit",
                "http_fetch",
                "http_download",
                "run_command",
                "shell",
            ],
        ),
        (ApprovalPolicy::ReadOnly, true, read_only_names),
        (
            ApprovalPolicy::Never,
            true,
            vec![
                "schema_discovery",
                "request_clarification",
                "read",
                "list",
                "glob",
                "grep",
                "bounded_sql_query",
                "bounded_sql_query_all",
                "result_shape",
                "column_health",
                "join_check",
                "designate_answer",
            ],
        ),
    ] {
        assert_eq!(
            advertised(&universe, AgentMode::Build, mode, can_obtain_approval),
            names
                .into_iter()
                .map(str::to_string)
                .collect::<Vec<String>>(),
            "Build under {mode:?} pins the advertised names"
        );
    }
    let _ = (fs::remove_dir_all(&project), fs::remove_dir_all(&state));
}

/// `render_chart` is advertised on no session surface: no approval policy,
/// either agent mode, with or without an approval surface.
#[test]
fn the_session_universe_never_advertises_the_retired_render_chart() {
    let project = worktree("chart-retired");
    let state = temp_dir("chart-retired-state");
    let universe = compose(&session_runtime(), &project, &state);
    for agent_mode in [AgentMode::Build, AgentMode::Plan] {
        for policy in [
            ApprovalPolicy::Ask,
            ApprovalPolicy::Bypass,
            ApprovalPolicy::ReadOnly,
            ApprovalPolicy::Never,
        ] {
            for can_obtain_approval in [false, true] {
                let definitions =
                    universe.definitions(
                        agent_mode,
                        false,
                        policy,
                        can_obtain_approval,
                        true,
                        false,
                        false,
                        &Default::default(),
                    );
                let names: Vec<&str> =
                    definitions.iter().map(|tool| tool.name.as_str()).collect();
                assert!(
                    !names.contains(&"render_chart"),
                    "render_chart is retired; {agent_mode:?}/{policy:?} \
                     (can_obtain_approval={can_obtain_approval}) still advertises it: {names:?}"
                );
            }
        }
    }
    let _ = (fs::remove_dir_all(&project), fs::remove_dir_all(&state));
}
