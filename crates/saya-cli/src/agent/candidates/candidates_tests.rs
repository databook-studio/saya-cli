//! Tests for the candidate-orchestration core ([`orchestrate`]).
//!
//! Each numbered behaviour from the task has its own test, driven by a scripted
//! [`AttemptRunner`] (returning `AgentOutput`s with nominated SQL, or errors)
//! and a fake [`CandidateExecutor`] keyed by exact SQL — mirroring how
//! `decide_tests.rs` tests the decision layer. No live provider, no database.
//!
//! The orchestration is injectable via [`AttemptRunner`] the way the runtime
//! is injectable via `run_prompt_with_inputs` / `TurnInputs`: the production
//! runner delegates to `run_prompt_with_sink` (which builds `TurnInputs` from
//! config), and these tests inject a scripted runner so the loop, the
//! nomination, the decision, and the `ConsensusDecided` event are assertable in
//! isolation from the runtime (which has its own tests in `runtime_tests.rs`).

use super::super::decide::CandidateExecutor;
use super::super::question_scope::QuestionScope;
use super::super::runtime::AgentRuntimeError;
use super::AttemptRunner;
use crate::{answering_receipts::AnsweringReceiptSink, config::runtime::RuntimeConfig};
use async_trait::async_trait;
use saya_agent::{AgentEvent, AgentEventSink, AgentOutput, CancellationToken, TokenUsage};
use saya_config::{ConnectionsFile, ResolutionInput, resolve};
use saya_types::{QueryResult, SqlDialect};
use serde_json::{Value, json};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::{
    collections::{BTreeMap, HashMap},
    path::PathBuf,
};

use super::orchestrate;

/// Postgres accepts the most syntax, so it is the default dialect here.
const D: SqlDialect = SqlDialect::Postgres;

use saya_agent::{BudgetDimension, CancellationReason};

fn test_scope(cancellation: CancellationToken) -> QuestionScope {
    let connections = ConnectionsFile::default();
    let runtime = RuntimeConfig {
        resolved: resolve(ResolutionInput::new(connections.clone())).expect("test config resolves"),
        connections,
        config_path: None,
        connections_path: None,
        cache_scope: PathBuf::from("/tmp/saya-candidate-tests"),
        investigations_root: PathBuf::from("/tmp/saya-candidate-tests/investigations"),
        secret_values: BTreeMap::new(),
    };
    QuestionScope::new(&runtime, cancellation)
}

async fn orchestrate_test(
    candidates: usize,
    runner: &dyn AttemptRunner,
    executor: &dyn CandidateExecutor,
    sink: &RecordingSink,
    cancellation: &CancellationToken,
) -> Result<AgentOutput, AgentRuntimeError> {
    let scope = test_scope(cancellation.clone());
    let receipts = AnsweringReceiptSink::new(sink);
    orchestrate(
        candidates,
        runner,
        executor,
        D,
        &receipts,
        cancellation,
        &scope,
    )
    .await
}

// ---------------------------------------------------------------------------
// scripted attempt runner
// ---------------------------------------------------------------------------

include!("candidates_tests/attempt_runner.rs");
// ---------------------------------------------------------------------------
// scripted candidate executor
// ---------------------------------------------------------------------------

include!("candidates_tests/executors.rs");
// ---------------------------------------------------------------------------
// result / output builders
// ---------------------------------------------------------------------------

include!("candidates_tests/output_fixtures.rs");
// ===========================================================================
// 1. candidates = 1 runs the agent exactly once and emits no consensus event.
// Prove it by counting, not by inspection.
// ===========================================================================

include!("candidates_tests/consensus_outcomes.rs");

// ===========================================================================
// 7. cancellation between attempts stops early.
// ===========================================================================

include!("candidates_tests/attempt_lifecycle.rs");

// ===========================================================================
// A settled clarification stops candidates: once an attempt's returned output
// ended on a settled ClarificationNeeded, no further attempt may launch, the
// question must not enter verification/vote (the executor is never called —
// the vote segment is the only path that calls it), and no consensus may be
// fabricated for a paused turn. The paused output itself is the answer.
// ===========================================================================

include!("candidates_tests/clarification.rs");

// ===========================================================================
// A budget stop between attempts: the stop-return keeps the last settled
// answer, but the retained output must carry the actual stop — partial
// (`truncated: true`) with the typed `AnsweringBudgetStopped` event naming the
// real dimension — not come back looking like a natural completion. Before the
// repair the returned output has `truncated: false` and no budget-stop event.
// ===========================================================================

include!("candidates_tests/budget_before_and_during_vote.rs");

// ===========================================================================
// Green-side pins for the retained-output stop rules. (These cannot be red:
// two of them pass by accident on the un-repaired code, which never touched
// the output; they pin what the repair must keep true.)
// ===========================================================================

/// An `AgentOutput` the answering loop itself budget-stopped: partial, with
/// its own typed event already in the retained events.
fn budget_stopped_output(sql: &str) -> AgentOutput {
    let mut out = output_with_sql(sql);
    out.truncated = true;
    out.events.push(AgentEvent::answering_budget_stopped(
        BudgetDimension::ElapsedTime,
    ));
    out
}

/// An `AgentOutput` already truncated for a non-budget reason — the legacy
/// truncation shape (e.g. a salvaged partial from a truncated provider
/// response): partial, no typed budget event.
fn legacy_truncated_output(sql: &str) -> AgentOutput {
    let mut out = output_with_sql(sql);
    out.truncated = true;
    out
}

include!("candidates_tests/budget_retention.rs");

include!("candidates_tests/user_stop.rs");
