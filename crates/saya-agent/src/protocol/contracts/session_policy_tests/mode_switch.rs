/// A switch is visible to a turn already in flight: every decider holds a
/// clone, the clones share the mode cells, so the turn's own policy resolves
/// its next call under the mode the switch just wrote.
#[test]
fn a_mid_turn_set_mode_flips_the_next_resolve_on_a_cloned_policy() {
    let session = SessionPolicy::new(ApprovalPolicy::Ask);
    let turn = session.clone();
    assert_eq!(
        turn.resolve(&side_effecting(), None),
        ApprovalDecision::Ask,
        "the turn starts under the session's mode"
    );
    assert!(
        session.set_mode(ApprovalPolicy::Bypass),
        "an interactive session's mode is swappable"
    );
    assert_eq!(
        turn.resolve(&side_effecting(), None),
        ApprovalDecision::Allow,
        "the in-flight turn's next call reads the mode the switch wrote"
    );
    assert_eq!(
        session.mode(),
        ApprovalPolicy::Bypass,
        "one cell: the session reads exactly what the turn reads"
    );
}

/// The task posture rides the same cell: a mid-turn switch into `plan`
/// narrows the in-flight turn's next call, and the switch back out widens it
/// again — nothing about the turn is rebuilt.
#[test]
fn a_mid_turn_set_agent_mode_narrows_the_next_resolve() {
    let session = SessionPolicy::new(ApprovalPolicy::Bypass);
    let turn = session.clone();
    assert_eq!(
        turn.resolve(&write_shaped(), None),
        ApprovalDecision::Allow,
        "bypass allows the write to begin with"
    );
    assert!(session.set_agent_mode(AgentMode::Plan));
    assert_eq!(
        turn.resolve(&write_shaped(), None),
        ApprovalDecision::Deny { reason: None },
        "the plan posture reaches the in-flight turn's next call"
    );
    assert_eq!(turn.agent_mode(), AgentMode::Plan);
    assert!(session.set_agent_mode(AgentMode::Build));
    assert_eq!(
        turn.resolve(&write_shaped(), None),
        ApprovalDecision::Allow,
        "and the way back out is the same cell"
    );
}

/// The grants ride a mid-turn switch the way they ride a rebuilt policy: the
/// store is shared by every clone, and a switch moves the mode, never the
/// store.
#[test]
fn a_mid_turn_switch_carries_the_grants() {
    let session = SessionPolicy::new(ApprovalPolicy::Ask);
    assert!(session.record(ApprovalChoice::AllowSession {
        token: "runner:report".into(),
    }));
    let turn = session.clone();
    assert!(session.set_mode(ApprovalPolicy::ReadOnly));
    assert!(
        turn.grants().is_granted("runner:report"),
        "the grant survives the switch"
    );
    assert!(session.set_mode(ApprovalPolicy::Ask));
    assert_eq!(
        turn.resolve(&side_effecting(), Some("runner:report")),
        ApprovalDecision::Allow,
        "back under ask, the pre-switch grant pre-answers as before"
    );
}

/// A frozen policy — a headless run's — refuses both setters: its approval is
/// stated before the run starts and cannot be swapped under it. The refusal
/// leaves the posture exactly where it was.
#[test]
fn a_frozen_policy_refuses_both_setters() {
    let policy = SessionPolicy::frozen(ApprovalPolicy::Ask, &[]);
    assert!(
        !policy.set_mode(ApprovalPolicy::Bypass),
        "a run's policy is never swappable"
    );
    assert!(
        !policy.set_agent_mode(AgentMode::Plan),
        "and its task posture is the run's own, fixed before it started"
    );
    assert_eq!(policy.mode(), ApprovalPolicy::Ask);
    assert_eq!(policy.agent_mode(), AgentMode::Build);
    assert_eq!(
        policy.resolve(&read_shaped(), None),
        ApprovalDecision::Deny {
            reason: Some(HEADLESS_ASK_REASON)
        },
        "the refused switch moved nothing: the run still resolves as it did"
    );
}
