/// At the real definitions boundary: a headless ask session
/// (no approval surface, `can_obtain_approval=false`) whose live policy
/// holds an explicit `workspace-write` grant advertises exactly the two
/// guarded file tools — the grant pre-answers their calls
/// (`SessionPolicy::resolve` → `Allow`), so the advertised tools are
/// usable, never the advertised-but-unusable anti-pattern — and nothing
/// else write-shaped: the grant is one family, not a posture. No
/// `run_command`.
#[test]
fn a_headless_ask_session_advertises_only_the_two_file_tools_under_its_workspace_write_grant() {
    let project = worktree("grant-headless");
    let state = temp_dir("grant-headless-state");
    let universe = compose(&session_runtime(), &project, &state);
    let policy = granted(&universe, &["workspace-write"]);
    let names = advertised(&universe, AgentMode::Build, ApprovalPolicy::Ask, false);
    for tool in ["write", "edit"] {
        assert!(
            names.contains(&tool.to_string()),
            "the granted workspace file family advertises under headless ask: {names:?}"
        );
    }
    for tool in [
        "http_fetch",
        "http_download",
        "run_command",
    ] {
        assert!(
            !names.contains(&tool.to_string()),
            "a workspace-write grant widens no other family: {tool} stays hidden: {names:?}"
        );
    }
    // The advertised calls are usable and honestly declared: the engine
    // resolves the grant to `Allow`, and each definition still carries its
    // per-call approval shape.
    let definitions = universe.definitions(
        AgentMode::Build,
        false,
        ApprovalPolicy::Ask,
        false,
        true,
        false,
        false,
    
        &Default::default(),
    );
    for tool in ["write", "edit"] {
        let definition = definitions
            .iter()
            .find(|definition| definition.name == tool)
            .expect("the granted file tool is advertised");
        assert!(
            definition.effect.requires_approval,
            "{tool} declares its approval shape honestly: the engine resolves the grant"
        );
    }
    let effect = &definitions
        .iter()
        .find(|definition| definition.name == "write")
        .expect("write advertised")
        .effect;
    assert_eq!(
        policy.resolve(effect, Some("workspace-write")),
        saya_agent::ApprovalDecision::Allow,
        "the grant pre-answers the advertised call: usable, not advertised-but-denied"
    );
    let _ = (fs::remove_dir_all(&project), fs::remove_dir_all(&state));
}

/// A held workspace-write grant cannot widen a restrictive posture:
/// read-only and never never consult grants (the engine denies without
/// them), and Plan denies before the approval match — so the file tools
/// stay hidden under each, grant or no grant. The advertisement rule never
/// outruns the engine.
#[test]
fn a_workspace_write_grant_cannot_widen_read_only_never_or_plan() {
    let project = worktree("grant-restrictive");
    let state = temp_dir("grant-restrictive-state");
    let universe = compose(&session_runtime(), &project, &state);
    granted(&universe, &["workspace-write"]);
    for (agent_mode, mode, can_obtain_approval, label) in [
        (
            AgentMode::Build,
            ApprovalPolicy::ReadOnly,
            true,
            "read-only",
        ),
        (AgentMode::Build, ApprovalPolicy::Never, true, "never"),
        (
            AgentMode::Plan,
            ApprovalPolicy::Ask,
            false,
            "plan, headless",
        ),
        (
            AgentMode::Plan,
            ApprovalPolicy::Ask,
            true,
            "plan, with a surface",
        ),
    ] {
        let names = advertised(&universe, agent_mode, mode, can_obtain_approval);
        for tool in ["write", "edit"] {
            assert!(
                !names.contains(&tool.to_string()),
                "{label} stays closed under a held grant: {names:?}"
            );
        }
    }
    let _ = (fs::remove_dir_all(&project), fs::remove_dir_all(&state));
}

/// No grant, no advertisement — the headless ask shape stays exactly as
/// today — and a grant from another family (`fetch:`) is not the workspace
/// file family: one token authorizes its own family only.
#[test]
fn an_unbound_view_and_a_foreign_grant_keep_the_headless_ask_closed() {
    let project = worktree("grant-closed");
    let state = temp_dir("grant-closed-state");
    let universe = compose(&session_runtime(), &project, &state);
    let names = advertised(&universe, AgentMode::Build, ApprovalPolicy::Ask, false);
    for tool in ["write", "edit"] {
        assert!(
            !names.contains(&tool.to_string()),
            "no grant: the headless ask stays closed exactly as today: {names:?}"
        );
    }
    granted(&universe, &["fetch:https+docs.example"]);
    let names = advertised(&universe, AgentMode::Build, ApprovalPolicy::Ask, false);
    for tool in ["write", "edit"] {
        assert!(
            !names.contains(&tool.to_string()),
            "a foreign family's grant authorizes nothing here: {names:?}"
        );
    }
    let _ = (fs::remove_dir_all(&project), fs::remove_dir_all(&state));
}

/// The interactive shapes do not move: ask with an approval surface
/// advertises the full write-shaped set exactly as before, grant held or
/// not — and the granted case advertises the file tools exactly once, no
/// duplicate from the grant arm. Bypass is untouched: the mode allows on
/// its own.
#[test]
fn interactive_ask_advertisement_is_unchanged_under_a_bound_grant_view() {
    let project = worktree("grant-interactive");
    let state = temp_dir("grant-interactive-state");
    let universe = compose(&session_runtime(), &project, &state);
    granted(&universe, &["workspace-write"]);
    let names = advertised(&universe, AgentMode::Build, ApprovalPolicy::Ask, true);
    for tool in [
        "write",
        "edit",
        "http_fetch",
        "http_download",
        "run_command",
    ] {
        assert!(
            names.contains(&tool.to_string()),
            "an approval surface advertises as before, grant or no grant: {names:?}"
        );
    }
    assert_eq!(
        names
            .iter()
            .filter(|name| *name == "edit")
            .count(),
        1,
        "the granted ask advertises edit exactly once: {names:?}"
    );
    let bypass = advertised(&universe, AgentMode::Build, ApprovalPolicy::Bypass, false);
    for tool in [
        "write",
        "edit",
        "http_fetch",
    ] {
        assert!(
            bypass.contains(&tool.to_string()),
            "bypass unchanged under a bound grant view: {bypass:?}"
        );
    }
    let _ = (fs::remove_dir_all(&project), fs::remove_dir_all(&state));
}
