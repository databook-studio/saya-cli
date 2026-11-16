use super::*;

// ---------------------------------------------------------------------------
// RunSpec
// ---------------------------------------------------------------------------

#[test]
fn run_spec_rejects_empty_goal() {
    let error =
        RunSpec::new(run_id(), "", Capabilities::default(), Budgets::default()).unwrap_err();
    assert!(matches!(error, RunContractError::EmptyGoal));
}

#[test]
fn run_spec_rejects_goal_over_the_byte_bound() {
    let goal = "a".repeat(MAX_GOAL_BYTES + 1);
    let error =
        RunSpec::new(run_id(), goal, Capabilities::default(), Budgets::default()).unwrap_err();
    assert!(matches!(error, RunContractError::GoalTooLong));
}

#[test]
fn run_spec_rejects_goal_with_control_characters() {
    let error = RunSpec::new(
        run_id(),
        "goal\u{0007}bell",
        Capabilities::default(),
        Budgets::default(),
    )
    .unwrap_err();
    assert!(matches!(error, RunContractError::GoalControlCharacter));
}

#[test]
fn run_spec_round_trips_through_serde() {
    let spec = run_spec();
    let json = serde_json::to_string(&spec).unwrap();
    let back: RunSpec = serde_json::from_str(&json).unwrap();
    assert_eq!(spec, back);
}
