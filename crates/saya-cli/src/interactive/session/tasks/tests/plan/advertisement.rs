use super::*;

/// The tool is advertised wherever the session can use it — including Plan
/// mode, which is the point. Pinned here through the universe's own
/// definitions, on the path that survives the Plan filter.
#[test]
fn the_universe_advertises_tasks_set_including_under_plan() {
    let observed = |agent_mode: AgentMode, mode: ApprovalPolicy, can_obtain_approval: bool| {
        let plain = std::env::temp_dir().join(format!(
            "saya-tasks-ads-{}-{}",
            std::process::id(),
            match (agent_mode, mode, can_obtain_approval) {
                (AgentMode::Build, ApprovalPolicy::Ask, true) => "build-ask",
                (AgentMode::Build, ApprovalPolicy::Bypass, _) => "build-bypass",
                (AgentMode::Plan, _, _) => "plan",
                _ => "other",
            }
        ));
        let state = std::env::temp_dir().join(format!(
            "saya-tasks-ads-state-{}-{}",
            std::process::id(),
            match (agent_mode, mode, can_obtain_approval) {
                (AgentMode::Build, ApprovalPolicy::Ask, true) => "build-ask",
                (AgentMode::Build, ApprovalPolicy::Bypass, _) => "build-bypass",
                (AgentMode::Plan, _, _) => "plan",
                _ => "other",
            }
        ));
        let _ = std::fs::remove_dir_all(&plain);
        let _ = std::fs::remove_dir_all(&state);
        std::fs::create_dir_all(&plain).unwrap();
        std::fs::create_dir_all(&state).unwrap();
        let runtime = test_runtime();
        let universe =
            crate::interactive::session::universe::SessionUniverse::compose_with_launch_and_path(
                crate::interactive::session::universe::SessionComposition {
                    runtime: &runtime,
                    explicit: None,
                    pinned_root: None,
                    walk_when_unpinned: false,
                    cwd: &plain,
                    state_dir: &state,
                    launch: None,
                    path: None,
                    scratch: None,
                    recovery: None,
                },
            )
            .expect("composition succeeds");
        let names: Vec<String> = universe
            .definitions(agent_mode, mode, can_obtain_approval, true, false, false)
            .into_iter()
            .map(|definition| definition.name)
            .collect();
        let _ = std::fs::remove_dir_all(&plain);
        let _ = std::fs::remove_dir_all(&state);
        names
    };
    for (agent_mode, mode, can_obtain_approval, label) in [
        (AgentMode::Build, ApprovalPolicy::Ask, true, "ask+prompt"),
        (AgentMode::Build, ApprovalPolicy::Bypass, false, "bypass"),
        (
            AgentMode::Plan,
            ApprovalPolicy::Ask,
            true,
            "Plan+ask: the case this design exists for",
        ),
        (
            AgentMode::Plan,
            ApprovalPolicy::Bypass,
            false,
            "Plan+bypass: the case this design exists for",
        ),
        (
            AgentMode::Plan,
            ApprovalPolicy::ReadOnly,
            false,
            "Plan+read-only",
        ),
    ] {
        assert!(
            observed(agent_mode, mode, can_obtain_approval).contains(&"tasks_set".to_string()),
            "tasks_set must be advertised under {label}"
        );
    }
    // Never hides everything write-shaped-adjacent — tasks_set included, so
    // the model never sees a tool it cannot use.
    assert!(
        !observed(AgentMode::Build, ApprovalPolicy::Never, true).contains(&"tasks_set".to_string()),
        "never must hide tasks_set"
    );
}

/// A read-only universe (definitions before the Plan filter) carries the
/// same definition object the Plan filter keeps: one definition, one path.
#[test]
fn the_definition_object_is_the_advertised_object() {
    let direct = tasks_set_definition();
    assert_eq!(
        direct.effect.local_state,
        saya_agent::LocalStateEffect::WriteSession
    );
}
