//! Runs the agent multiple times and decides which answer to believe.
//!
//! Wires [`crate::agent::decide`] into the `ask` path: when `candidates > 1`,
//! the agent runs `candidates` times in sequence (each with a fresh empty
//! history), the nominated SQL of each attempt is collected, and [`decide`]
//! votes on result-set fingerprints — emitting one `ConsensusDecided` event
//! and returning the winning attempt's output.
//!
//! [`run_with_candidates`] is the production entry: `candidates <= 1` returns
//! [`run_prompt_with_sink`] unchanged so the overwhelmingly common case keeps
//! today's exact code path — no loop, no extra connector, no new event. The
//! loop, the decision, and the emit live in [`orchestrate`], which is injectable
//! for tests via the [`AttemptRunner`] trait (mirroring [`CandidateExecutor`]).

mod live;
mod orchestrate;

use super::runtime::{AgentRuntimeError, PromptOverrides, run_prompt_with_sink};
use saya_agent::{AgentOutput, ApprovalPolicy, CancellationToken, ChatMessage};
use saya_store::SqliteStateStore;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use self::live::LiveAttemptRunner;
use self::orchestrate::orchestrate;

/// Runs one agent attempt and returns its output. Exists so [`orchestrate`] can
/// be tested against a fake runner without a live provider or database, the way
/// [`CandidateExecutor`] lets the decision logic be tested against a fake.
///
/// Uses a manually-boxed future with a lifetime tied to `&self` rather than
/// `async_trait`: the production runner delegates to [`run_prompt_with_sink`],
/// whose future captures borrowed connection-build state that does not satisfy
/// `async_trait`'s higher-ranked `Send`/lifetime bounds.
pub(crate) trait AttemptRunner: Sync {
    fn run(&self) -> Pin<Box<dyn Future<Output = Result<AgentOutput, AgentRuntimeError>> + '_>>;
}

/// The `ask` entry for candidate runs. Takes the same arguments as
/// [`run_prompt_with_sink`] plus `candidates`.
///
/// `candidates <= 1` returns [`run_prompt_with_sink`] unchanged: no loop, no
/// extra connector, no consensus event — today's single-run path, byte for
/// byte. `candidates > 1` builds a live [`CandidateExecutor`] for the active
/// profile, then hands the loop to [`orchestrate`].
#[allow(clippy::too_many_arguments)]
pub(crate) async fn run_with_candidates(
    runtime: &crate::config::runtime::RuntimeConfig,
    prompt: &str,
    approval: ApprovalPolicy,
    can_prompt: bool,
    overrides: PromptOverrides,
    history: Vec<ChatMessage>,
    sink: &dyn saya_agent::AgentEventSink,
    cancellation: CancellationToken,
    state_db: Option<SqliteStateStore>,
    decider: Option<Arc<dyn saya_agent::ApprovalDecider>>,
    last_sql: Option<String>,
    candidates: usize,
) -> Result<AgentOutput, AgentRuntimeError> {
    if candidates <= 1 {
        return run_prompt_with_sink(
            runtime,
            prompt,
            approval,
            can_prompt,
            overrides,
            history,
            sink,
            cancellation,
            state_db,
            decider,
            last_sql,
        )
        .await;
    }
    let (executor, dialect) = live::build_executor(runtime, &overrides, can_prompt).await?;
    let runner = LiveAttemptRunner::new(
        runtime,
        prompt,
        approval,
        can_prompt,
        overrides,
        sink,
        cancellation.clone(),
        state_db,
        decider,
        last_sql,
    );
    orchestrate(candidates, &runner, &executor, dialect, sink, &cancellation).await
}

#[cfg(test)]
#[path = "candidates_tests.rs"]
mod tests;
