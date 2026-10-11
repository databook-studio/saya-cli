mod batch;
mod counting;
mod projection;

use crate::{BudgetDimension, ChatMessage, ToolDefinition};
use std::num::NonZeroU64;

/// Known provider context and output reservation for neutral request preflight.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RequestPreflightConfig {
    model_window_tokens: Option<NonZeroU64>,
    output_reserve_tokens: NonZeroU64,
}

impl RequestPreflightConfig {
    /// Builds configuration from the resolved model window and actual positive
    /// output reservation. `None` leaves the model window unknown.
    pub fn new(model_window_tokens: Option<NonZeroU64>, output_reserve_tokens: NonZeroU64) -> Self {
        Self {
            model_window_tokens,
            output_reserve_tokens,
        }
    }
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
    sink: &dyn crate::AgentEventSink,
) -> Result<(), PreflightError> {
    let (byte_limit, config) = budget;
    let (cap, dimension) = effective_limit(byte_limit, config);
    match projection::measure(
        model,
        projection::Messages::Slice(messages),
        definitions,
        cap,
    ) {
        projection::Measurement::Fits(_) => return Ok(()),
        projection::Measurement::Serialization => return Err(PreflightError::Serialization),
        projection::Measurement::Exceeded => {}
    }
    let fixed = match projection::measure(model, projection::Messages::Slice(&[]), definitions, cap)
    {
        projection::Measurement::Fits(size) => size,
        projection::Measurement::Exceeded => return Err(PreflightError::Budget(dimension)),
        projection::Measurement::Serialization => return Err(PreflightError::Serialization),
    };
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
            projection::Measurement::Fits(_) => return Ok(()),
            projection::Measurement::Serialization => {
                return Err(PreflightError::Serialization);
            }
            projection::Measurement::Exceeded => {}
        }
    }
    super::output::trim_to_budget(
        messages,
        cap.saturating_sub(fixed),
        *protected_prefix_len,
        Some(sink),
    );
    match projection::measure(
        model,
        projection::Messages::Slice(messages),
        definitions,
        cap,
    ) {
        projection::Measurement::Fits(_) => Ok(()),
        projection::Measurement::Exceeded => {
            // The established compactor uses a deliberately cheap message-size
            // estimate. If that estimate leaves the exact neutral projection
            // oversized, evict older complete groups once more before refusing.
            super::output::trim_to_budget(messages, 0, *protected_prefix_len, Some(sink));
            match projection::measure(
                model,
                projection::Messages::Slice(messages),
                definitions,
                cap,
            ) {
                projection::Measurement::Fits(_) => Ok(()),
                projection::Measurement::Exceeded => Err(PreflightError::Budget(dimension)),
                projection::Measurement::Serialization => Err(PreflightError::Serialization),
            }
        }
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
    sink: &dyn crate::AgentEventSink,
) -> Result<usize, PreflightError> {
    batch::reserve(
        model,
        messages,
        protected_prefix_len,
        assistant,
        definitions,
        budget,
        sink,
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
        .saturating_sub(config.output_reserve_tokens.get())
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
