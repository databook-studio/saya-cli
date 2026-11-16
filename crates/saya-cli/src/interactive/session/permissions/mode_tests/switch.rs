use saya_agent::{
    ApprovalDecider, ApprovalDecision, ApprovalPolicy, LocalStateEffect, PermissionMode,
    ToolEffect, ToolExecutor, WORKSPACE_WRITE_TOKEN,
};
use saya_store::JournalEvent;
use std::sync::Arc;

use super::super::{BYPASS_REFUSAL, name, switch};
use super::support::{Approves, Fixture};

/// The `write`/`edit` family's shape: a workspace file write, needing an
/// approval and carrying the token the mode's pre-answer names.
fn workspace_write() -> ToolEffect {
    ToolEffect {
        database_data: false,
        external_side_effect: false,
        requires_approval: true,
        local_state: LocalStateEffect::WriteWorkspace,
    }
}

/// A call outside the workspace file family — a shell or fetch shape — with
/// the same effect bits, so the two differ only in their grant token.
fn outside_the_family() -> ToolEffect {
    ToolEffect {
        database_data: false,
        external_side_effect: true,
        requires_approval: true,
        local_state: LocalStateEffect::None,
    }
}

/// `/mode accept-edits` sets the pair and moves the session's one policy, so
/// the next turn's decider pre-answers the workspace file family and still
/// asks for everything else.
#[test]
fn a_switch_to_accept_edits_sets_the_pair_and_pre_answers_workspace_writes() {
    let mut fixture = Fixture::acquire("accept-edits", ApprovalPolicy::Ask, false);
    let message = switch(
        &mut fixture.state,
        &mut fixture.session,
        PermissionMode::AcceptEdits,
    )
    .expect("accept-edits needs no gate");

    assert_eq!(fixture.state.approval_mode, "accept-edits");
    assert_eq!(fixture.state.agent_mode, "build");
    assert!(message.contains("Mode: accept-edits"), "{message}");

    let policy = fixture.session.policy();
    assert_eq!(
        policy.resolve(&workspace_write(), Some(WORKSPACE_WRITE_TOKEN)),
        ApprovalDecision::Allow,
        "the workspace file family runs unasked under accept-edits"
    );
    assert_eq!(
        policy.resolve(&outside_the_family(), Some("runner:report")),
        ApprovalDecision::Ask,
        "everything else still asks"
    );
    assert_eq!(policy.mode(), ApprovalPolicy::AcceptEdits);
}

/// A switch into `plan` leaves the consent posture where it found it — the
/// two axes are orthogonal — and closes the write surface at the engine, not
/// only in the advertisement.
#[test]
fn a_switch_into_plan_keeps_the_consent_and_closes_the_write_surface() {
    let mut fixture = Fixture::acquire("into-plan", ApprovalPolicy::Ask, false);
    switch(
        &mut fixture.state,
        &mut fixture.session,
        PermissionMode::Plan,
    )
    .expect("plan needs no gate");

    assert_eq!(fixture.state.agent_mode, "plan");
    assert_eq!(
        fixture.state.approval_mode, "ask",
        "plan is a task posture: the consent is untouched"
    );
    assert_eq!(
        fixture
            .session
            .policy()
            .resolve(&workspace_write(), Some(WORKSPACE_WRITE_TOKEN)),
        ApprovalDecision::Deny { reason: None },
        "plan denies a write before the approval match, grant or no grant"
    );
}

/// The plan gate is the plan posture's one door, and the switch is what
/// re-arms it. Under `plan` the engine denies a workspace write; the operator's
/// approval of a submitted plan opens it at the engine — not only in the
/// advertisement — and leaving the posture closes it again, because the
/// approval belonged to the plan the operator read.
#[tokio::test]
async fn leaving_plan_re_arms_the_gate() {
    let mut fixture = Fixture::acquire("leave-plan", ApprovalPolicy::Ask, true);
    switch(
        &mut fixture.state,
        &mut fixture.session,
        PermissionMode::Plan,
    )
    .expect("plan needs no gate");
    assert_eq!(
        fixture.session.policy().resolve(&workspace_write(), None),
        ApprovalDecision::Deny { reason: None },
        "the closed gate denies the write the plan posture hides"
    );

    let universe = fixture.session.universe();
    universe.bind_plan_ask(Some(Arc::new(Approves) as Arc<dyn ApprovalDecider>));
    let answer = universe
        .plan_gate()
        .execute(
            "submit_plan",
            serde_json::json!({"plan": "1. change the thing"}),
        )
        .await
        .expect("the gate serves a bounded plan");
    assert_eq!(answer["approved"], true, "{answer}");
    assert!(universe.plan_gate_open(), "the approval opens the gate");
    assert_eq!(
        fixture.session.policy().resolve(&workspace_write(), None),
        ApprovalDecision::Ask,
        "an approved plan opens the write surface at the engine, not only in the advertisement"
    );

    switch(
        &mut fixture.state,
        &mut fixture.session,
        PermissionMode::Ask,
    )
    .expect("ask needs no gate");
    assert!(!universe.plan_gate_open(), "leaving plan re-arms the gate");
}

/// `bypass` is unreachable while the configuration withholds it: the switch
/// refuses, names the key, and changes nothing — no half posture, no
/// activation recorded.
#[test]
fn a_switch_to_bypass_is_refused_without_the_config_key_and_changes_nothing() {
    let mut fixture = Fixture::acquire("bypass-refused", ApprovalPolicy::Ask, false);
    let error = switch(
        &mut fixture.state,
        &mut fixture.session,
        PermissionMode::Bypass,
    )
    .expect_err("bypass is withheld");

    assert_eq!(error, BYPASS_REFUSAL);
    assert!(
        error.contains("allow_bypass"),
        "the refusal names the key: {error}"
    );
    assert_eq!(fixture.state.approval_mode, "ask");
    assert_eq!(fixture.state.agent_mode, "build");
    assert_eq!(
        fixture.session.policy().mode(),
        ApprovalPolicy::Ask,
        "a refused switch leaves the policy where it was"
    );
    assert!(
        !fixture
            .session
            .journal()
            .read()
            .expect("the journal reads")
            .iter()
            .any(|event| matches!(event, JournalEvent::Bypass { .. })),
        "a refused switch records no consent"
    );
}

/// Where the configuration allows it, `/mode bypass` sets the mode, records
/// the consent once, and answers with the activation line — the same seam
/// and the same words the launch and `/approvals bypass` use.
#[test]
fn an_allowed_switch_to_bypass_activates_and_journals_once() {
    let mut fixture = Fixture::acquire("bypass-allowed", ApprovalPolicy::Ask, true);
    let message = switch(
        &mut fixture.state,
        &mut fixture.session,
        PermissionMode::Bypass,
    )
    .expect("the configuration allows bypass");

    assert_eq!(fixture.state.approval_mode, "bypass");
    assert!(message.contains("Mode: bypass"), "{message}");
    assert!(
        message.contains("bypass on: every tool call runs without asking"),
        "the answer carries the activation line: {message}"
    );
    let activations = fixture
        .session
        .journal()
        .read()
        .expect("the journal reads")
        .into_iter()
        .filter(|event| matches!(event, JournalEvent::Bypass { .. }))
        .count();
    assert_eq!(activations, 1, "the switch records exactly one activation");

    // A re-statement of the mode it is already in records nothing new — the
    // `/allow`-over-a-granted-token shape.
    switch(
        &mut fixture.state,
        &mut fixture.session,
        PermissionMode::Bypass,
    )
    .expect("re-stating bypass is allowed");
    let activations = fixture
        .session
        .journal()
        .read()
        .expect("the journal reads")
        .into_iter()
        .filter(|event| matches!(event, JournalEvent::Bypass { .. }))
        .count();
    assert_eq!(activations, 1, "a re-statement is not a new consent");
}

/// The switch's own vocabulary is the one the bar reads: every mode's name
/// round-trips, so a surface cannot spell a mode its own way.
#[test]
fn every_mode_names_itself() {
    for (mode, spelling) in [
        (PermissionMode::Ask, "ask"),
        (PermissionMode::AcceptEdits, "accept-edits"),
        (PermissionMode::Plan, "plan"),
        (PermissionMode::Bypass, "bypass"),
    ] {
        assert_eq!(name(mode), spelling);
    }
}
