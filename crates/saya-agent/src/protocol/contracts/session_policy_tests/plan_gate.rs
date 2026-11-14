/// The gate is the plan posture's one door, and it is one shared cell: with a
/// gate composed, an approved plan resolves a write exactly as the consent
/// posture alone would, and closing the gate puts the posture's denial back.
/// Nothing about the effect changes — only the door.
#[test]
fn an_open_gate_is_the_plan_postures_one_door() {
    let cell = PlanGateCell::default();
    let gated = with_mode(ApprovalPolicy::Ask, AgentMode::Plan).with_plan_gate(Some(cell.clone()));
    assert!(
        !gated.plan_gate_open(),
        "a gate composed shut is the closed shape"
    );
    assert_eq!(
        gated.resolve(&write_shaped(), None),
        ApprovalDecision::Deny { reason: None },
        "a shut gate leaves the plan posture's write denial exactly as it was"
    );
    cell.open();
    assert!(gated.plan_gate_open(), "the cell is the policy's own");
    assert_eq!(
        gated.resolve(&write_shaped(), None),
        ApprovalDecision::Ask,
        "an open gate resolves the write as the consent posture alone would"
    );
    assert_eq!(
        gated.resolve(&external_effect(), None),
        ApprovalDecision::Ask,
        "the door opens the whole write-shaped surface, side effects included"
    );
    assert_eq!(
        gated.resolve(&read_shaped(), None),
        SessionPolicy::new(ApprovalPolicy::Ask).resolve(&read_shaped(), None),
        "a read is untouched by the gate either way"
    );
    cell.close();
    assert_eq!(
        gated.resolve(&write_shaped(), None),
        ApprovalDecision::Deny { reason: None },
        "closing the gate re-arms the denial, so leaving plan asks again"
    );
}

/// A gate widens the plan posture only. Under Build the write resolves as it
/// always did, open or shut: the gate is not a second consent posture.
#[test]
fn a_gate_widens_the_plan_posture_only() {
    let cell = PlanGateCell::default();
    let build =
        with_mode(ApprovalPolicy::ReadOnly, AgentMode::Build).with_plan_gate(Some(cell.clone()));
    assert_eq!(
        build.resolve(&write_shaped(), None),
        ApprovalDecision::Deny { reason: None },
        "read-only denies the write under Build"
    );
    cell.open();
    assert_eq!(
        build.resolve(&write_shaped(), None),
        ApprovalDecision::Deny { reason: None },
        "an open gate does not widen Build: read-only still denies"
    );
    let plan =
        with_mode(ApprovalPolicy::ReadOnly, AgentMode::Plan).with_plan_gate(Some(cell.clone()));
    assert_eq!(
        plan.resolve(&write_shaped(), None),
        ApprovalDecision::Deny { reason: None },
        "the gate is not a consent of its own: read-only still refuses under plan"
    );
    let bypass = with_mode(ApprovalPolicy::Bypass, AgentMode::Plan).with_plan_gate(Some(cell));
    assert_eq!(
        bypass.resolve(&write_shaped(), None),
        ApprovalDecision::Allow,
        "under bypass the gate's approval is the only thing that moved"
    );
}

/// No gate is the closed shape: a policy composed without one — the headless
/// run's, and every policy that predates the gate — denies under `plan`
/// exactly as it did before the gate existed.
#[test]
fn no_gate_is_the_closed_shape() {
    let plan = with_mode(ApprovalPolicy::Bypass, AgentMode::Plan);
    assert!(!plan.plan_gate_open());
    assert_eq!(
        plan.resolve(&write_shaped(), None),
        ApprovalDecision::Deny { reason: None },
        "a policy with no gate has no door"
    );
    assert_eq!(
        plan.with_plan_gate(None).resolve(&write_shaped(), None),
        ApprovalDecision::Deny { reason: None },
        "binding `None` is the same closed shape"
    );
}
