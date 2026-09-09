//! The read-only approval rule, pinned as an (effect shape → permit) matrix
//! over the `AllowReadOnlyApproval` decider. Read-only approval auto-approves
//! only read-shaped tools — no external side effect, no local-state write. Two
//! invariants are load-bearing: `requires_approval` does not mean
//! side-effecting (the SQL tools declare `requires_approval: true,
//! external_side_effect: false` and stay auto-approved under read-only, which
//! is what the bench harness runs on), and the decider decides from the
//! declared effect, never from the tool name or arguments.

use saya_agent::{
    AllowReadOnlyApproval, ApprovalDecider, LocalStateEffect, ToolDefinition, ToolEffect,
    read_only_permits,
};

fn definition(effect: ToolEffect) -> ToolDefinition {
    ToolDefinition {
        name: "tool_under_test".into(),
        description: "matrix subject".into(),
        read_only: true,
        parameters: serde_json::json!({"type": "object"}),
        effect,
        // The matrix is about effects, not completion wording.
        completion: None,
    }
}

/// Every effect shape that exists today, and whether read-only approval
/// permits it. The `render_chart` cell is the behavioural change: the tool
/// requires approval *and* has an external side effect, so read-only denies it
/// even though it still advertises itself as a database tool.
fn cases() -> Vec<(&'static str, ToolEffect, bool)> {
    vec![
        (
            "bounded_sql_query shape: requires approval, no side effect — stays auto-approved",
            ToolEffect {
                database_data: true,
                external_side_effect: false,
                requires_approval: true,
                local_state: LocalStateEffect::None,
            },
            true,
        ),
        (
            "contract_search shape: contained local read",
            ToolEffect {
                database_data: false,
                external_side_effect: false,
                requires_approval: false,
                local_state: LocalStateEffect::Read,
            },
            true,
        ),
        (
            "schema_discovery shape: no effect at all",
            ToolEffect {
                database_data: false,
                external_side_effect: false,
                requires_approval: false,
                local_state: LocalStateEffect::None,
            },
            true,
        ),
        (
            "render_chart shape: requires approval AND has an external side effect",
            ToolEffect {
                database_data: false,
                external_side_effect: true,
                requires_approval: true,
                local_state: LocalStateEffect::None,
            },
            false,
        ),
        (
            "external side effect without requires_approval (misconfiguration)",
            ToolEffect {
                database_data: false,
                external_side_effect: true,
                requires_approval: false,
                local_state: LocalStateEffect::None,
            },
            false,
        ),
        (
            "candidate-writing tool (persisting a claim)",
            ToolEffect {
                database_data: false,
                external_side_effect: false,
                requires_approval: false,
                local_state: LocalStateEffect::WriteCandidate,
            },
            false,
        ),
        (
            "external side effect and candidate write together",
            ToolEffect {
                database_data: false,
                external_side_effect: true,
                requires_approval: true,
                local_state: LocalStateEffect::WriteCandidate,
            },
            false,
        ),
    ]
}

/// The read-only policy denies every side-effecting shape and auto-approves
/// every read-shaped one, regardless of the tool's name or arguments.
#[tokio::test]
async fn allow_read_only_approval_denies_every_side_effecting_shape() {
    let decider = AllowReadOnlyApproval;
    for (label, effect, permitted) in cases() {
        let approved = decider
            .approve(&definition(effect), &serde_json::json!({}))
            .await;
        assert_eq!(approved, permitted, "{label}");
    }
}

/// The rule behind the policy, pinned cell by cell: `read_only_permits` is
/// exactly "no external side effect and no local-state write".
#[test]
fn read_only_permits_exactly_the_read_shaped_effects() {
    for (label, effect, permitted) in cases() {
        assert_eq!(read_only_permits(&effect), permitted, "{label}");
    }
}

/// The load-bearing invariant: needing approval is not side-effecting. The SQL
/// tools declare `requires_approval: true, external_side_effect: false` and
/// must stay auto-approved under read-only.
#[tokio::test]
async fn requiring_approval_is_not_side_effecting() {
    let sql = ToolEffect {
        database_data: true,
        external_side_effect: false,
        requires_approval: true,
        local_state: LocalStateEffect::None,
    };
    assert!(
        AllowReadOnlyApproval
            .approve(&definition(sql), &serde_json::json!({}))
            .await,
        "a read-shaped tool that requires approval stays auto-approved under read-only"
    );
}
