//! Salvaging an agent run that hit a turn or tool-call ceiling: answer from
//! work already done instead of discarding it.

use super::{emit, receive, sum_reported, tools};
use crate::{
    AgentError, AgentEvent, AgentEventSink, AgentOutput, CancellationToken, ChatProvider,
    ProviderError, TokenUsage,
};

/// Appended to the conversation before the salvage call so the model knows its
/// tools are gone and it must answer from what it has, not call more tools.
const SALVAGE_INSTRUCTION: &str = "You have no further tool calls available. Using only the results you have already gathered, give your best answer to the question now.";

/// Salvages a run that has hit a ceiling instead of discarding everything. Any
/// tool calls the run did not get to execute are answered with a "budget
/// exhausted" result so the conversation the provider sees is well-formed;
/// then one final call with no tools asks for the best answer from the work
/// already done. The returned output is marked [`AgentOutput::truncated`]. If
/// the final call itself fails, the run degrades to a structured truncated
/// result instead of propagating the error: the turns already ran — tools
/// executed, events gathered, usage billed — and discarding all of it because
/// the last call failed would throw away real work. The degraded output keeps
/// everything accumulated and invents nothing: the failed call produced no
/// prose, so the answer is empty. Cancellation is not a salvage failure to
/// work around and still propagates.
///
/// When the model never designated an answer (it hit the budget first), the
/// output's `answer_sql` is the last statement that completed successfully —
/// the best available answer from work already done — or `None` when nothing
/// succeeded. A failed statement is never nominated.
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
    best_successful_sql: Option<String>,
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
    let salvage_call =
        receive::receive(provider, model, messages, &[], sink, cancellation, events).await;
    let (assistant, turn_usage) = match salvage_call {
        Ok((assistant, turn_usage, _)) => (assistant, turn_usage),
        // A cancellation surfacing from the salvage call is the user stopping
        // the run, not a salvage failure to route around — it must propagate.
        Err(error @ (AgentError::Cancelled | AgentError::Provider(ProviderError::Cancelled))) => {
            return Err(error);
        }
        // The final call failing must not discard the work already done: tools
        // executed, events gathered, usage billed. Degrade to the structured
        // truncated result, keeping everything accumulated — and invent
        // nothing: the failed call produced no prose, so the answer is empty.
        Err(AgentError::Provider(_)) => {
            emit(events, sink, AgentEvent::Complete).await;
            return Ok(AgentOutput {
                answer: String::new(),
                events: std::mem::take(events),
                used_bounded_sql_query,
                tool_metadata,
                usage: *usage,
                learning_usage: None,
                truncated: true,
                answer_sql: best_successful_sql,
            });
        }
        Err(error) => return Err(error),
    };
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
    // When the run ended because a budget ran out rather than by the model
    // finishing, surface the best available answer if the model never nominated
    // one. Prefer the last statement that completed successfully — never a
    // failed one (only successes are tracked) — so a wrong nomination is not
    // invented. If nothing succeeded, nominate nothing: an absent answer is
    // better than a wrong one.
    Ok(AgentOutput {
        answer: assistant.content,
        events: std::mem::take(events),
        used_bounded_sql_query,
        tool_metadata,
        usage: *usage,
        learning_usage: None,
        truncated: true,
        answer_sql: best_successful_sql,
    })
}
