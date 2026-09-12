//! Contract tests for the run types in `saya_types::run`.
//!
//! The plan validator is the load-bearing surface: a plan is model-proposed,
//! untrusted input the engine binds, so every bound the design names —
//! capabilities inside the approved scopes, step budgets within the run's
//! remaining, goal bytes — must be rejected here, including on values that
//! arrive through deserialization rather than the validating constructors.

use std::collections::BTreeMap;
use std::time::Duration;

use proptest::prelude::*;

use saya_types::{
    Budgets, Capabilities, Deliverable, Destination, EndpointBindings, FetchScope,
    InterpreterScope, MAX_GOAL_BYTES, MAX_PLAN_STEPS, OutputHint, PauseReason, RunContractError,
    RunEvent, RunFailureCode, RunId, RunPlan, RunSpec, RunnerScope, StepSpec,
};

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

fn run_id() -> RunId {
    RunId::parse("run-abc123").unwrap()
}

fn endpoint_bindings() -> EndpointBindings {
    EndpointBindings::new([
        ("orchestrator".to_string(), "primary".to_string()),
        ("target".to_string(), "deepseek".to_string()),
    ])
    .unwrap()
}

fn approved_scopes() -> Capabilities {
    let mut scopes = Capabilities::default();
    scopes.workspace_write = true;
    scopes.scratch = true;
    scopes.fetch = Some(
        FetchScope::new(vec![
            Destination::new("https", "example.com").unwrap(),
            Destination::new("https", "data.example.org").unwrap(),
        ])
        .unwrap(),
    );
    scopes.runner = Some(RunnerScope::new(vec!["python3".to_string()]).unwrap());
    scopes.endpoints = endpoint_bindings();
    scopes
}

fn run_budgets() -> Budgets {
    let mut budgets = Budgets::default();
    budgets.wall_clock = Some(Duration::from_secs(3600));
    budgets.turns = Some(200);
    budgets.tool_calls = Some(500);
    budgets.tokens_per_endpoint = BTreeMap::from([("deepseek".to_string(), 1_000_000)]);
    budgets
}

fn run_spec() -> RunSpec {
    RunSpec::new(
        run_id(),
        "survey data quality",
        approved_scopes(),
        run_budgets(),
    )
    .unwrap()
}

fn step(goal: &str) -> StepSpec {
    StepSpec::new(goal, Capabilities::default(), None, Vec::new(), None).unwrap()
}

fn plan(steps: Vec<StepSpec>) -> RunPlan {
    RunPlan::new(steps).unwrap()
}

// ---------------------------------------------------------------------------
// RunId
// ---------------------------------------------------------------------------

#[test]
fn run_id_accepts_identifier_shape() {
    let id = RunId::parse("run-2026-09-10_a1").unwrap();
    assert_eq!(id.as_str(), "run-2026-09-10_a1");
}

#[test]
fn run_id_rejects_empty_oversize_or_fancy_characters() {
    assert!(matches!(
        RunId::parse(""),
        Err(RunContractError::InvalidRunId)
    ));
    assert!(matches!(
        RunId::parse(&"a".repeat(129)),
        Err(RunContractError::InvalidRunId)
    ));
    assert!(matches!(
        RunId::parse("run abc"),
        Err(RunContractError::InvalidRunId)
    ));
    assert!(matches!(
        RunId::parse("run/../etc"),
        Err(RunContractError::InvalidRunId)
    ));
}

#[test]
fn run_id_round_trips_through_serde() {
    let id = run_id();
    let json = serde_json::to_string(&id).unwrap();
    let back: RunId = serde_json::from_str(&json).unwrap();
    assert_eq!(id, back);
}

#[test]
fn run_id_serde_rejects_invalid_shape() {
    let result: Result<RunId, _> = serde_json::from_str("\"run abc\"");
    assert!(result.is_err());
}

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

// ---------------------------------------------------------------------------
// Capabilities: subset semantics and serde
// ---------------------------------------------------------------------------

#[test]
fn capabilities_subset_requires_each_declared_scope_to_be_approved() {
    let approved = approved_scopes();

    let mut workspace = Capabilities::default();
    workspace.workspace_write = true;
    assert!(workspace.is_subset_of(&approved));

    let mut unapproved_workspace = Capabilities::default();
    unapproved_workspace.workspace_write = true;
    assert!(Capabilities::default().is_subset_of(&unapproved_workspace));

    let mut scratch = Capabilities::default();
    scratch.scratch = true;
    assert!(scratch.is_subset_of(&approved));
    assert!(!scratch.is_subset_of(&Capabilities::default()));

    let mut unapproved_destination = Capabilities::default();
    unapproved_destination.fetch = Some(
        FetchScope::new(vec![Destination::new("https", "internal.example").unwrap()]).unwrap(),
    );
    assert!(!unapproved_destination.is_subset_of(&approved));

    let mut subset_destination = Capabilities::default();
    subset_destination.fetch =
        Some(FetchScope::new(vec![Destination::new("https", "example.com").unwrap()]).unwrap());
    assert!(subset_destination.is_subset_of(&approved));

    let mut unapproved_program = Capabilities::default();
    unapproved_program.runner = Some(RunnerScope::new(vec!["bash".to_string()]).unwrap());
    assert!(!unapproved_program.is_subset_of(&approved));

    let mut unapproved_endpoint = Capabilities::default();
    unapproved_endpoint.endpoints =
        EndpointBindings::new([("target".to_string(), "unbound-endpoint".to_string())]).unwrap();
    assert!(!unapproved_endpoint.is_subset_of(&approved));
}

/// `missing_from` is `is_subset_of`'s message, not a second opinion: it is
/// empty exactly when the subset holds, and every token it names is in the
/// `--allow` grammar the CLI renders refusals with.
#[test]
fn missing_from_is_empty_exactly_when_the_subset_holds_and_names_the_scope() {
    let approved = approved_scopes();
    let mut workspace = Capabilities::default();
    workspace.workspace_write = true;
    assert!(workspace.missing_from(&approved).is_empty());
    assert!(Capabilities::default().missing_from(&approved).is_empty());
    assert!(approved.missing_from(&approved).is_empty());

    let mut scratch = Capabilities::default();
    scratch.scratch = true;
    assert_eq!(
        scratch.missing_from(&Capabilities::default()),
        vec!["scratch"]
    );

    let mut fetch = Capabilities::default();
    fetch.fetch = Some(
        FetchScope::new(vec![
            Destination::new("https", "unapproved.example").unwrap(),
        ])
        .unwrap(),
    );
    assert_eq!(
        fetch.missing_from(&approved),
        vec!["fetch:https+unapproved.example"]
    );

    let mut runner = Capabilities::default();
    runner.runner = Some(RunnerScope::new(vec!["bash".to_string()]).unwrap());
    assert_eq!(runner.missing_from(&approved), vec!["runner:bash"]);

    let mut endpoint = Capabilities::default();
    endpoint.endpoints =
        EndpointBindings::new([("target".to_string(), "unbound-endpoint".to_string())]).unwrap();
    assert_eq!(
        endpoint.missing_from(&approved),
        vec!["endpoint:target=unbound-endpoint"]
    );

    // A step asking for several scopes at once names each one that is
    // missing, and nothing that was approved.
    let mut mixed = Capabilities::default();
    mixed.workspace_write = true;
    mixed.runner = Some(RunnerScope::new(vec!["bash".to_string()]).unwrap());
    let missing = mixed.missing_from(&approved);
    assert_eq!(missing, vec!["runner:bash"]);
}

/// The property the equivalence name promises, over every shape the subset
/// test exercises: `missing_from` is empty exactly when `is_subset_of` holds.
#[test]
fn missing_from_agrees_with_is_subset_of_on_every_shape() {
    let approved = approved_scopes();
    let shapes: Vec<Capabilities> = vec![
        Capabilities::default(),
        approved.clone(),
        {
            let mut c = Capabilities::default();
            c.workspace_write = true;
            c
        },
        {
            let mut c = Capabilities::default();
            c.scratch = true;
            c
        },
        {
            let mut c = Capabilities::default();
            c.fetch = Some(
                FetchScope::new(vec![Destination::new("https", "internal.example").unwrap()])
                    .unwrap(),
            );
            c
        },
        {
            let mut c = Capabilities::default();
            c.fetch = Some(
                FetchScope::new(vec![Destination::new("https", "example.com").unwrap()]).unwrap(),
            );
            c
        },
        {
            let mut c = Capabilities::default();
            c.runner = Some(RunnerScope::new(vec!["bash".to_string()]).unwrap());
            c
        },
        {
            let mut c = Capabilities::default();
            c.endpoints =
                EndpointBindings::new([("target".to_string(), "unbound-endpoint".to_string())])
                    .unwrap();
            c
        },
        {
            let mut c = Capabilities::default();
            c.endpoints = endpoint_bindings();
            c
        },
    ];
    for requested in &shapes {
        assert_eq!(
            requested.missing_from(&approved).is_empty(),
            requested.is_subset_of(&approved),
            "missing_from must agree with is_subset_of for {requested:?}"
        );
    }
}

#[test]
fn capabilities_round_trip_through_serde() {
    let json = serde_json::to_string(&approved_scopes()).unwrap();
    let back: Capabilities = serde_json::from_str(&json).unwrap();
    assert_eq!(approved_scopes(), back);
}

#[test]
fn endpoint_bindings_reject_unshaped_names() {
    assert!(EndpointBindings::new([("or chestrator".to_string(), "a".to_string())]).is_err());
    assert!(EndpointBindings::new([("orchestrator".to_string(), String::new())]).is_err());
}

// ---------------------------------------------------------------------------
// Plan validation: the engine's binding-time gate
// ---------------------------------------------------------------------------

#[test]
fn plan_rejects_no_steps_or_too_many() {
    assert!(matches!(
        RunPlan::new(Vec::new()),
        Err(RunContractError::EmptyPlan)
    ));
    let steps: Vec<StepSpec> = (0..=MAX_PLAN_STEPS).map(|_| step("one step")).collect();
    assert!(matches!(
        RunPlan::new(steps),
        Err(RunContractError::TooManySteps)
    ));
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

// ---------------------------------------------------------------------------
// RunEvent: serde-tagged, one line, and usage absence is "unknown", not zero
// ---------------------------------------------------------------------------

#[test]
fn usage_event_serializes_absence_as_null_never_zero() {
    let event = RunEvent::Usage {
        endpoint: "deepseek".to_string(),
        tokens: None,
        turns: None,
        tool_calls: None,
        cached_input_tokens: None,
        cache_creation_input_tokens: None,
    };
    let json = serde_json::to_string(&event).unwrap();
    assert!(
        json.contains("\"tokens\":null"),
        "absence must stay null: {json}"
    );
    assert!(
        json.contains("\"turns\":null"),
        "absence must stay null: {json}"
    );
    assert!(
        json.contains("\"cached_input_tokens\":null"),
        "an unreported cache figure must stay null: {json}"
    );
    assert!(
        json.contains("\"cache_creation_input_tokens\":null"),
        "an unreported cache-write figure must stay null: {json}"
    );
    assert!(!json.contains("\"tokens\":0"));
    assert!(!json.contains("\"cached_input_tokens\":0"));

    let reported = RunEvent::Usage {
        endpoint: "deepseek".to_string(),
        tokens: Some(0),
        turns: Some(3),
        tool_calls: Some(7),
        cached_input_tokens: Some(0),
        cache_creation_input_tokens: Some(0),
    };
    let json = serde_json::to_string(&reported).unwrap();
    assert!(
        json.contains("\"tokens\":0"),
        "a reported zero stays 0: {json}"
    );
    assert!(
        json.contains("\"cached_input_tokens\":0"),
        "a reported cache zero stays 0, never unknown: {json}"
    );

    let back: RunEvent = serde_json::from_str(&json).unwrap();
    assert_eq!(reported, back);

    // A journal line written before the cache fields existed still parses:
    // the fields default to "unreported", never to a reported zero.
    let legacy = r#"{"type":"usage","endpoint":"deepseek","tokens":120,"turns":1,"tool_calls":2}"#;
    let old: RunEvent = serde_json::from_str(legacy).unwrap();
    assert_eq!(
        old,
        RunEvent::Usage {
            endpoint: "deepseek".to_string(),
            tokens: Some(120),
            turns: Some(1),
            tool_calls: Some(2),
            cached_input_tokens: None,
            cache_creation_input_tokens: None,
        },
        "an old journal line must read as unreported cache figures, not zeros"
    );
}

#[test]
fn plan_approved_journal_line_without_scopes_replays_as_scopes_unstated() {
    // A journal written before the `scopes` payload existed carries no
    // `scopes` field; `#[serde(default)]` keeps it parseable as an empty
    // list — "scopes unstated here" — so old runs resume unchanged.
    let legacy = r#"{"type":"plan_approved"}"#;
    let old: RunEvent = serde_json::from_str(legacy).unwrap();
    assert_eq!(
        old,
        RunEvent::PlanApproved { scopes: vec![] },
        "an old PlanApproved line must replay as scopes unstated, not fail to parse"
    );

    // And the round trip of a modern line keeps the tokens verbatim, in
    // declaration order — the journal is the authority a resume re-grants
    // from, so the field must survive the round trip exactly.
    let approved = RunEvent::PlanApproved {
        scopes: vec!["interpreter:python3".to_string()],
    };
    let json = serde_json::to_string(&approved).unwrap();
    let back: RunEvent = serde_json::from_str(&json).unwrap();
    assert_eq!(approved, back);
    assert!(
        json.contains(r#""scopes":["interpreter:python3"]"#),
        "the journal line carries the approved scopes verbatim: {json}"
    );
}

#[test]
fn every_event_variant_round_trips_through_serde() {
    let events = vec![
        RunEvent::RunStarted,
        RunEvent::PlanApproved { scopes: vec![] },
        RunEvent::PlanApproved {
            scopes: vec![
                "fetch:https+example.com".to_string(),
                "interpreter:python3".to_string(),
            ],
        },
        RunEvent::StepStarted { step: 0 },
        RunEvent::StepCompleted { step: 0 },
        RunEvent::StepFailed { step: 0 },
        RunEvent::Deliverables {
            step: 0,
            entries: vec![
                Deliverable::present("report.md", 29, "4634".to_string()),
                Deliverable::missing("draft.md"),
            ],
        },
        RunEvent::Paused {
            reason: PauseReason::BudgetExhausted,
        },
        RunEvent::Paused {
            reason: PauseReason::WallClockExceeded,
        },
        RunEvent::Completed,
        RunEvent::Failed {
            code: RunFailureCode::SafetyQuery,
        },
        RunEvent::Failed {
            code: RunFailureCode::Provider,
        },
        RunEvent::Failed {
            code: RunFailureCode::ConnectionConfig,
        },
        RunEvent::Cancelled,
        RunEvent::Usage {
            endpoint: "deepseek".to_string(),
            tokens: Some(120),
            turns: Some(1),
            tool_calls: Some(2),
            cached_input_tokens: Some(60),
            cache_creation_input_tokens: None,
        },
        RunEvent::DownloadedBytes { bytes: 0 },
        RunEvent::DownloadedBytes { bytes: 97 },
    ];
    for event in events {
        let json = serde_json::to_string(&event).unwrap();
        let back: RunEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(event, back, "round trip failed for {json}");
    }
}

proptest! {
    /// Every `RunEvent` serializes to exactly one NDJSON line — the string
    /// `serde_json` produces never contains a raw newline — and every line
    /// parses back to the event it came from. The run journal and the
    /// `--format ndjson` wire both depend on this.
    #[test]
    fn event_is_one_line_and_round_trips(
        endpoint in "[a-z0-9_-]{1,32}",
        tokens in prop::option::of(0u64..1_000_000),
        turns in prop::option::of(0u64..1_000),
        tool_calls in prop::option::of(0u64..1_000),
        cached in prop::option::of(0u64..1_000_000),
        cache_creation in prop::option::of(0u64..1_000_000),
        step_index in 0usize..64,
        reason in prop::sample::select(vec![
            PauseReason::BudgetExhausted,
            PauseReason::WallClockExceeded,
            PauseReason::StepFailedAfterRetry,
            PauseReason::StoreUnavailable,
            PauseReason::ProcessDeath,
            PauseReason::UserPaused,
        ]),
        code in prop::sample::select(vec![
            RunFailureCode::SafetyQuery,
            RunFailureCode::Provider,
            RunFailureCode::ConnectionConfig,
        ]),
        bytes in 0u64..1_000_000_000,
        kind in prop::sample::select(vec![0u8, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11]),
    ) {
        let event = match kind {
            0 => RunEvent::RunStarted,
            1 => RunEvent::PlanApproved {
                scopes: vec![format!("fetch:https+{endpoint}")],
            },
            2 => RunEvent::StepStarted { step: step_index },
            3 => RunEvent::StepCompleted { step: step_index },
            4 => RunEvent::StepFailed { step: step_index },
            5 => RunEvent::Deliverables {
                step: step_index,
                entries: vec![
                    Deliverable::present("report.md", 29, "4634"),
                    Deliverable::missing("draft.md"),
                ],
            },
            6 => RunEvent::Paused { reason },
            7 => RunEvent::Completed,
            8 => RunEvent::Failed { code },
            9 => RunEvent::Cancelled,
            11 => RunEvent::DownloadedBytes { bytes },
            _ => RunEvent::Usage {
                endpoint: endpoint.clone(),
                tokens,
                turns,
                tool_calls,
                cached_input_tokens: cached,
                cache_creation_input_tokens: cache_creation,
            },
        };
        let line = serde_json::to_string(&event).unwrap();
        prop_assert!(!line.contains('\n'), "event spanned multiple lines: {line}");
        let back: RunEvent = serde_json::from_str(&line).unwrap();
        prop_assert_eq!(back, event);
    }
}
