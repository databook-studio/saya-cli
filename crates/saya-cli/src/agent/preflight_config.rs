//! Composition of the agent's request preflight from the effective AI
//! settings a turn runs with.
//!
//! The model window follows the frozen semantics: the user's explicit
//! `ai.context_window_tokens` when declared, otherwise the built-in table's
//! answer for the effective model, otherwise unknown — unknown stays unknown,
//! never inferred from usage or summaries. The output reservation is the
//! effective `ai.max_output_tokens` the provider is actually built with, or
//! absent when the operator configured none — a declared zero is a typo, not
//! an absent fact, so it is a typed configuration error rather than a
//! silently missing ceiling.

use super::turn_config::AgentRuntimeError;
use saya_agent::RequestPreflightConfig;
use saya_config::{ResolvedAi, context_window_tokens};
use std::num::NonZeroU64;

/// Prompt margin kept above the output reservation when capping requests at
/// the model window. Matches the fixed 1024-token margin subtracted in
/// `crates/saya-agent/src/loop_runner/preflight.rs` (`effective_limit`): an
/// output ceiling that leaves at most this margin cannot hold a prompt.
const PROMPT_MARGIN_TOKENS: u64 = 1024;

/// Composes the neutral request preflight from the same effective `ai` the
/// turn's provider is built from. Counting and any estimator stay in the
/// agent; this only hands it the configured facts.
pub(crate) fn request_preflight_config(
    ai: &ResolvedAi,
) -> Result<RequestPreflightConfig, AgentRuntimeError> {
    let output_reserve_tokens = match ai.max_output_tokens {
        Some(0) => {
            return Err(AgentRuntimeError::Configuration(
                "ai.max_output_tokens must be a positive token count, got 0".into(),
            ));
        }
        Some(configured) => Some(NonZeroU64::new(u64::from(configured)).expect("checked above")),
        None => None,
    };
    let model_window_tokens = match ai.context_window_tokens {
        Some(0) => {
            return Err(AgentRuntimeError::Configuration(
                "ai.context_window_tokens must be a positive token count, got 0".into(),
            ));
        }
        Some(explicit) => NonZeroU64::new(explicit),
        None => context_window_tokens(&ai.model).and_then(NonZeroU64::new),
    };
    if let (Some(reserve), Some(window)) = (output_reserve_tokens, model_window_tokens)
        && reserve.get() + PROMPT_MARGIN_TOKENS >= window.get()
    {
        return Err(AgentRuntimeError::Configuration(format!(
            "ai.max_output_tokens {} leaves no room for a prompt in the \
             model context window {}",
            reserve.get(),
            window.get()
        )));
    }
    Ok(
        RequestPreflightConfig::new(model_window_tokens, output_reserve_tokens)
            .with_compaction_trigger_percent(u64::from(ai.compaction_trigger_percent)),
    )
}

#[cfg(test)]
#[path = "preflight_config_tests.rs"]
mod tests;
