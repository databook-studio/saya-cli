fn write_shaped() -> ToolEffect {
    // The session's workspace_write shape: requires approval, writes the
    // workspace, no external side effect.
    ToolEffect {
        database_data: false,
        external_side_effect: false,
        requires_approval: true,
        local_state: LocalStateEffect::WriteWorkspace,
    }
}

fn session_write_shaped() -> ToolEffect {
    // The session task list's shape: writes session-scoped metadata only, no
    // external side effect.
    ToolEffect {
        database_data: false,
        external_side_effect: false,
        requires_approval: false,
        local_state: LocalStateEffect::WriteSession,
    }
}

/// A session-metadata write is read-shaped for approval purposes: read-only
/// allows it, and Plan allows it under every policy — a read-only or Plan
/// session that could not record what it is doing would be absurd. `never`
/// still denies it, and an external side effect beside the session write
/// still refuses.
#[test]
fn session_write_is_allowed_wherever_reads_are() {
    let read_only = SessionPolicy::new(ApprovalPolicy::ReadOnly);
    assert_eq!(
        read_only.resolve(&session_write_shaped(), None),
        ApprovalDecision::Allow,
        "read-only allows a session-metadata write"
    );
    for policy in [
        ApprovalPolicy::Ask,
        ApprovalPolicy::ReadOnly,
        ApprovalPolicy::Never,
        ApprovalPolicy::Bypass,
    ] {
        let plan = with_mode(policy, AgentMode::Plan).resolve(&session_write_shaped(), None);
        if policy == ApprovalPolicy::Never {
            assert_eq!(
                plan,
                ApprovalDecision::Deny { reason: None },
                "never denies everything, session writes included, under {policy:?}"
            );
        } else {
            let build = SessionPolicy::new(policy).resolve(&session_write_shaped(), None);
            assert_eq!(
                plan, build,
                "Plan leaves session writes exactly as {policy:?} resolves them"
            );
            assert!(
                !matches!(plan, ApprovalDecision::Deny { .. }),
                "Plan allows a session-metadata write under {policy:?}: bypass included"
            );
        }
    }
    let never = SessionPolicy::new(ApprovalPolicy::Never);
    assert_eq!(
        never.resolve(&session_write_shaped(), None),
        ApprovalDecision::Deny { reason: None },
        "never is a standing refusal for session writes too"
    );
    let side_effecting_session_write = ToolEffect {
        external_side_effect: true,
        ..session_write_shaped()
    };
    assert_eq!(
        read_only.resolve(&side_effecting_session_write, None),
        ApprovalDecision::Deny { reason: None },
        "a session write with an external side effect is still refused"
    );
    assert_eq!(
        with_mode(ApprovalPolicy::Bypass, AgentMode::Plan)
            .resolve(&side_effecting_session_write, None),
        ApprovalDecision::Deny { reason: None },
        "Plan still refuses a session write carrying an external side effect"
    );
}

fn external_effect() -> ToolEffect {
    ToolEffect {
        database_data: false,
        external_side_effect: true,
        requires_approval: true,
        local_state: LocalStateEffect::None,
    }
}

fn with_mode(policy: ApprovalPolicy, mode: AgentMode) -> SessionPolicy {
    SessionPolicy::new(policy).with_agent_mode(mode)
}

fn with_frozen_mode(policy: SessionPolicy, mode: AgentMode) -> SessionPolicy {
    policy.with_agent_mode(mode)
}

/// Plan narrows, never widens: under every approval policy a read-shaped
/// call resolves as it does under Build, while a workspace write or an
/// external side effect denies outright. Build is byte-identical to today.
#[test]
fn plan_narrows_every_approval_policy_without_moving_reads() {
    for policy in [
        ApprovalPolicy::Ask,
        ApprovalPolicy::ReadOnly,
        ApprovalPolicy::Never,
        ApprovalPolicy::Bypass,
    ] {
        for (effect, label) in [
            (read_shaped(), "read-shaped"),
            (write_shaped(), "workspace-write"),
            (external_effect(), "external side effect"),
        ] {
            let build = SessionPolicy::new(policy).resolve(&effect, None);
            let plan = with_mode(policy, AgentMode::Plan).resolve(&effect, None);
            if label == "read-shaped" {
                assert_eq!(
                    plan, build,
                    "Plan leaves reads exactly as {policy:?} resolves them: {label}"
                );
            } else {
                assert_eq!(
                    plan,
                    ApprovalDecision::Deny { reason: None },
                    "Plan denies what the approval policy would have allowed: {policy:?} {label}"
                );
            }
        }
    }
}

/// Bypass auto-allows reads only under Plan: a write-shaped call denies
/// where Build allows every effect without asking.
#[test]
fn plan_denies_writes_under_bypass() {
    let build = SessionPolicy::new(ApprovalPolicy::Bypass);
    assert_eq!(
        build.resolve(&write_shaped(), None),
        ApprovalDecision::Allow,
        "bypass allows a workspace write under Build, as today"
    );
    let plan = with_mode(ApprovalPolicy::Bypass, AgentMode::Plan);
    assert_eq!(
        plan.resolve(&write_shaped(), None),
        ApprovalDecision::Deny { reason: None },
        "Plan denies the write bypass would have allowed"
    );
    assert_eq!(
        plan.resolve(&external_effect(), None),
        ApprovalDecision::Deny { reason: None },
        "Plan denies the side effect bypass would have allowed"
    );
    assert_eq!(
        plan.resolve(&read_shaped(), None),
        ApprovalDecision::Allow,
        "Plan keeps the read bypass auto-allows"
    );
}

/// A grant made in Build is inert under Plan and live again on return: the
/// same write denies under Plan without touching the grant's call count,
/// then allows under Build and counts exactly that call.
#[test]
fn a_grant_does_not_rescue_a_write_under_plan() {
    let plan = with_mode(ApprovalPolicy::Ask, AgentMode::Plan);
    assert!(plan.record(ApprovalChoice::AllowSession {
        token: "workspace:write".into(),
    }));
    assert_eq!(
        plan.resolve(&write_shaped(), Some("workspace:write")),
        ApprovalDecision::Deny { reason: None },
        "the grant cannot move Plan's write denial"
    );
    assert_eq!(
        plan.grants().calls("workspace:write"),
        0,
        "the denied call records nothing under its token"
    );
    let build = plan.with_agent_mode(AgentMode::Build);
    assert_eq!(
        build.resolve(&write_shaped(), Some("workspace:write")),
        ApprovalDecision::Allow,
        "the same grant allows the same call back under Build"
    );
    assert_eq!(
        build.grants().calls("workspace:write"),
        1,
        "only the Build-allowed call counts"
    );
}

/// Plan + frozen + a seed covering the write still denies: the seed cannot
/// answer for a write Plan refuses before the approval match runs.
#[test]
fn a_frozen_plan_denies_a_seeded_write() {
    let policy = with_frozen_mode(
        SessionPolicy::frozen(ApprovalPolicy::Ask, &["workspace:write".to_owned()]),
        AgentMode::Plan,
    );
    assert_eq!(
        policy.resolve(&write_shaped(), Some("workspace:write")),
        ApprovalDecision::Deny { reason: None },
        "Plan denies the seeded write the frozen ask would have allowed"
    );
    assert_eq!(
        policy.grants().calls("workspace:write"),
        0,
        "the denied call records nothing under its seed"
    );
}
