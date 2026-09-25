//! The turn's post-turn extraction step: gate → started event → extraction
//! → usage/proposed/skipped events → trace, with the per-session circuit
//! breaker ([`super::breaker::LearningBreaker`]) consulted before any of it
//! runs. Moved out of `runtime.rs` whole to keep that file under its line
//! cap; `runtime.rs` calls [`run_post_turn_extraction`] with everything it
//! needs, bundled in [`PostTurnInputs`] to stay under the arity lint.

use saya_agent::{
    AgentError, AgentEvent, AgentEventSink, AgentOutput, ChatProvider, LearningSkipReason,
    OverrideFindingDto, TokenUsage, UsageCall,
};

use super::breaker::{AttemptOutcome, LearningBreaker, classify};
use super::{ProposalGating, TurnRecord, run_extraction};
use crate::agent::extraction_trace::trace_extraction;
use crate::agent::tools::{DatabaseTools, ObservationLog};
use crate::contracts::RecallReceipt;

/// Everything [`run_post_turn_extraction`] needs, bundled so the call stays
/// under the arity lint's budget (the struct is one parameter).
pub(crate) struct PostTurnInputs<'a> {
    pub(crate) permit_candidate_writes: bool,
    pub(crate) database: &'a DatabaseTools,
    pub(crate) output: Result<&'a AgentOutput, &'a AgentError>,
    pub(crate) prompt: &'a str,
    pub(crate) observations_log: Option<&'a ObservationLog>,
    pub(crate) receipt: &'a RecallReceipt,
    pub(crate) overridden: &'a [OverrideFindingDto],
    pub(crate) provider: &'a dyn ChatProvider,
    pub(crate) model: &'a str,
    pub(crate) breaker: &'a LearningBreaker,
}

/// Runs the turn's post-turn structured extraction step (Safety Property 1:
/// fail-soft isolation) and returns the usage the extraction call reported,
/// if any. `None` when learning is off for this turn, the gate declines, the
/// breaker has already tripped this session, or the call produced no
/// response — absent is not zero.
pub(crate) async fn run_post_turn_extraction(
    inputs: PostTurnInputs<'_>,
    sink: &dyn AgentEventSink,
) -> Option<TokenUsage> {
    let PostTurnInputs {
        permit_candidate_writes,
        database,
        output,
        prompt,
        observations_log,
        receipt,
        overridden,
        provider,
        model,
        breaker,
    } = inputs;

    if !permit_candidate_writes {
        return None;
    }
    let store = database.state_db()?;
    let out = output.ok()?;
    // The breaker tripped on an earlier turn this session: no extraction
    // request at all, not even the gate.
    if breaker.is_disabled() {
        return None;
    }

    let drained_obs = observations_log.map(|log| log.drain()).unwrap_or_default();
    let turn_record = TurnRecord::assemble(
        prompt,
        &out.answer,
        database.registry(),
        &drained_obs,
        Some(receipt),
        overridden,
    );
    let gate = ProposalGating::evaluate(&turn_record, &drained_obs, !overridden.is_empty());
    if !gate.is_run() {
        // Gate decline stays silent on screen (decision 2); trace it for
        // observability when debugging the boundary.
        trace_extraction(
            "gate_declined",
            turn_record.object_table.len(),
            None,
            None,
            None,
        );
        return None;
    }

    let object_count = turn_record.object_table.len();
    // The answer is already on screen; this call is what the adapter is
    // still waiting on, so say so before starting it.
    sink.emit(AgentEvent::KnowledgeLearningStarted).await;
    let extraction_started = std::time::Instant::now();
    // No wall-clock ceiling on this call — an explicit owner decision that
    // departs from AGENTS.md's "bound untrusted work — time" rule for this
    // one path. A slow model may take as long as it needs; the call is
    // bounded only by the provider transport's own limits (the per-attempt
    // connect timeout, the per-chunk stream idle timeout, the output-token
    // ceiling, bounded retries). A stalled or truncated reply trips the
    // per-session `LearningBreaker` instead of a clock.
    let outcome = run_extraction(
        provider,
        model,
        &turn_record,
        database.registry(),
        store,
        receipt,
    )
    .await;
    let extraction_elapsed = Some(extraction_started.elapsed());

    // The extraction call's report crosses the stream named as an
    // extraction call, so a consumer can keep it apart from the answering
    // rounds' — it is billed separately and would otherwise lower the cache
    // hit rate computed over the answer's calls. `None` (a provider that
    // reported nothing, or no response at all) emits nothing.
    if let Some(counts) = outcome.usage {
        sink.emit(AgentEvent::usage(UsageCall::Extraction, counts))
            .await;
    }
    match outcome.dtos {
        Ok(dtos) => {
            trace_extraction(
                "ok",
                object_count,
                Some(dtos.len()),
                None,
                extraction_elapsed,
            );
            breaker.record(AttemptOutcome::Other);
            for dto in dtos {
                sink.emit(AgentEvent::knowledge_proposed(dto)).await;
            }
        }
        Err(error) => {
            trace_extraction(
                "failed",
                object_count,
                Some(0),
                Some(&error.to_string()),
                extraction_elapsed,
            );
            let tripped = breaker.record(classify(&error));
            sink.emit(AgentEvent::knowledge_learning_skipped(
                LearningSkipReason::Failed,
            ))
            .await;
            // Emitted after this turn's own `KnowledgeLearningSkipped`
            // (decision 3), only on the attempt that just tripped the
            // breaker — never again this session.
            if let Some(misses) = tripped {
                sink.emit(AgentEvent::knowledge_learning_disabled(model, misses))
                    .await;
            }
        }
    }
    outcome.usage
}
