//! The database budget arms on the turn's first database use (DB3).
//!
//! One in-process run with the database plugin on the chat surface, over a
//! scripted provider. The budget is `[limits.database] requests = 1`, so it
//! bounds a turn only once the turn spends it: a turn that makes three requests
//! without a database call is answered in full, while a turn whose first call is
//! a database tool has its next request refused with chat's budget wording — and
//! the run stays open for the turn after it.

#![cfg(unix)]

use std::num::NonZeroU64;
use std::sync::Arc;
use std::time::Duration;

use saya_agent::{
    AgentOutput, BudgetDimension, NoopEventSink, PluginRegistry, RunContext, RunSurface,
};
use serde_json::json;

use crate::budget_wording::budget_partial_message;
use crate::commands::coding::parent_turn_fixture::in_place_writing;
use crate::commands::coding::parent_turn_provider::{Reply, Scripted};
use crate::commands::coding::{CodingError, NativeCodingJob, RunPlugins, TurnInputs};
use crate::plugins::database::DatabaseBudget;
use crate::plugins::database::tests::{chat_plugin_over, scratch};

/// Load ceiling, not a product bound: how long one scripted turn may take.
const RUN: Duration = Duration::from_secs(120);

/// The chat's run context: the surface has a database, so the plugin activates.
const CHAT: RunContext = RunContext {
    surface: RunSurface::Chat,
    database_profiles: 1,
};

/// The database tool the turn's first call is routed to the plugin for.
const EXPLORE: &str = "schema_discovery";

/// `[limits.database] requests = 1`: one counted request, so the request after
/// the one a database call answered is the one refused.
fn one_request() -> DatabaseBudget {
    let root = scratch("arming");
    let mut runtime = crate::load_with_sources(
        &crate::GlobalOptions::default(),
        &root,
        &root,
        std::collections::BTreeMap::new(),
    )
    .expect("a runtime resolves from an empty configuration");
    runtime.resolved.limits.database = saya_config::ResolvedAnsweringLimits {
        requests: saya_config::BudgetLimit::Finite(
            NonZeroU64::new(1).expect("a stated ceiling is nonzero"),
        ),
        ..saya_config::ResolvedAnsweringLimits::default()
    };
    DatabaseBudget::of(&runtime)
}

/// The run's turn inputs: one registry holding the database plugin on the chat
/// surface, and nothing else.
fn inputs(budget: DatabaseBudget) -> TurnInputs {
    let plugin = Arc::new(chat_plugin_over(budget));
    TurnInputs {
        policy: None,
        plugins: Some(RunPlugins {
            registry: PluginRegistry::new(vec![plugin], &[])
                .expect("the database plugin declares no name an empty core list owns"),
            context: CHAT,
        }),
        seed: Vec::new(),
    }
}

/// Runs the first turn of a fresh run, bounded so a wedged turn fails the test.
async fn first_turn(
    job: &Arc<NativeCodingJob>,
    inputs: TurnInputs,
) -> Result<AgentOutput, CodingError> {
    tokio::time::timeout(RUN, job.run_open(&NoopEventSink, inputs))
        .await
        .expect("the turn must settle")
}

/// Test 1 — a turn that never uses the database makes three requests and
/// answers normally: its one-request budget is never armed.
#[tokio::test]
async fn an_unarmed_turn_is_not_budgeted() {
    let provider = Scripted::start_with_script(vec![
        Reply::Call {
            name: "list",
            arguments: json!({}),
        },
        Reply::Call {
            name: "list",
            arguments: json!({"path": "."}),
        },
        Reply::Text("the unarmed turn answered"),
    ]);
    let fixture = in_place_writing("db-unarmed", &provider.endpoint).await;

    let output = first_turn(&fixture.job, inputs(one_request()))
        .await
        .expect("a turn that never uses the database answers normally");
    assert_eq!(output.answer, "the unarmed turn answered");
    assert!(
        !output.truncated,
        "no database budget stopped the turn: its answer is whole"
    );
    assert_eq!(
        provider.bodies().len(),
        3,
        "three requests were served: none of them was budgeted"
    );
    assert_eq!(
        fixture.job.turn_stop(),
        None,
        "nothing stopped the turn, so there is no budget wording"
    );
}

/// Test 2 — the turn's first database call arms the budget, and the request
/// after that call is refused: the run stays open and takes another turn.
#[tokio::test]
async fn the_first_db_call_arms_the_budget() {
    let provider = Scripted::start_with_script(vec![
        Reply::Call {
            name: EXPLORE,
            arguments: json!({}),
        },
        Reply::Text("the next turn answered"),
    ]);
    let fixture = in_place_writing("db-armed", &provider.endpoint).await;

    let stopped = first_turn(&fixture.job, inputs(one_request()))
        .await
        .expect("a spent budget is a partial answer, not a run failure");
    assert_eq!(
        provider.bodies().len(),
        1,
        "the request after the database call was refused"
    );
    let stop = fixture
        .job
        .turn_stop()
        .expect("the turn states what stopped it");
    assert_eq!(
        stop.dimension(),
        Some(BudgetDimension::LogicalRequests),
        "the database budget's request ceiling stopped the turn"
    );
    assert_eq!(
        stop.message(),
        budget_partial_message(BudgetDimension::LogicalRequests),
        "the stop keeps chat's budget wording"
    );
    assert!(stopped.truncated, "the answer it produced is partial");

    let next = tokio::time::timeout(RUN, fixture.job.run_next_turn(&NoopEventSink, "again"))
        .await
        .expect("the further turn must settle");
    assert!(next.is_ok(), "a spent budget is per turn: {next:?}");
    assert_eq!(
        provider.bodies().len(),
        2,
        "the further turn reached the provider"
    );
}
