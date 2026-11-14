//! Salvaging an agent run that hit a turn or tool-call ceiling: answer from
//! work already done instead of discarding it.

use super::{add_usage, admission::Admission, emit, receive, tool_diagnostics, tools};
use crate::{
    ActorCorrelationDigest, AgentError, AgentEvent, AgentEventSink, AgentOutput, CancellationToken,
    ChatProvider, CorrelationId, ProviderError, TokenUsage,
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
/// When the model designated an answer before the ceiling hit, the output's
/// `answer_sql` is that designation — the query it committed to as the answer.
/// Otherwise it is the last statement that completed successfully — the best
/// available answer from work already done — or `None` when nothing
/// succeeded. A failed statement is never nominated.
#[allow(clippy::too_many_arguments)]
pub(super) async fn salvage(
    provider: &dyn ChatProvider,
    model: &str,
    messages: &mut Vec<crate::ChatMessage>,
    ordinary_history_pairs: &mut usize,
    protected_prefix_len: &mut usize,
    pending: &[crate::ToolCall],
    checkpoint: &super::coding_checkpoint::CodingCheckpoint,
    context_byte_budget: usize,
    request_preflight: Option<super::preflight::RequestPreflightConfig>,
    sink: &dyn AgentEventSink,
    cancellation: &CancellationToken,
    events: &mut Vec<AgentEvent>,
    usage: &mut TokenUsage,
    admission: &mut Admission,
    used_bounded_sql_query: bool,
    tool_metadata: Vec<crate::ToolMetadata>,
    // The SQL to carry on the output: the run's designation when the model
    // made one, else the last statement that completed successfully.
    designated_or_best_sql: Option<String>,
    effect_admission: Option<&dyn crate::EffectAdmission>,
    require_physical_effect_admission: bool,
    conversation_correlation: CorrelationId,
    invocation_correlation: CorrelationId,
    actor_correlation: Option<ActorCorrelationDigest>,
    request_ordinal: u64,
    first_request: bool,
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
    let reasoning_effort = super::reasoning_policy::decide(
        messages,
        first_request,
        false,
        provider.configured_reasoning_effort(),
    )
    .effort;
    let salvage_call = receive::receive(receive::ReceiveContext {
        provider,
        model,
        messages,
        checkpoint,
        ordinary_history_pairs,
        protected_prefix_len,
        definitions: &[],
        context_byte_budget,
        request_preflight,
        continuation: None,
        reasoning_effort,
        remaining_wall: None,
        sink,
        cancellation,
        events,
        admission,
        effect_admission,
        require_physical_effect_admission,
        call_kind: crate::EffectCallKind::Salvage,
        conversation_correlation,
        invocation_correlation,
        actor_correlation,
        request_ordinal,
    })
    .await;
    let (assistant, turn_usage) = match salvage_call {
        Ok((assistant, turn_usage, _)) => (assistant, turn_usage),
        // A cancellation surfacing from the salvage call is the user stopping
        // the run, not a salvage failure to route around — it must propagate.
        Err(
            failure @ super::attempt::ReceiveFailure {
                error: AgentError::Cancelled | AgentError::Provider(ProviderError::Cancelled),
                ..
            },
        ) => {
            add_usage(usage, failure.usage);
            return Err(failure.error);
        }
        // The final call failing must not discard the work already done: tools
        // executed, events gathered, usage billed. Degrade to the structured
        // truncated result, keeping everything accumulated — and invent
        // nothing: the failed call produced no prose, so the answer is empty.
        Err(
            failure @ super::attempt::ReceiveFailure {
                error: AgentError::Provider(_) | AgentError::InvalidToolCall,
                ..
            },
        ) => {
            add_usage(usage, failure.usage);
            emit(events, sink, AgentEvent::Complete).await;
            return Ok(AgentOutput {
                answer: String::new(),
                events: std::mem::take(events),
                used_bounded_sql_query,
                tool_metadata,
                usage: *usage,
                learning_usage: None,
                truncated: true,
                answer_sql: designated_or_best_sql,
            });
        }
        Err(super::attempt::ReceiveFailure {
            error,
            usage: settled,
        }) if admission.stopped_dimension(&error, cancellation).is_some() => {
            let dimension = admission
                .stopped_dimension(&error, cancellation)
                .expect("guarded budget reason");
            add_usage(usage, settled);
            return Ok(super::budget_output(
                dimension,
                events,
                sink,
                super::PartialOutput {
                    usage: *usage,
                    used_bounded_sql_query,
                    tool_metadata,
                    answer_sql: designated_or_best_sql,
                    answer: String::new(),
                },
            )
            .await);
        }
        Err(failure) => return Err(failure.error),
    };
    add_usage(usage, turn_usage);
    // The salvage provider has no tool definitions; any new proposed calls
    // are refused regardless of whether the answering ceiling was crossed.
    tool_diagnostics::refused(&assistant.tool_calls, &[], sink).await;
    if !assistant.tool_calls.is_empty()
        && let Err(error) = admission.enforce_tools(cancellation)
    {
        if let Some(dimension) = admission.stopped_dimension(&error, cancellation) {
            return Ok(super::budget_output(
                dimension,
                events,
                sink,
                super::PartialOutput {
                    usage: *usage,
                    used_bounded_sql_query,
                    tool_metadata,
                    answer_sql: designated_or_best_sql,
                    answer: String::new(),
                },
            )
            .await);
        }
        return Err(error);
    }
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
        answer_sql: designated_or_best_sql,
    })
}
