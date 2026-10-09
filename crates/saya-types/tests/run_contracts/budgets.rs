use super::*;

// ---------------------------------------------------------------------------
// Budgets: step budget must stay within the run's remaining
// ---------------------------------------------------------------------------

#[test]
fn budget_is_within_allows_unset_step_dimensions_against_a_ceiling() {
    let mut step_budget = Budgets::default();
    step_budget.turns = Some(10);
    assert!(step_budget.is_within(&run_budgets()));
}

#[test]
fn budget_is_within_allows_any_step_value_when_the_ceiling_is_unset() {
    let mut step_budget = Budgets::default();
    step_budget.turns = Some(u64::MAX);
    assert!(step_budget.is_within(&Budgets::default()));
}

#[test]
fn budget_is_within_rejects_each_dimension_above_the_ceiling() {
    let mut turns = Budgets::default();
    turns.turns = Some(201);
    assert!(!turns.is_within(&run_budgets()));

    let mut tool_calls = Budgets::default();
    tool_calls.tool_calls = Some(501);
    assert!(!tool_calls.is_within(&run_budgets()));

    let mut wall_clock = Budgets::default();
    wall_clock.wall_clock = Some(Duration::from_secs(3601));
    assert!(!wall_clock.is_within(&run_budgets()));

    let mut tokens = Budgets::default();
    tokens.tokens_per_endpoint = BTreeMap::from([("deepseek".to_string(), 1_000_001)]);
    assert!(!tokens.is_within(&run_budgets()));
}

#[test]
fn budgets_round_trip_through_serde() {
    let json = serde_json::to_string(&run_budgets()).unwrap();
    let back: Budgets = serde_json::from_str(&json).unwrap();
    assert_eq!(run_budgets(), back);
}
