/// The derived permit falls off: a Plan definition list contains no
/// `WriteWorkspace` definition, so the derivation at `runtime.rs:216-218`
/// yields `false`. Asserted on the definition list itself — the derivation
/// is only reachable through the full turn composition, and it reads this
/// exact property off the list.
#[test]
fn plan_definitions_carry_no_workspace_write_permit() {
    use saya_agent::LocalStateEffect;
    let project = worktree("plan-permit");
    let state = temp_dir("plan-permit-state");
    let universe = compose(&session_runtime(None), &project, &state);
    let definitions = universe.definitions(
        AgentMode::Plan,
        ApprovalPolicy::Ask,
        true,
        true,
        false,
        false,
    );
    assert!(
        definitions
            .iter()
            .all(|definition| definition.effect.local_state != LocalStateEffect::WriteWorkspace),
        "no Plan definition writes the workspace, so the runtime derivation reads false: {:?}",
        definitions
            .iter()
            .map(|definition| &definition.name)
            .collect::<Vec<_>>()
    );
    let _ = (fs::remove_dir_all(&project), fs::remove_dir_all(&state));
}

/// Advertisement matches enforcement: for every definition Plan
/// advertises, `SessionPolicy` under Plan does not deny it. An
/// advertised-but-always-refused tool is the exact anti-pattern the
/// hidden-not-advertised rule exists to prevent — this is the anti-drift
/// test between slice 1 and slice 2. `permit_candidate_writes` admits
/// nothing write-shaped into the database surface — the flag gates no tool
/// declared today.
#[test]
fn every_plan_advertised_definition_is_not_denied_by_plan_enforcement() {
    use saya_agent::{ApprovalDecision, SessionPolicy};
    let project = worktree("plan-anti-drift");
    let state = temp_dir("plan-anti-drift-state");
    let universe = compose(&session_runtime(None), &project, &state);
    for policy in [
        ApprovalPolicy::Ask,
        ApprovalPolicy::Bypass,
        ApprovalPolicy::ReadOnly,
    ] {
        let definitions = universe.definitions(
            AgentMode::Plan,
            policy,
            policy == ApprovalPolicy::Ask,
            true,
            false,
            false,
        );
        let enforcer = SessionPolicy::new(policy).with_agent_mode(AgentMode::Plan);
        for definition in &definitions {
            assert!(
                !matches!(
                    enforcer.resolve(&definition.effect, None),
                    ApprovalDecision::Deny { .. }
                ),
                "Plan advertises {} under {policy:?} but enforcement denies it",
                definition.name
            );
        }
    }
    let _ = (fs::remove_dir_all(&project), fs::remove_dir_all(&state));
}
