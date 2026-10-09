//! Contract tests for the run types in `saya_types::run`.
//!
//! The plan validator is the load-bearing surface: a plan is model-proposed,
//! untrusted input the engine binds, so every bound the design names —
//! capabilities inside the approved scopes, step budgets within the run's
//! remaining, goal bytes — must be rejected here, including on values that
//! arrive through deserialization rather than the validating constructors.

use std::collections::BTreeMap;
use std::time::Duration;

use saya_types::{
    Budgets, Capabilities, Deliverable, Destination, EndpointBindings, FetchScope,
    InterpreterScope, MAX_GOAL_BYTES, OutputHint, PauseReason, RunContractError, RunEvent,
    RunFailureCode, RunId, RunPlan, RunSpec, RunnerScope, StepSpec,
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

#[path = "run_contracts/budgets.rs"]
mod budgets;
#[path = "run_contracts/capabilities.rs"]
mod capabilities;
#[path = "run_contracts/event_compatibility.rs"]
mod event_compatibility;
#[path = "run_contracts/event_property.rs"]
mod event_property;
#[path = "run_contracts/event_wire.rs"]
mod event_wire;
#[path = "run_contracts/identifiers.rs"]
mod identifiers;
#[path = "run_contracts/plan_validation.rs"]
mod plan_validation;
#[path = "run_contracts/spec.rs"]
mod spec;
