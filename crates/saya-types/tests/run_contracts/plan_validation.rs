use super::*;

// ---------------------------------------------------------------------------
// Plan validation: the engine's binding-time gate
// ---------------------------------------------------------------------------

#[test]
fn plan_rejects_no_steps() {
    assert!(matches!(
        RunPlan::new(Vec::new()),
        Err(RunContractError::EmptyPlan)
    ));
}

#[test]
fn plan_accepts_two_hundred_steps() {
    let steps: Vec<StepSpec> = (0..200).map(|i| step(&format!("step {i}"))).collect();
    plan(steps)
        .validate(&approved_scopes(), &run_budgets())
        .expect("a 200-step plan validates: length alone never refuses");
}

#[test]
fn plan_accepts_one_thousand_steps() {
    let steps: Vec<StepSpec> = (0..1000).map(|i| step(&format!("step {i}"))).collect();
    plan(steps)
        .validate(&approved_scopes(), &run_budgets())
        .expect("a 1000-step plan validates: length alone never refuses");
}

#[test]
fn plan_validates_a_well_scoped_plan() {
    let mut first = step("profile the tables");
    first.capabilities.scratch = true;
    first.endpoint = Some("target".to_string());
    let mut first_budget = Budgets::default();
    first_budget.turns = Some(20);
    first.budget = Some(first_budget);
    let second = step("write the report");
    plan(vec![first, second])
        .validate(&approved_scopes(), &run_budgets())
        .unwrap();
}

#[test]
fn plan_rejects_a_capability_outside_the_approved_scopes() {
    let mut rogue = step("escape the scopes");
    rogue.capabilities.scratch = true;
    let error = plan(vec![rogue])
        .validate(&Capabilities::default(), &run_budgets())
        .unwrap_err();
    assert!(matches!(error, RunContractError::CapabilityNotApproved(0)));

    let mut fetcher = step("fetch somewhere undeclared");
    fetcher.capabilities.fetch = Some(
        FetchScope::new(vec![Destination::new("https", "internal.example").unwrap()]).unwrap(),
    );
    assert!(matches!(
        plan(vec![fetcher]).validate(&approved_scopes(), &run_budgets()),
        Err(RunContractError::CapabilityNotApproved(0))
    ));

    let mut sheller = step("run an unapproved program");
    sheller.capabilities.runner = Some(RunnerScope::new(vec!["bash".to_string()]).unwrap());
    assert!(matches!(
        plan(vec![sheller]).validate(&approved_scopes(), &run_budgets()),
        Err(RunContractError::CapabilityNotApproved(0))
    ));

    let mut writer = step("write without the workspace scope");
    writer.capabilities.workspace_write = true;
    assert!(matches!(
        plan(vec![writer]).validate(&Capabilities::default(), &run_budgets()),
        Err(RunContractError::CapabilityNotApproved(0))
    ));
}

#[test]
fn plan_rejects_a_step_budget_above_the_runs_remaining() {
    let mut greedy = step("spend more than the run has");
    let mut greedy_budget = Budgets::default();
    greedy_budget.turns = Some(201);
    greedy.budget = Some(greedy_budget);
    let error = plan(vec![greedy])
        .validate(&approved_scopes(), &run_budgets())
        .unwrap_err();
    assert!(matches!(error, RunContractError::StepBudgetExceeded(0)));
}

#[test]
fn plan_rejects_a_step_goal_over_the_byte_bound() {
    let long_goal = "a".repeat(MAX_GOAL_BYTES + 1);
    // Constructed through deserialization, bypassing the validating
    // constructor: the validator itself must catch the over-long goal.
    let step_json = format!(
        r#"{{"goal":"{long_goal}","capabilities":{{}},"budget":null,"expects":[],"endpoint":null}}"#
    );
    let rogue: StepSpec = serde_json::from_str(&step_json).unwrap();
    let error = plan(vec![rogue])
        .validate(&approved_scopes(), &run_budgets())
        .unwrap_err();
    assert!(matches!(error, RunContractError::GoalTooLong));
}

#[test]
fn plan_rejects_a_step_endpoint_role_that_is_not_bound() {
    let mut bound = step("call an unknown role");
    bound.endpoint = Some("reviewer".to_string());
    let error = plan(vec![bound])
        .validate(&approved_scopes(), &run_budgets())
        .unwrap_err();
    assert!(matches!(error, RunContractError::EndpointNotBound(0)));
}

#[test]
fn plan_rejects_a_hint_name_that_would_resolve_outside_the_workspace() {
    // Built through deserialization, bypassing the validating constructor:
    // the binding gate itself must catch a hint name that would resolve
    // outside the workspace — never bind it, never let it reach a step.
    let step_json = r#"{"goal":"exfiltrate","capabilities":{},"budget":null,"expects":[{"name":"../sentinel.txt"}],"endpoint":null}"#;
    let rogue: StepSpec = serde_json::from_str(step_json).unwrap();
    let error = plan(vec![rogue])
        .validate(&approved_scopes(), &run_budgets())
        .unwrap_err();
    assert!(matches!(error, RunContractError::InvalidOutputHintName(0)));
}

/// The interpreter step's credential refusal (the interpreter approval's
/// design §5): a step whose capabilities include the interpreter scope
/// declares no credentials — model-authored code can encode, split, and
/// reverse a declared credential, so redaction's adversary against an
/// interpreter child is deliberate, not incidental. Refused at plan-bind,
/// the way an unbound endpoint role is.
#[test]
fn plan_rejects_credentials_declared_beside_an_interpreter_scope() {
    let mut interpreter_scopes = Capabilities::default();
    interpreter_scopes_set(&mut interpreter_scopes);
    let mut credentialed = step("score the fetched benchmark");
    credentialed.capabilities.interpreter = interpreter_scopes.interpreter.clone();
    credentialed.credentials = vec!["api_token".to_string()];
    let error = plan(vec![credentialed])
        .validate(&interpreter_scopes, &run_budgets())
        .unwrap_err();
    assert!(
        matches!(error, RunContractError::CredentialsWithInterpreter(0)),
        "the interpreter step's credentials must refuse at plan-bind: {error:?}"
    );

    // The same step without the credentials binds: the refusal is the
    // combination, not the interpreter scope itself.
    let mut clean = step("score the fetched benchmark");
    clean.capabilities.interpreter = interpreter_scopes.interpreter.clone();
    plan(vec![clean])
        .validate(&interpreter_scopes, &run_budgets())
        .expect("an interpreter step declaring no credentials binds");
}

/// An approved interpreter scope: the grammar's own family, one refusal-list
/// name.
fn interpreter_scopes_set(scopes: &mut Capabilities) {
    scopes.interpreter = Some(InterpreterScope::new(vec!["python3".to_string()]).unwrap());
}

#[test]
fn plan_round_trips_through_serde() {
    let mut first = step("profile the tables");
    first.capabilities.scratch = true;
    first.endpoint = Some("target".to_string());
    let mut first_budget = Budgets::default();
    first_budget.turns = Some(20);
    first.budget = Some(first_budget);
    let second = StepSpec::new(
        "write the report",
        Capabilities::default(),
        None,
        vec![
            OutputHint::new("report.md", Some("the quality survey")).unwrap(),
            OutputHint::new("findings.csv", None).unwrap(),
        ],
        None,
    )
    .unwrap();
    let value = plan(vec![first, second]);
    let json = serde_json::to_string(&value).unwrap();
    let back: RunPlan = serde_json::from_str(&json).unwrap();
    assert_eq!(value, back);
}

#[test]
fn plan_rejects_an_output_hint_that_is_not_a_plain_artifact_name() {
    assert!(OutputHint::new("../escape.md", None).is_err());
    assert!(OutputHint::new("a/b.md", None).is_err());
    assert!(OutputHint::new("", None).is_err());
    assert!(OutputHint::new("report\u{0007}.md", None).is_err());
}
