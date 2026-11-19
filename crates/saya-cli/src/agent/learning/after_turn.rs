//! What a turn does *after* it answered: drain the override findings and tell
//! the user, then run the post-turn extraction — unless the turn was
//! cancelled, in which case it learns nothing.
//!
//! One function, so the chat's turn and the database plugin's `on_turn_finish`
//! hook cannot drift into two policies: the hook hands over the same inputs the
//! chat path assembles, and the same call runs. [`LearnAfterTurnInputs`] is
//! everything the step reads; the prompt and the turn's output are the only
//! things a hook has that the rest of the source does not.

use std::sync::Arc;

use saya_agent::{AgentEvent, AgentEventSink, AgentOutput, ChatProvider, TokenUsage};

use super::breaker::LearningBreaker;
use super::post_turn::{PostTurnInputs, run_post_turn_extraction};
use crate::agent::tools::{DatabaseTools, ObservationLog, OverrideLog};
use crate::contracts::RecallReceipt;

/// Everything [`learn_after_turn`] reads. Owned only where the turn owns it:
/// the override log is shared with the tools that record into it, and the
/// provider outlives the turn.
pub(crate) struct LearnAfterTurnInputs<'a> {
    /// The findings the turn's SQL raised, drained here so one turn reports
    /// each contradicted claim once.
    pub(crate) override_log: Arc<OverrideLog>,
    /// Whether the user cancelled the turn before it answered.
    pub(crate) cancelled: bool,
    /// Whether this run may write candidate claims.
    pub(crate) permit_candidate_writes: bool,
    pub(crate) database: &'a DatabaseTools,
    /// The turn's output, or `None` when the answering call failed — there is
    /// nothing to learn from.
    pub(crate) output: Option<&'a AgentOutput>,
    pub(crate) prompt: &'a str,
    pub(crate) observations_log: Option<&'a ObservationLog>,
    /// What recall supplied this turn, as the extraction path must not
    /// re-propose a claim the model was already given.
    pub(crate) receipt: &'a RecallReceipt,
    pub(crate) provider: &'a dyn ChatProvider,
    pub(crate) model: &'a str,
    /// The breaker to consult. The chat's turn passes the session's, so it
    /// trips across the session's turns; the database plugin passes its own,
    /// which it keeps across the turns it serves. `None` means the run has no
    /// session (the one-shot `ask` path, a candidate attempt): a fresh breaker
    /// per call, which can never trip within one turn, since there is no next
    /// turn for it to disable.
    pub(crate) breaker: Option<&'a LearningBreaker>,
}

/// Drains and announces this turn's override findings, then runs the post-turn
/// extraction, returning the usage that call reported — `None` when the turn
/// learned nothing, absent rather than zero.
pub(crate) async fn learn_after_turn(
    inputs: LearnAfterTurnInputs<'_>,
    sink: &dyn AgentEventSink,
) -> Option<TokenUsage> {
    let LearnAfterTurnInputs {
        override_log,
        cancelled,
        permit_candidate_writes,
        database,
        output,
        prompt,
        observations_log,
        receipt,
        provider,
        model,
        breaker,
    } = inputs;

    let overridden = override_log.drain();
    if !overridden.is_empty() {
        sink.emit(AgentEvent::knowledge_overridden(overridden.clone()))
            .await;
    }

    let fresh_breaker;
    let breaker: &LearningBreaker = match breaker {
        Some(breaker) => breaker,
        None => {
            fresh_breaker = LearningBreaker::new();
            &fresh_breaker
        }
    };

    let output = output?;
    if cancelled {
        return None;
    }

    run_post_turn_extraction(
        PostTurnInputs {
            permit_candidate_writes,
            database,
            output: Ok(output),
            prompt,
            observations_log,
            receipt,
            overridden: &overridden,
            provider,
            model,
            breaker,
        },
        sink,
    )
    .await
}
