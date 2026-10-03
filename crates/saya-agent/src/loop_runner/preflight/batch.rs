use super::{
    PreflightError, effective_limit,
    projection::{self, BatchReplies, Measurement, Messages},
};
use crate::{ChatMessage, ToolDefinition};

pub(super) fn reserve(
    model: &str,
    prefix: &mut Vec<ChatMessage>,
    assistant: &ChatMessage,
    definitions: &[ToolDefinition],
    byte_limit: usize,
    config: Option<super::RequestPreflightConfig>,
) -> Result<usize, PreflightError> {
    let (cap, dimension) = effective_limit(byte_limit, config);
    let skeleton = Messages::Batch {
        prefix,
        assistant,
        calls: &assistant.tool_calls,
        replies: BatchReplies::Refusal,
    };
    if matches!(
        projection::measure(model, skeleton, definitions, cap),
        Measurement::Exceeded
    ) {
        // Include the pending assistant batch while the established compactor
        // evicts older complete groups, so it cannot rewrite the batch itself.
        prefix.push(assistant.clone());
        super::super::output::trim_to_budget(prefix, 0);
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

    let base = Messages::Batch {
        prefix,
        assistant,
        calls: &assistant.tool_calls,
        replies: BatchReplies::Empty,
    };
    let size = match projection::measure(model, base, definitions, cap) {
        Measurement::Fits(size) => size,
        Measurement::Exceeded => return Err(PreflightError::Budget(dimension)),
        Measurement::Serialization => return Err(PreflightError::Serialization),
    };
    let content_share = cap.saturating_sub(size) / 6;
    if content_share == 0 {
        return Err(PreflightError::Budget(dimension));
    }
    Ok(content_share.min(byte_limit))
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
        })
        .collect()
}
