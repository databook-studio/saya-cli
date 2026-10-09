/// `/mode` mirrors `/approvals`: bare reports without changing, a value
/// switches, and the answer always names the current posture. Switching
/// back to build restores the Build surface the status line reads. Both
/// answers state the bypass composition verbatim — under `bypass`, Plan
/// still denies writes.
#[test]
fn mode_reports_switches_and_restores() {
    use crate::slash::PLAN_BYPASS_SENTENCE;
    use saya_agent::AgentMode;
    let mut state = SessionState::new("test", None, "gpt-4o");
    // Bare reports the default without changing it.
    let SessionAction::Message(report) = state.apply(SlashCommand::Mode(None), &[]) else {
        panic!("expected SessionAction::Message");
    };
    assert_eq!(
        report,
        format!("Mode: build — writes allowed, asks per approval policy. {PLAN_BYPASS_SENTENCE}")
    );
    assert_eq!(state.agent_mode, "build");
    // Switching to plan answers plan and sticks.
    let SessionAction::Message(switched) =
        state.apply(SlashCommand::Mode(Some(AgentMode::Plan)), &[])
    else {
        panic!("expected SessionAction::Message");
    };
    assert_eq!(
        switched,
        format!(
            "Mode: plan — read-only; write-shaped tools are hidden and refuse. {PLAN_BYPASS_SENTENCE}"
        )
    );
    assert_eq!(state.agent_mode, "plan");
    // Bare now reports plan.
    let SessionAction::Message(again) = state.apply(SlashCommand::Mode(None), &[]) else {
        panic!("expected SessionAction::Message");
    };
    assert_eq!(
        again,
        format!(
            "Mode: plan — read-only; write-shaped tools are hidden and refuse. {PLAN_BYPASS_SENTENCE}"
        )
    );
    // Switching back restores the Build surface.
    state.apply(SlashCommand::Mode(Some(AgentMode::Build)), &[]);
    assert_eq!(state.agent_mode_parsed(), AgentMode::Build);
    let SessionAction::Message(back) = state.apply(SlashCommand::Mode(None), &[]) else {
        panic!("expected SessionAction::Message");
    };
    assert_eq!(
        back,
        format!("Mode: build — writes allowed, asks per approval policy. {PLAN_BYPASS_SENTENCE}")
    );
}

/// Both `/mode` answers assert the bypass composition sentence verbatim —
/// the shared constant, not a copy that can drift.
#[test]
fn mode_answers_state_the_bypass_composition_verbatim() {
    use crate::slash::PLAN_BYPASS_SENTENCE;
    use saya_agent::AgentMode;
    let mut state = SessionState::new("test", None, "gpt-4o");
    let SessionAction::Message(build) =
        state.apply(SlashCommand::Mode(Some(AgentMode::Build)), &[])
    else {
        panic!("expected SessionAction::Message");
    };
    let SessionAction::Message(plan) = state.apply(SlashCommand::Mode(Some(AgentMode::Plan)), &[])
    else {
        panic!("expected SessionAction::Message");
    };
    assert!(
        build.contains(PLAN_BYPASS_SENTENCE),
        "build answer must carry the bypass sentence verbatim: {build}"
    );
    assert!(
        plan.contains(PLAN_BYPASS_SENTENCE),
        "plan answer must carry the bypass sentence verbatim: {plan}"
    );
}

/// `/run …` from a Plan session refuses — a run writes through its
/// own declared capabilities rather than the session's mode, so a Plan
/// session that spawned one would step around the posture it just set.
/// The refusal names the mode and the remedy (`/mode build`), never
/// silently narrowing the run, and never silently allowing it. Cancelling
/// a run and listing runs are read-shaped and stay available.
#[test]
fn run_from_a_plan_session_refuses_with_mode_and_remedy() {
    use saya_agent::AgentMode;
    let mut state = SessionState::new("test", None, "gpt-4o");
    state.apply(SlashCommand::Mode(Some(AgentMode::Plan)), &[]);
    let action = state.apply(
        SlashCommand::Run("survey the data --allow workspace-write".into()),
        &[],
    );
    let SessionAction::Error(message) = action else {
        panic!("a Plan session must refuse /run, got {action:?}");
    };
    assert!(
        message.contains("Plan mode"),
        "the refusal names the mode, got:\n{message}"
    );
    assert!(
        message.contains("/mode build"),
        "the refusal names the remedy, got:\n{message}"
    );
    // `/runs` and `/run cancel` are read-shaped: still available.
    let action = state.apply(SlashCommand::Runs(None), &[]);
    assert_eq!(action, SessionAction::Runs(None));
    let action = state.apply(SlashCommand::RunCancel("r-1".into()), &[]);
    assert_eq!(action, SessionAction::RunCancel("r-1".into()));
}

/// `/mode build` then `/run …` works exactly as today: the guard reads
/// only Plan, so a Build session's `/run` family is byte-identical to
/// `release/0.4.1` on every path.
#[test]
fn run_after_mode_build_matches_today() {
    use saya_agent::AgentMode;
    let mut state = SessionState::new("test", None, "gpt-4o");
    state.apply(SlashCommand::Mode(Some(AgentMode::Plan)), &[]);
    state.apply(SlashCommand::Mode(Some(AgentMode::Build)), &[]);
    let action = state.apply(SlashCommand::Run("survey the data".into()), &[]);
    assert_eq!(action, SessionAction::Run("survey the data".into()));
    let action = state.apply(SlashCommand::Runs(Some("r-1".into())), &[]);
    assert_eq!(action, SessionAction::Runs(Some("r-1".into())));
    let action = state.apply(SlashCommand::RunCancel("r-1".into()), &[]);
    assert_eq!(action, SessionAction::RunCancel("r-1".into()));
}
