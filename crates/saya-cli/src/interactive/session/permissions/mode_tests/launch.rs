use saya_agent::{AgentMode, ApprovalPolicy, PermissionMode};

use super::super::{launch_posture, switch};
use super::support::Fixture;

/// A resumed session keeps the task posture the record carries: `plan` is
/// the session's own axis, and nothing in the launch path moves it.
#[test]
fn a_resumed_plan_session_resumes_as_plan() {
    let (approval, agent) = launch_posture("ask", "plan", false, false);
    assert_eq!(approval, "ask");
    assert_eq!(agent, "plan", "the record's task posture rides the resume");
    assert_eq!(
        agent.parse::<AgentMode>().unwrap(),
        AgentMode::Plan,
        "and it parses back to the engine's own type"
    );
}

/// A record that says `bypass` while this process's configuration withholds
/// it degrades to `ask`: the record cannot grant a consent the configuration
/// refuses, and the degradation only ever narrows.
#[test]
fn a_resumed_bypass_session_degrades_to_ask_when_the_config_withholds_it() {
    let (approval, agent) = launch_posture("bypass", "build", false, false);
    assert_eq!(
        approval, "ask",
        "the withheld mode is not restored by the record"
    );
    assert_eq!(agent, "build");
}

/// Two things keep a stated consent: an explicit `--approval-mode` this
/// process made, and a configuration that allows the mode. Neither is
/// overridden by the other.
#[test]
fn a_stated_or_allowed_bypass_rides_the_launch() {
    assert_eq!(
        launch_posture("bypass", "build", true, false).0,
        "bypass",
        "the process stated the consent itself"
    );
    assert_eq!(
        launch_posture("bypass", "build", false, true).0,
        "bypass",
        "the configuration allows the mode"
    );
    assert_eq!(
        launch_posture("never", "build", false, false).0,
        "never",
        "every other posture rides verbatim"
    );
    assert_eq!(
        launch_posture("bogus", "plan", false, false),
        ("bogus".to_owned(), "plan".to_owned()),
        "an unparseable spelling is the record's, and the parse sites fall back"
    );
}

/// The launch's task posture reaches the session's one policy, so a resumed
/// Plan session refuses a write at the engine — not only in the
/// advertisement — from its first turn.
#[test]
fn the_launch_task_posture_reaches_the_policy() {
    let mut fixture = Fixture::acquire("launch-plan", ApprovalPolicy::Ask, false);
    let (_, agent) = launch_posture("ask", "plan", false, false);
    fixture.state.agent_mode = agent;
    fixture
        .session
        .switch_policy(None, fixture.state.agent_mode_parsed());

    let write = saya_agent::ToolEffect {
        database_data: false,
        external_side_effect: false,
        requires_approval: true,
        local_state: saya_agent::LocalStateEffect::WriteWorkspace,
    };
    assert_eq!(
        fixture
            .session
            .policy()
            .resolve(&write, Some(saya_agent::WORKSPACE_WRITE_TOKEN)),
        saya_agent::ApprovalDecision::Deny { reason: None },
        "a resumed plan session denies writes before the approval match"
    );
}

/// A launch that allows bypass reaches the mode through the same vocabulary
/// the switch uses: the pair is what `PermissionMode::Bypass` decomposes to,
/// so the launch and `/mode bypass` cannot land on different postures.
#[test]
fn an_allowed_launch_lands_on_the_same_pair_the_switch_sets() {
    let (approval, agent) = launch_posture("bypass", "build", true, false);
    let (consent, task) = PermissionMode::Bypass.decompose();
    assert_eq!(approval, "bypass");
    assert_eq!(consent, Some(ApprovalPolicy::Bypass));
    assert_eq!(agent, task.as_str());
}

/// The launch's copy of the gate is the field the switch reads, so the two
/// cannot disagree about whether the mode is reachable.
#[test]
fn the_launch_copy_of_the_gate_is_the_switches_own() {
    let mut fixture = Fixture::acquire("launch-gate", ApprovalPolicy::Ask, true);
    assert!(fixture.state.allow_bypass);
    switch(
        &mut fixture.state,
        &mut fixture.session,
        PermissionMode::Bypass,
    )
    .expect("the gate the launch copied is the one the switch reads");
    assert_eq!(fixture.state.approval_mode, "bypass");
}
