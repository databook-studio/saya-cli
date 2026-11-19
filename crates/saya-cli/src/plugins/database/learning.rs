//! The database plugin's optional learning source.
//!
//! A run that configured memory hands the plugin a [`LearningSource`]: the
//! turn inputs the post-turn learning step reads, everything except the prompt
//! and the output — those two are the turn's, and only the `on_turn_finish`
//! hook has them. The hook learns through the same [`learn_after_turn`] the
//! chat's turn calls, so the plugin can never learn under a second policy that
//! drifts from the chat's.

use std::sync::Arc;

use saya_agent::{AgentEventSink, ChatProvider, TokenUsage, TurnOutcome};

use crate::agent::learning::LearningBreaker;
use crate::agent::learning::after_turn::{LearnAfterTurnInputs, learn_after_turn};
use crate::agent::tools::{DatabaseTools, ObservationLog, OverrideLog};
use crate::contracts::RecallReceipt;

/// What a turn's post-turn learning needs beyond the prompt and the output,
/// owned so the plugin can hold it across turns. The database tools are
/// shared: the turn's own handle is the same tool set, and learning only reads
/// its store and connection set.
pub(crate) struct LearningSource {
    permit_candidate_writes: bool,
    database: Arc<DatabaseTools>,
    observations_log: Option<Arc<ObservationLog>>,
    override_log: Arc<OverrideLog>,
    provider: Arc<dyn ChatProvider>,
    model: String,
    /// The breaker the plugin consults for its turns. One per plugin until a
    /// breaker is session-scoped: its misses accumulate across the turns the
    /// plugin serves, instead of starting fresh every turn.
    breaker: LearningBreaker,
}

impl LearningSource {
    pub(crate) fn new(
        permit_candidate_writes: bool,
        database: Arc<DatabaseTools>,
        observations_log: Option<Arc<ObservationLog>>,
        override_log: Arc<OverrideLog>,
        provider: Arc<dyn ChatProvider>,
        model: impl Into<String>,
    ) -> Self {
        Self {
            permit_candidate_writes,
            database,
            observations_log,
            override_log,
            provider,
            model: model.into(),
            breaker: LearningBreaker::new(),
        }
    }

    /// Learns from `outcome` for `receipt`, announcing through `sink` — the
    /// owned inputs become the borrows the shared function takes.
    pub(crate) async fn learn(
        &self,
        outcome: &TurnOutcome<'_>,
        receipt: &RecallReceipt,
        sink: &dyn AgentEventSink,
    ) -> Option<TokenUsage> {
        learn_after_turn(
            LearnAfterTurnInputs {
                override_log: self.override_log.clone(),
                cancelled: outcome.cancelled,
                permit_candidate_writes: self.permit_candidate_writes,
                database: &self.database,
                output: outcome.output,
                prompt: outcome.prompt,
                observations_log: self.observations_log.as_deref(),
                receipt,
                provider: self.provider.as_ref(),
                model: &self.model,
                breaker: Some(&self.breaker),
            },
            sink,
        )
        .await
    }
}
