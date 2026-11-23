//! Twins of the plan rows PT4 deleted with the old universe
//! (`universe/tests/plan_read_only.rs` and `plan_enforcement.rs`) on the live
//! coding root's gate.
//!
//! The old file asserted three things the live gate tests did not: the closed
//! advertisement is *exactly* the read-only surface minus the shell, no
//! advertised definition carries a `WriteWorkspace` permit (the old runtime's
//! derived write permission read that bit off this list), and every advertised
//! definition is one the plan posture's enforcement does not deny. The live
//! gate composes the same three facts from `PlanGate` and
//! `SessionPolicy::with_agent_mode(AgentMode::Plan)`, so this file states them
//! against the live shapes.

use saya_agent::{
    AgentMode, ApprovalDecision, ApprovalPolicy, LocalStateEffect, SessionPolicy, ToolDefinition,
};

use super::direct::RootTools;
use super::plan_gate_tests::{READ_SURFACE, plan_root};
use crate::agent::tools::child::tests_root_compose_fixture as fixture;

fn names(definitions: &[ToolDefinition]) -> Vec<String> {
    definitions
        .iter()
        .map(|definition| definition.name.clone())
        .collect()
}

/// A closed plan posture advertises the read surface and nothing else — no
/// members beyond `READ_SURFACE`, and no definition carrying a workspace-write
/// permit. Read from the same list the dispatch gate filters, so the
/// advertisement cannot drift from what a call may reach.
#[tokio::test]
async fn a_closed_plan_advertises_exactly_the_read_surface_and_no_write_permit() {
    let fixture = fixture::Fixture::new("plan-twin-surface", &["tracked.txt"]).await;
    let root = plan_root(&fixture, None).await;
    let advertised = root.definitions();
    let advertised_names = names(&advertised);

    for name in READ_SURFACE {
        assert!(
            advertised_names.iter().any(|advertised| advertised == name),
            "a closed plan still offers {name}: {advertised_names:?}"
        );
    }
    for definition in &advertised {
        assert!(
            definition.effect.local_state != LocalStateEffect::WriteWorkspace,
            "no Plan definition carries a workspace-write permit, so the write \
             permission derived from this list is false: {}",
            definition.name
        );
    }
    for name in &advertised_names {
        assert!(
            READ_SURFACE.contains(&name.as_str()),
            "a closed plan advertises only the read surface; {name} is beyond it: {advertised_names:?}"
        );
    }
}

/// Advertisement matches enforcement: every definition a closed Plan posture
/// advertises is one `SessionPolicy` under Plan does not deny — under each
/// consent posture the root can compose with. An advertised-but-always-denied
/// tool is the anti-pattern the hidden-not-offered rule exists to prevent.
#[tokio::test]
async fn every_advertised_plan_definition_is_one_plan_enforcement_does_not_deny() {
    let fixture = fixture::Fixture::new("plan-twin-anti-drift", &["tracked.txt"]).await;
    let root: RootTools = plan_root(&fixture, None).await;
    let advertised = root.definitions();

    for policy in [
        ApprovalPolicy::Ask,
        ApprovalPolicy::Bypass,
        ApprovalPolicy::ReadOnly,
    ] {
        let enforcer = SessionPolicy::new(policy).with_agent_mode(AgentMode::Plan);
        for definition in &advertised {
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
}
