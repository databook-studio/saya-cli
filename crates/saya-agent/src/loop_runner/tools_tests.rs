//! The side-effect guard and the tool-message cap: the S2 loop seam's pins.

use super::*;
use crate::{LocalStateEffect, ToolDefinition, ToolEffect};

/// The fetch-shaped declaration: an external side effect approved once by
/// the run's scope, not per call — the exact combination the misconfiguration
/// guard exists to refuse.
fn plan_gated_egress() -> ToolDefinition {
    ToolDefinition {
        name: "http_fetch".into(),
        description: "the fetch lane".into(),
        read_only: false,
        parameters: serde_json::json!({"type": "object"}),
        effect: ToolEffect {
            database_data: false,
            external_side_effect: true,
            requires_approval: false,
            local_state: LocalStateEffect::None,
        },
        completion: None,
    }
}

/// Default limits deny the plan-gated egress tool in both paths: the guard
/// is unchanged wherever the permit is false, so interactive `ask` turns are
/// byte-identical, and an author who set the bit carelessly is still refused.
#[test]
fn the_guard_stands_everywhere_the_permit_is_false() {
    let definition = plan_gated_egress();
    assert!(
        external_side_effect_gated(&definition, &AgentLimits::default()),
        "the misconfiguration guard keeps denying by default"
    );
    assert!(
        !auto_runnable(&definition, &AgentLimits::default()),
        "the batch path refuses it by default"
    );
}

/// The permit opens the gated tool in the shared policy both paths consult:
/// `auto_runnable` (the batch path) and the sequential path's by-name gate
/// resolve through the same function, so a permit granted per step cannot
/// run in one path and not the other.
#[test]
fn the_permit_opens_the_gated_tool_in_both_paths() {
    let definition = plan_gated_egress();
    let limits = AgentLimits {
        permit_external_effects: true,
        ..AgentLimits::default()
    };
    assert!(
        !external_side_effect_gated(&definition, &limits),
        "the plan-gated egress permit satisfies the guard"
    );
    assert!(
        auto_runnable(&definition, &limits),
        "with the permit, a no-approval external tool is auto-runnable in \
         the batch path — and the sequential path consults the same gate"
    );
}

/// The permit cannot smuggle a write-shaped tool past the other gates: the
/// fetch downloader is external *and* a workspace write, so it stays denied
/// by the write permit even with external effects permitted.
#[test]
fn the_permit_does_not_carry_the_write_shaped_members() {
    let mut definition = plan_gated_egress();
    definition.name = "http_download".into();
    definition.effect.local_state = LocalStateEffect::WriteWorkspace;
    let limits = AgentLimits {
        permit_external_effects: true,
        ..AgentLimits::default()
    };
    assert!(
        !external_side_effect_gated(&definition, &limits),
        "the external gate is satisfied by the permit"
    );
    assert!(
        !auto_runnable(&definition, &limits),
        "the workspace-write gate still refuses it: one permit per shape"
    );
}

/// The drift pin: the exported cap is exactly the number the loop's own
/// `tool_message` truncates at. A serialized result at the cap arrives whole;
/// one byte over it is cut with the marker. The harness's fetch bound
/// derives from this same helper, so the pre-bound and the truncation point
/// are the same number by construction.
#[test]
fn the_exported_cap_is_the_loop_s_truncation_point() {
    let budget = usize::MAX;
    let cap = tool_message_cap(budget);
    assert_eq!(
        cap, MAX_TOOL_MESSAGE_BYTES,
        "the absolute ceiling is the cap for any larger budget"
    );
    // A string whose serialized form is exactly the cap arrives untruncated.
    let whole = Value::String("x".repeat(cap - 2));
    assert_eq!(
        serde_json::to_string(&whole).unwrap().len(),
        cap,
        "test setup: the value serializes to exactly the cap"
    );
    let (message, truncated) = tool_message("c1".into(), whole, budget);
    assert!(
        !truncated && message.content.len() == cap,
        "a result at the cap is whole: {truncated}, {}",
        message.content.len()
    );
    // One byte over the cap is cut, with the loop's own marker.
    let over = Value::String("x".repeat(cap - 1));
    let (message, truncated) = tool_message("c1".into(), over, budget);
    assert!(
        truncated && message.content.len() <= cap,
        "a result over the cap is cut at the cap: {truncated}, {}",
        message.content.len()
    );
    assert!(
        message.content.contains("[truncated:"),
        "the cut must be visible to the model: {}",
        message.content.len()
    );
    // A budget tighter than the absolute ceiling binds first — the helper is
    // `min`, both ways.
    assert_eq!(tool_message_cap(1024), 1024);
}
