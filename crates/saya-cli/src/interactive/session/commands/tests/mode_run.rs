/// `/mode` mirrors `/approvals`: bare reports without changing, a mode name
/// is handed to the loop as a switch, and the bare answer always names the
/// current posture. Both answers state the bypass composition verbatim —
/// under `bypass`, Plan still denies writes.
#[test]
fn mode_reports_and_hands_a_switch_to_the_loop() {
    use crate::slash::PLAN_BYPASS_SENTENCE;
    use saya_agent::PermissionMode;
    let mut state = SessionState::new("test", None, "gpt-4o");
    // Bare reports the default without changing it.
    let SessionAction::Message(report) = state.apply(SlashCommand::Mode(None), &[]) else {
        panic!("expected SessionAction::Message");
    };
    assert_eq!(
        report,
        format!(
            "Mode: ask — writes allowed, asks per approval policy. {PLAN_BYPASS_SENTENCE}"
        )
    );
    assert_eq!(state.agent_mode, "build");
    assert_eq!(state.approval_mode, "ask");
    // A named mode is a switch, not a state edit here: the loops run the one
    // shared `permissions::mode::switch` over the session's policy.
    assert_eq!(
        state.apply(SlashCommand::Mode(Some(PermissionMode::Plan)), &[]),
        SessionAction::Mode(PermissionMode::Plan)
    );
    assert_eq!(
        state.agent_mode, "build",
        "the pure state layer moves nothing on its own"
    );
    assert_eq!(state.approval_mode, "ask");
    // With the pair set the way the loop's switch sets it, bare reports plan.
    state.agent_mode = "plan".into();
    let SessionAction::Message(again) = state.apply(SlashCommand::Mode(None), &[]) else {
        panic!("expected SessionAction::Message");
    };
    assert!(
        again.starts_with("Mode: plan — read-only;"),
        "bare /mode reports the task posture: {again}"
    );
}

/// Every `/mode` answer states the bypass composition sentence verbatim —
/// the shared constant, not a copy that can drift.
#[test]
fn mode_answers_state_the_bypass_composition_verbatim() {
    use crate::slash::PLAN_BYPASS_SENTENCE;
    use saya_agent::PermissionMode;
    let mut state = SessionState::new("test", None, "gpt-4o");
    for mode in [
        PermissionMode::Ask,
        PermissionMode::AcceptEdits,
        PermissionMode::Plan,
        PermissionMode::Bypass,
    ] {
        state.approval_mode = match mode {
            PermissionMode::AcceptEdits => "accept-edits".into(),
            PermissionMode::Bypass => "bypass".into(),
            _ => "ask".into(),
        };
        state.agent_mode = match mode {
            PermissionMode::Plan => "plan".into(),
            _ => "build".into(),
        };
        let SessionAction::Message(answer) = state.apply(SlashCommand::Mode(None), &[]) else {
            panic!("expected SessionAction::Message");
        };
        assert!(
            answer.contains(PLAN_BYPASS_SENTENCE),
            "the {mode:?} answer must carry the bypass sentence verbatim: {answer}"
        );
    }
}

/// A consent posture with no mode of its own — `/approvals read-only` — is
/// reported as the pair it is, never rounded to one of the four modes.
#[test]
fn a_non_switchable_consent_posture_is_reported_as_the_pair() {
    let mut state = SessionState::new("test", None, "gpt-4o");
    state.approval_mode = "read-only".into();
    let SessionAction::Message(report) = state.apply(SlashCommand::Mode(None), &[]) else {
        panic!("expected SessionAction::Message");
    };
    assert!(
        report.contains("read-only") && report.contains("not one of the switchable modes"),
        "the report states the pair rather than inventing a mode: {report}"
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
    let mut state = SessionState::new("test", None, "gpt-4o");
    state.agent_mode = "plan".into();
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

/// A Build session's `/run` family is byte-identical to an ungated session on
/// every path: the guard reads only Plan.
#[test]
fn run_after_mode_build_matches_today() {
    let mut state = SessionState::new("test", None, "gpt-4o");
    state.agent_mode = "build".into();
    let action = state.apply(SlashCommand::Run("survey the data".into()), &[]);
    assert_eq!(action, SessionAction::Run("survey the data".into()));
    let action = state.apply(SlashCommand::Runs(Some("r-1".into())), &[]);
    assert_eq!(action, SessionAction::Runs(Some("r-1".into())));
    let action = state.apply(SlashCommand::RunCancel("r-1".into()), &[]);
    assert_eq!(action, SessionAction::RunCancel("r-1".into()));
}
