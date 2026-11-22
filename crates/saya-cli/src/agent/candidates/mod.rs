//! Runs the agent multiple times and decides which answer to believe.
//!
//! Wires [`crate::agent::decide`] into the `ask` path: when `candidates > 1`,
//! the question is asked `candidates` times in sequence — each attempt one
//! turn of its own chat session, on a fresh history — the nominated SQL of each
//! attempt is collected, and [`decide`] votes on result-set fingerprints,
//! emitting one `ConsensusDecided` event and returning the winning attempt's
//! output.
//!
//! [`run_with_candidates`] is the production entry: every attempt, including
//! the overwhelmingly common single one, runs as a one-turn chat session — the
//! same composition `saya ask` opens — so there is no second turn body beside
//! it. The loop, the decision and the emit live in [`orchestrate`], which is
//! injectable for tests via the [`AttemptRunner`] trait (mirroring
//! [`CandidateExecutor`]).

mod live;
mod orchestrate;
mod stop;

use super::question_scope::QuestionScope;
use super::turn_config::{AgentRuntimeError, PromptOverrides};
use crate::answering_receipts::AnsweringReceiptSink;
use saya_agent::{AgentEventSink, AgentOutput, ApprovalPolicy};
use saya_store::SqliteStateStore;
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;

use self::live::LiveAttemptRunner;
use self::orchestrate::orchestrate;

/// Runs one agent attempt and returns its output. Exists so [`orchestrate`] can
/// be tested against a fake runner without a live provider or database, the way
/// [`CandidateExecutor`] lets the decision logic be tested against a fake.
///
/// Uses a manually-boxed future with a lifetime tied to `&self` rather than
/// `async_trait`: the production runner holds a chat session's composition,
/// whose future captures borrowed connection-build state that does not satisfy
/// `async_trait`'s higher-ranked `Send`/lifetime bounds.
pub(crate) trait AttemptRunner: Sync {
    fn run(&self) -> Pin<Box<dyn Future<Output = Result<AgentOutput, AgentRuntimeError>> + '_>>;
}

/// The `ask` entry for candidate runs: `candidates` one-turn sessions over the
/// ask's own facts.
///
/// `candidates <= 1` runs the one attempt and returns it — no loop, no extra
/// connector, no consensus event. `candidates > 1` builds a live
/// [`CandidateExecutor`] for the active profile, then hands the loop to
/// [`orchestrate`].
///
/// Every attempt renders its answer through `sink`, the surface's own answer
/// sink, and the calls it paid are counted beside it: the receipts the
/// consensus sums ride the same sink. `folder` is the folder each attempt's
/// session opens over, and `format` is the rendering a session falls back to.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn run_with_candidates(
    runtime: &crate::config::runtime::RuntimeConfig,
    prompt: &str,
    approval: ApprovalPolicy,
    can_prompt: bool,
    overrides: PromptOverrides,
    folder: PathBuf,
    format: crate::render::RenderFormat,
    sink: Arc<dyn AgentEventSink>,
    scope: QuestionScope,
    state_db: Option<SqliteStateStore>,
    candidates: usize,
) -> Result<AgentOutput, AgentRuntimeError> {
    let cancellation = scope.cancellation();
    if cancellation.is_cancelled() {
        return Err(AgentRuntimeError::Agent("request cancelled".into()));
    }
    let receipts = Arc::new(AnsweringReceiptSink::new(sink));
    let build_overrides = overrides.clone();
    let runner = LiveAttemptRunner::new(
        runtime,
        prompt,
        approval,
        can_prompt,
        overrides,
        Arc::clone(&receipts) as Arc<dyn AgentEventSink>,
        scope.clone(),
        state_db,
        folder,
        format,
    );
    if candidates <= 1 {
        return runner.run().await;
    }
    let (executor, dialect) = {
        let deadline = scope.begin_segment();
        if cancellation.is_cancelled() {
            scope.finish_segment();
            return Err(AgentRuntimeError::Agent("request cancelled".into()));
        }
        let build = live::build_executor(runtime, &build_overrides, can_prompt, &scope);
        tokio::pin!(build);
        let result = match deadline {
            Some(duration) => tokio::select! {
                result = &mut build => result,
                _ = cancellation.cancelled() => build.await,
                _ = tokio::time::sleep(duration) => {
                    cancellation.cancel_with_reason(saya_agent::CancellationReason::Budget(saya_agent::BudgetDimension::ElapsedTime));
                    build.await
                }
            },
            None => tokio::select! {
                result = &mut build => result,
                _ = cancellation.cancelled() => build.await,
            },
        };
        scope.finish_segment();
        if cancellation.is_cancelled() {
            return Err(AgentRuntimeError::Agent("request cancelled".into()));
        }
        result?
    };
    orchestrate(
        candidates,
        &runner,
        &executor,
        dialect,
        &receipts,
        &cancellation,
        &scope,
    )
    .await
}

#[cfg(test)]
#[path = "candidates_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "receipt_tests.rs"]
mod receipt_tests;
