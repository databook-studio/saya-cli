//! The candidate-orchestration core: run the agent `candidates` times, collect
//! each attempt's nominated SQL, and call [`decide`] to pick a winner. Testable
//! in isolation via the [`AttemptRunner`] trait and a fake [`CandidateExecutor`].
//!
//! [`decide`]: super::super::decide::decide

use super::super::decide::{CandidateExecutor, decide};
use super::super::runtime::AgentRuntimeError;
use super::AttemptRunner;
use saya_agent::{AgentEvent, AgentEventSink, AgentOutput, CancellationToken};
use saya_types::SqlDialect;
use std::collections::HashSet;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Runs the agent `candidates` times and decides which answer to believe.
///
/// `candidates <= 1` runs the attempt once and returns it — no loop, no
/// decision, no event. Production short-circuits before here for `candidates <=
/// 1`, but the core is defensive so the boundary is testable: the
/// `candidates = 1` test proves (by counting) that one attempt ran and no
/// consensus event was emitted.
///
/// For `candidates > 1`: each attempt runs in sequence with a fresh empty
/// history; an attempt that errors contributes no nomination and does not abort
/// the rest. When more than one attempt ran and at least one succeeded,
/// [`decide`] votes on the result sets, a `ConsensusDecided` event is emitted
/// once, and the winning attempt's output is returned — or, with no winner, the
/// last successful attempt's output, so the caller always has an answer to show.
pub(crate) async fn orchestrate(
    candidates: usize,
    runner: &dyn AttemptRunner,
    executor: &dyn CandidateExecutor,
    dialect: SqlDialect,
    sink: &dyn AgentEventSink,
    cancellation: &CancellationToken,
) -> Result<AgentOutput, AgentRuntimeError> {
    if candidates <= 1 {
        return runner.run().await;
    }
    // Aligned by index: `nominations[i]` is attempt i's designated SQL (or
    // `None` when it errored or designated nothing); `outputs[i]` is its output
    // (or `None` when it errored). `decide` returns winner indices into this
    // same alignment.
    let mut nominations: Vec<Option<String>> = Vec::with_capacity(candidates);
    let mut outputs: Vec<Option<AgentOutput>> = Vec::with_capacity(candidates);
    let mut last_error: Option<AgentRuntimeError> = None;
    for _ in 0..candidates {
        if cancellation.is_cancelled() {
            break;
        }
        match runner.run().await {
            Ok(out) => {
                nominations.push(out.answer_sql.clone());
                outputs.push(Some(out));
            }
            // A failed attempt contributes no nomination and does not abort the
            // rest; it is recorded so the alignment holds and the error is
            // available if nothing else succeeds.
            Err(error) => {
                last_error = Some(error);
                nominations.push(None);
                outputs.push(None);
            }
        }
    }

    let last_success = outputs.iter().rev().flatten().next().cloned();
    let Some(last_success) = last_success else {
        // Nothing succeeded. Surface the last error, or — when the loop was
        // cancelled before any attempt completed — a cancellation error rather
        // than a panic over an absent error.
        return Err(
            last_error.unwrap_or_else(|| AgentRuntimeError::Agent("request cancelled".into()))
        );
    };

    // The event is emitted once whenever more than one attempt ran — including
    // when there is no winner, because "the attempts disagreed" is the most
    // interesting thing a reader can learn here.
    if nominations.len() > 1 {
        let nominated: HashSet<String> = nominations.iter().flatten().cloned().collect();
        let counter = VoteCounter::new(executor, &nominated);
        let decision = decide(&nominations, &counter, dialect, cancellation).await;
        let winner_sql = decision.winner.and_then(|i| {
            outputs
                .get(i)
                .and_then(|o| o.as_ref().and_then(|x| x.answer_sql.clone()))
        });
        sink.emit(AgentEvent::consensus_decided(
            winner_sql,
            nominations.len(),
            counter.voted(),
            decision.votes,
            decision.margin,
            decision.tied,
            decision.probe_broke_tie,
        ))
        .await;
        if let Some(i) = decision.winner
            && let Some(Some(out)) = outputs.get(i)
        {
            return Ok(out.clone());
        }
    }

    // No winner (tie, or a single survivor after cancellation): return the last
    // successful attempt's output so the caller always has an answer to show.
    Ok(last_success)
}

/// Wraps a [`CandidateExecutor`] to count how many nominated statements
/// produced a result that could vote. Only SQL present in `nominated` is
/// counted, so the fan-out probe statements [`decide`] runs during tie
/// resolution (which are not in `nominated`) are not mistaken for votes.
struct VoteCounter<'a> {
    inner: &'a dyn CandidateExecutor,
    nominated: &'a HashSet<String>,
    voted: AtomicUsize,
}

impl<'a> VoteCounter<'a> {
    fn new(inner: &'a dyn CandidateExecutor, nominated: &'a HashSet<String>) -> Self {
        Self {
            inner,
            nominated,
            voted: AtomicUsize::new(0),
        }
    }

    fn voted(&self) -> usize {
        self.voted.load(Ordering::Relaxed)
    }
}

#[async_trait::async_trait]
impl CandidateExecutor for VoteCounter<'_> {
    async fn run(&self, sql: &str) -> Option<saya_types::QueryResult> {
        let result = self.inner.run(sql).await;
        if result.is_some() && self.nominated.contains(sql) {
            self.voted.fetch_add(1, Ordering::Relaxed);
        }
        result
    }
}
