/// Under `read-only` and under `never` — and under `ask` where no approval
/// surface exists at all — none of the write-shaped tools are advertised:
/// the advertised-but-unusable anti-pattern (a definition the engine always
/// denies) must not return. Everything read-shaped stays.
///
/// The flag is the approval-surface flag, not the stdin flag: a surface
/// that can obtain approvals (the line REPL, the TUI's modal) advertises
/// under `ask`; a surface that can obtain none still hides. `read-only`
/// and `never` hide on every surface, approval surface or not.
#[test]
fn write_shaped_tools_stay_hidden_where_a_prompt_is_impossible() {
    let project = worktree("readonly");
    let state = temp_dir("readonly-state");
    let universe = compose(&session_runtime(None), &project, &state);
    for (mode, can_obtain_approval, label) in [
        (ApprovalPolicy::ReadOnly, true, "read-only"),
        (ApprovalPolicy::Never, true, "never"),
        (ApprovalPolicy::Ask, false, "no approval surface"),
        (
            ApprovalPolicy::ReadOnly,
            false,
            "read-only, no approval surface",
        ),
    ] {
        let names = advertised(&universe, AgentMode::Build, mode, can_obtain_approval);
        for tool in SESSION_WRITE_TOOLS {
            assert!(
                !names.contains(&tool.to_string()),
                "{mode_label} must not advertise {tool}: {names:?}",
                mode_label = label,
                names = names
            );
        }
    }
    let _ = (fs::remove_dir_all(&project), fs::remove_dir_all(&state));
}

/// The anti-silent-degradation pin (DESIGN §3): under `bypass` the session
/// advertises every write-shaped tool with no prompt surface at all — that is
/// the mode's meaning. Advertised-but-unusable cannot return under bypass,
/// because the advertised tools *are* usable: the engine resolves `Allow`.
/// A variant added without restating the advertisement rule would degrade
/// silently into "read-only with auto-approved SQL", hiding every
/// write-shaped tool while claiming everything runs.
#[test]
fn bypass_advertises_the_write_shaped_tools_without_a_prompt_surface() {
    let project = worktree("bypass-advertise");
    let state = temp_dir("bypass-advertise-state");
    let universe = compose(&session_runtime(None), &project, &state);
    for can_obtain_approval in [true, false] {
        let names = advertised(
            &universe,
            AgentMode::Build,
            ApprovalPolicy::Bypass,
            can_obtain_approval,
        );
        for tool in [
            "workspace_write",
            "workspace_edit",
            "scratch_sql",
            "http_fetch",
            "http_download",
        ] {
            assert!(
                names.contains(&tool.to_string()),
                "bypass advertises {tool} whether or not a prompt surface exists \
                 (can_obtain_approval={can_obtain_approval}): {names:?}"
            );
        }
        let definitions = universe.definitions(
            saya_agent::AgentMode::Build,
            ApprovalPolicy::Bypass,
            can_obtain_approval,
            true,
            false,
            false,
        );
        for tool in ["workspace_write", "workspace_edit"] {
            let write = definitions
                .iter()
                .find(|definition| definition.name == tool)
                .expect("the guarded file tool is advertised under bypass");
            assert!(
                write.effect.requires_approval,
                "{tool} still declares its approval shape honestly: the engine resolves it"
            );
        }
    }
    let _ = (fs::remove_dir_all(&project), fs::remove_dir_all(&state));
}

/// The digest-guarded edit rides beside the write, and only there: a bound
/// Build session advertises `workspace_edit` exactly once — no duplicate
/// from the run surface (whose `permit_workspace_writes` is off) and no
/// drift into compositions that cannot use it — while an unbound session
/// (no workspace root) and Plan mode never see it.
#[test]
fn a_bound_session_advertises_workspace_edit_exactly_once_and_unbound_and_plan_do_not() {
    let project = worktree("edit-advertise");
    let state = temp_dir("edit-advertise-state");
    let universe = compose(&session_runtime(None), &project, &state);
    let names = advertised(&universe, AgentMode::Build, ApprovalPolicy::Bypass, false);
    assert_eq!(
        names
            .iter()
            .filter(|name| *name == "workspace_edit")
            .count(),
        1,
        "the bound Build session advertises workspace_edit exactly once: {names:?}"
    );
    let plain = temp_dir("edit-advertise-unbound");
    let plain_state = temp_dir("edit-advertise-unbound-state");
    let unbound = SessionUniverse::compose(
        &session_runtime(None),
        None,
        None,
        true,
        &plain,
        &plain_state,
    )
    .expect("composition succeeds without a root");
    assert!(
        !advertised(&unbound, AgentMode::Build, ApprovalPolicy::Bypass, false)
            .contains(&"workspace_edit".to_string()),
        "no root, no edit tool"
    );
    assert!(
        !advertised(&universe, AgentMode::Plan, ApprovalPolicy::Bypass, false)
            .contains(&"workspace_edit".to_string()),
        "Plan hides the edit tool even under bypass"
    );
    let _ = (
        fs::remove_dir_all(&project),
        fs::remove_dir_all(&state),
        fs::remove_dir_all(&plain),
        fs::remove_dir_all(&plain_state),
    );
}
