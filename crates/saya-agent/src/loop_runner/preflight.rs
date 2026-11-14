mod batch;
mod counting;
mod projection;

use crate::{BudgetDimension, ChatMessage, ToolDefinition};
use std::num::NonZeroU64;

/// Default share of a known context window at which the loop compacts its
/// conversation. Half the window leaves room for the next request's prompt and
/// output without letting a long run reach the window itself.
pub const DEFAULT_COMPACTION_TRIGGER_PERCENT: u8 = 50;

/// Known provider context and output reservation for neutral request preflight
/// and token-keyed context compaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RequestPreflightConfig {
    model_window_tokens: Option<NonZeroU64>,
    output_reserve_tokens: Option<NonZeroU64>,
    compaction_trigger_percent: NonZeroU64,
}

impl RequestPreflightConfig {
    /// Builds configuration from the resolved model window and the configured
    /// output reservation. `None` leaves the model window unknown; a `None`
    /// reservation means the operator configured no output ceiling, so no room
    /// is held back for one. Compaction triggers at
    /// [`DEFAULT_COMPACTION_TRIGGER_PERCENT`] of a known window.
    pub fn new(
        model_window_tokens: Option<NonZeroU64>,
        output_reserve_tokens: Option<NonZeroU64>,
    ) -> Self {
        Self {
            model_window_tokens,
            output_reserve_tokens,
            compaction_trigger_percent: NonZeroU64::new(u64::from(
                DEFAULT_COMPACTION_TRIGGER_PERCENT,
            ))
            .expect("the default trigger percent is nonzero"),
        }
    }

    /// Sets the share of a known window, in percent, at which the loop
    /// compacts. Values outside `1..=100` are clamped: a zero trigger would
    /// compact on every turn, and above the whole window it could never fire.
    #[must_use]
    pub fn with_compaction_trigger_percent(mut self, percent: u64) -> Self {
        self.compaction_trigger_percent =
            NonZeroU64::new(percent.clamp(1, 100)).expect("clamped to at least one");
        self
    }

    /// Tokens at which the conversation compacts, or `None` when the model
    /// window is unknown. This is the token estimate's comparison point; the
    /// conversation byte budget remains a separate hard ceiling.
    pub fn compaction_trigger_tokens(&self) -> Option<u64> {
        self.model_window_tokens.map(|window| {
            window
                .get()
                .saturating_mul(self.compaction_trigger_percent.get())
                / 100
        })
    }
}

#[derive(Clone, Copy)]
pub(super) struct CompactionContext<'a> {
    pub(super) sink: &'a dyn crate::AgentEventSink,
    pub(super) checkpoint: &'a super::coding_checkpoint::CodingCheckpoint,
}

pub(super) enum PreflightError {
    Budget(BudgetDimension),
    Serialization,
}

pub(super) fn fit_request(
    model: &str,
    messages: &mut Vec<ChatMessage>,
    ordinary_history_pairs: &mut usize,
    protected_prefix_len: &mut usize,
    definitions: &[ToolDefinition],
    budget: (usize, Option<RequestPreflightConfig>),
    compaction: CompactionContext<'_>,
) -> Result<usize, PreflightError> {
    let CompactionContext { sink, checkpoint } = compaction;
    let (byte_limit, config) = budget;
    let (cap, dimension) = effective_limit(byte_limit, config);
    match projection::measure(
        model,
        projection::Messages::Slice(messages),
        definitions,
        cap,
    ) {
        projection::Measurement::Fits(size) => return Ok(size),
        projection::Measurement::Serialization => return Err(PreflightError::Serialization),
        projection::Measurement::Exceeded => {}
    }
    match projection::measure(model, projection::Messages::Slice(&[]), definitions, cap) {
        projection::Measurement::Fits(_) => {}
        projection::Measurement::Exceeded => return Err(PreflightError::Budget(dimension)),
        projection::Measurement::Serialization => return Err(PreflightError::Serialization),
    }
    while crate::history::discard_oldest_history_pair(
        messages,
        ordinary_history_pairs,
        protected_prefix_len,
    ) {
        match projection::measure(
            model,
            projection::Messages::Slice(messages),
            definitions,
            cap,
        ) {
            projection::Measurement::Fits(size) => return Ok(size),
            projection::Measurement::Serialization => {
                return Err(PreflightError::Serialization);
            }
            projection::Measurement::Exceeded => {}
        }
    }
    // Counting never allocates the wire payload. Remove the actual excess
    // instead of asking for a zero-byte conversation, which discards a safe
    // checkpoint despite available space. Recheck the exact owner projection.
    for _ in 0..4 {
        let actual = match projection::measure(
            model,
            projection::Messages::Slice(messages),
            definitions,
            usize::MAX,
        ) {
            projection::Measurement::Fits(size) if size <= cap => return Ok(size),
            projection::Measurement::Fits(size) => size,
            projection::Measurement::Exceeded => return Err(PreflightError::Budget(dimension)),
            projection::Measurement::Serialization => return Err(PreflightError::Serialization),
        };
        let estimate: usize = messages.iter().map(super::output::message_size).sum();
        super::output::trim_to_budget(
            messages,
            super::output::ContextBudget::new(
                estimate.saturating_sub(actual - cap),
                config.and_then(|config| config.compaction_trigger_tokens()),
            ),
            *protected_prefix_len,
            Some(sink),
            checkpoint,
        );
    }
    match projection::measure(
        model,
        projection::Messages::Slice(messages),
        definitions,
        cap,
    ) {
        projection::Measurement::Fits(size) => Ok(size),
        projection::Measurement::Exceeded => Err(PreflightError::Budget(dimension)),
        projection::Measurement::Serialization => Err(PreflightError::Serialization),
    }
}

pub(super) fn reserve_tool_batch(
    model: &str,
    messages: &mut Vec<ChatMessage>,
    protected_prefix_len: usize,
    assistant: &ChatMessage,
    definitions: &[ToolDefinition],
    budget: (usize, Option<RequestPreflightConfig>),
    compaction: CompactionContext<'_>,
) -> Result<usize, PreflightError> {
    batch::reserve(
        model,
        messages,
        protected_prefix_len,
        assistant,
        definitions,
        budget,
        compaction,
    )
}

pub(super) fn non_execution_replies(assistant: &ChatMessage) -> Vec<ChatMessage> {
    batch::non_execution_replies(assistant)
}

pub(super) fn effective_limit(
    byte_limit: usize,
    config: Option<RequestPreflightConfig>,
) -> (usize, BudgetDimension) {
    let Some(config) = config else {
        return (byte_limit, BudgetDimension::RequestContextBytes);
    };
    let Some(model_window) = config.model_window_tokens else {
        return (byte_limit, BudgetDimension::RequestContextBytes);
    };
    let window = model_window
        .get()
        .saturating_sub(config.output_reserve_tokens.map_or(0, NonZeroU64::get))
        .saturating_sub(1024);
    let window = usize::try_from(window).unwrap_or(usize::MAX);
    if window <= byte_limit {
        (window, BudgetDimension::RequestContextWindow)
    } else {
        (byte_limit, BudgetDimension::RequestContextBytes)
    }
}

#[cfg(test)]
#[path = "preflight/batch_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "preflight/reasoning_tests.rs"]
mod reasoning_tests;
