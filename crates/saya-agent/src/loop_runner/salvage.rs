//! Salvaging an agent run that hit a turn or tool-call ceiling: answer from
//! work already done instead of discarding it.

use super::{emit, receive, sum_reported, tools};
use crate::{
    AgentError, AgentEvent, AgentEventSink, AgentOutput, CancellationToken, ChatProvider,
    TokenUsage,
};

/// Appended to the conversation before the salvage call so the model knows its
/// tools are gone and it must answer from what it has, not call more tools.
const SALVAGE_INSTRUCTION: &str = "You have no further tool calls available. Using only the results you have already gathered, give your best answer to the question now.";

/// Salvages a run that has hit a ceiling instead of discarding everything. Any
/// tool calls the run did not get to execute are answered with a "budget
/// exhausted" result so the conversation the provider sees is well-formed;
/// then one final call with no tools asks for the best answer from the work
/// already done. The returned output is marked [`AgentOutput::truncated`]. If
/// the final call itself fails, its error is returned unchanged — the run does
/// not invent an answer and does not swallow the provider failure.
#[allow(clippy::too_many_arguments)]
pub(super) async fn salvage(
    provider: &dyn ChatProvider,
    model: &str,
    messages: &mut Vec<crate::ChatMessage>,
    pending: &[crate::ToolCall],
    context_byte_budget: usize,
    sink: &dyn AgentEventSink,
    cancellation: &CancellationToken,
    events: &mut Vec<AgentEvent>,
    usage: &mut TokenUsage,
    used_bounded_sql_query: bool,
    tool_metadata: Vec<crate::ToolMetadata>,
) -> Result<AgentOutput, AgentError> {
    for call in pending {
        let (message, _) = tools::tool_message(
            call.id.clone(),
            serde_json::json!({"error": "tool call budget exhausted"}),
            context_byte_budget,
        );
        messages.push(message);
    }
    messages.push(crate::ChatMessage::text("user", SALVAGE_INSTRUCTION));
    let (assistant, turn_usage, _) =
        receive::receive(provider, model, messages, &[], sink, cancellation, events).await?;
    usage.input_tokens += turn_usage.input_tokens;
    usage.output_tokens += turn_usage.output_tokens;
    sum_reported(
        &mut usage.cached_input_tokens,
        turn_usage.cached_input_tokens,
    );
    sum_reported(
        &mut usage.cache_creation_input_tokens,
        turn_usage.cache_creation_input_tokens,
    );
    sum_reported(&mut usage.reasoning_tokens, turn_usage.reasoning_tokens);
    emit(events, sink, AgentEvent::Complete).await;
    Ok(AgentOutput {
        answer: assistant.content,
        events: std::mem::take(events),
        used_bounded_sql_query,
        tool_metadata,
        usage: *usage,
        learning_usage: None,
        truncated: true,
        answer_sql: None,
    })
}
