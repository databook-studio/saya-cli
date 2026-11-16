use super::{
    PreflightError, effective_limit,
    projection::{self, BatchReplies, Measurement, Messages},
};
use crate::{ChatMessage, ToolDefinition};

pub(super) fn reserve(
    model: &str,
    prefix: &mut Vec<ChatMessage>,
    protected_prefix_len: usize,
    assistant: &ChatMessage,
    definitions: &[ToolDefinition],
    budget: (usize, Option<super::RequestPreflightConfig>),
    compaction: super::CompactionContext<'_>,
) -> Result<usize, PreflightError> {
    let super::CompactionContext { sink, checkpoint } = compaction;
    let (byte_limit, config) = budget;
    let (cap, dimension) = effective_limit(byte_limit, config);
    // The pending batch is the newest group during compaction and cannot be
    // executed until the exact projection, including paired refusals, fits.
    for _ in 0..4 {
        let skeleton = Messages::Batch {
            prefix,
            assistant,
            calls: &assistant.tool_calls,
            replies: BatchReplies::Refusal,
        };
        let actual = match projection::measure(model, skeleton, definitions, usize::MAX) {
            Measurement::Fits(size) if size <= cap => break,
            Measurement::Fits(size) => size,
            Measurement::Exceeded => return Err(PreflightError::Budget(dimension)),
            Measurement::Serialization => return Err(PreflightError::Serialization),
        };
        prefix.push(assistant.clone());
        let estimate: usize = prefix.iter().map(super::super::output::message_size).sum();
        super::super::output::trim_to_budget(
            prefix,
            super::super::output::ContextBudget::new(
                estimate.saturating_sub(actual - cap),
                config.and_then(|config| config.compaction_trigger_tokens()),
            ),
            protected_prefix_len,
            Some(sink),
            checkpoint,
        );
        prefix.pop();
    }
    let skeleton = Messages::Batch {
        prefix,
        assistant,
        calls: &assistant.tool_calls,
        replies: BatchReplies::Refusal,
    };
    match projection::measure(model, skeleton, definitions, cap) {
        Measurement::Fits(_) => {}
        Measurement::Exceeded => return Err(PreflightError::Budget(dimension)),
        Measurement::Serialization => return Err(PreflightError::Serialization),
    }

    // The room the batch has. Tool results are capped per call, not divided
    // out of a shared per-turn pool, so there is nothing left to reserve here.
    Ok(cap)
}

pub(super) fn non_execution_replies(assistant: &ChatMessage) -> Vec<ChatMessage> {
    assistant
        .tool_calls
        .iter()
        .map(|call| ChatMessage {
            role: "tool".into(),
            content: projection::REFUSAL.into(),
            tool_calls: Vec::new(),
            tool_call_id: Some(call.id.clone()),

            images: Vec::new(),
            reasoning: None,
            reasoning_signature: None,
        })
        .collect()
}
