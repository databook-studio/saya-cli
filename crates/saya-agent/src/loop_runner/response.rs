use crate::{AgentEvent, ChatMessage};

use super::{
    AgentError, PartialOutput, admission, clarification, designation, emit, preflight,
    response_tools, runner::RunState, tool_batch,
};

pub(super) async fn handle(
    state: &mut RunState<'_>,
    admission: &mut admission::Admission,
    assistant: ChatMessage,
    reasoning: Option<String>,
) -> Result<Option<super::AgentOutput>, AgentError> {
    if !assistant.tool_calls.is_empty()
        && let Err(error) = admission.enforce_tools(&state.cancellation)
        && let Some(dimension) = admission.stopped_dimension(&error, &state.cancellation)
    {
        return Ok(Some(
            super::budget_output(
                dimension,
                &mut state.events,
                state.sink,
                PartialOutput {
                    usage: state.usage,
                    used_bounded_sql_query: state.used_bounded_sql_query,
                    tool_metadata: std::mem::take(&mut state.tool_metadata),
                    answer_sql: state
                        .designation
                        .sql
                        .clone()
                        .or(state.last_successful_sql.clone()),
                    answer: assistant.content,
                },
            )
            .await,
        ));
    }
    let mut response_limits = state.limits;
    let result_caps = if !assistant.tool_calls.is_empty() {
        let result_budget = match preflight::reserve_tool_batch(
            &state.request.model,
            &mut state.messages,
            state.protected_prefix_len,
            &assistant,
            &state.definitions,
            state.limits.context_byte_budget,
            state.request_preflight,
        ) {
            Ok(budget) => budget,
            Err(preflight::PreflightError::Budget(dimension)) => {
                let error = admission.stop_preflight(&state.cancellation, dimension);
                if let Some(dimension) = admission.stopped_dimension(&error, &state.cancellation) {
                    state.messages.push(assistant.clone());
                    state
                        .messages
                        .extend(preflight::non_execution_replies(&assistant));
                    return Ok(Some(
                        super::budget_output(
                            dimension,
                            &mut state.events,
                            state.sink,
                            PartialOutput {
                                usage: state.usage,
                                used_bounded_sql_query: state.used_bounded_sql_query,
                                tool_metadata: std::mem::take(&mut state.tool_metadata),
                                answer_sql: state
                                    .designation
                                    .sql
                                    .clone()
                                    .or(state.last_successful_sql.clone()),
                                answer: assistant.content,
                            },
                        )
                        .await,
                    ));
                }
                return Err(error);
            }
            Err(preflight::PreflightError::Serialization) => {
                return Err(AgentError::Limit("request projection serialization failed"));
            }
        };
        response_limits.context_byte_budget =
            response_limits.context_byte_budget.min(result_budget);
        tool_batch::turn_result_caps(result_budget, assistant.tool_calls.len())
    } else {
        Vec::new()
    };
    // Forward the turn's captured chain-of-thought onto the event stream as
    // one `ReasoningText` event — the provider layer puts it on `ChatResponse.reasoning`
    // and bound it to `_reasoning` here; the CLI-boundary slice carries it across the crate
    // boundary so the CLI *can* reach it; whether to display it is the CLI's call.
    // This is the only way reasoning leaves `saya-agent`: it is NOT pushed
    // onto `messages` — `assistant` (a `ChatMessage`) is what gets replayed
    // to the provider as history, and `ChatMessage` has no reasoning field,
    // so reasoning cannot leak into the next turn's request. `None` (a
    // provider that reported no reasoning) emits nothing — byte-identical
    // to today.
    if let Some(text) = reasoning
        && !text.is_empty()
    {
        emit(
            &mut state.events,
            state.sink,
            AgentEvent::reasoning_text(text),
        )
        .await;
    }
    state.messages.push(assistant.clone());
    // Designation arm: see `designation` for the bounded prose recovery.
    match designation::handle(
        &mut state.designation,
        &assistant,
        &result_caps,
        (&mut state.events, state.sink, &state.cancellation),
        &mut state.messages,
        (
            state.usage,
            state.used_bounded_sql_query,
            &mut state.tool_metadata,
        ),
    )
    .await?
    {
        designation::Outcome::Done(output) => return Ok(Some(*output)),
        designation::Outcome::Recovering => return Ok(None),
        designation::Outcome::NotDesignated => {}
    }
    // The clarification arm (B3c): a `request_clarification` call ends the
    // turn — the question is surfaced, the tool result is fed, and no
    // further provider call happens until the user answers. A malformed
    // ask is fed back as a tool error and the turn continues.
    match clarification::handle(
        &assistant,
        &state.definitions,
        &response_limits,
        &result_caps,
        state.approval,
        state.tools,
        &state.designation,
        (&mut state.events, state.sink, &state.cancellation),
        &mut state.messages,
        (
            state.usage,
            state.used_bounded_sql_query,
            &mut state.tool_metadata,
        ),
    )
    .await?
    {
        clarification::Outcome::Done(output) => return Ok(Some(*output)),
        // The arm answered this message's calls (errors for malformed
        // asks, refusals for the siblings): its results are already on
        // the conversation, and the next turn retries — the ordinary
        // path must not run the same calls again.
        clarification::Outcome::Continue => return Ok(None),
        clarification::Outcome::NotCalled => {}
    }
    if assistant.tool_calls.is_empty() {
        if !matches!(
            state.cancellation.reason(),
            Some(crate::CancellationReason::Budget(_))
        ) {
            super::check_cancelled(&state.cancellation)?;
        }
        emit(&mut state.events, state.sink, AgentEvent::Complete).await;
        return Ok(Some(super::AgentOutput {
            answer: assistant.content,
            events: std::mem::take(&mut state.events),
            used_bounded_sql_query: state.used_bounded_sql_query,
            tool_metadata: std::mem::take(&mut state.tool_metadata),
            usage: state.usage,
            learning_usage: None,
            truncated: false,
            answer_sql: state.designation.sql.clone(),
        }));
    }
    response_tools::execute(state, admission, assistant, result_caps, response_limits).await
}
