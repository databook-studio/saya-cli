use crate::{AgentEvent, ProviderContinuationPhase};

use super::{
    AgentError, PartialOutput, add_usage, admission, budget_output, emit, receive, response,
    runner::RunState,
};

pub(super) async fn run(
    state: &mut RunState<'_>,
    admission: &mut admission::Admission,
) -> Result<Option<super::AgentOutput>, AgentError> {
    admission.legacy.admit_turn();
    // A truncation is deterministic — re-sending the identical request fails
    // identically, so `receive` never retries it. Within budget the loop
    // instead re-instructs: the partial stays discarded, one user-role note
    // tells the model to resume compactly, and the turn is spent — the
    // The shared admission owner has already charged this turn, so `max_turns` keeps binding.
    let (assistant, turn_usage, reasoning) = match receive::receive(receive::ReceiveContext {
        provider: state.provider,
        model: &state.request.model,
        messages: &mut state.messages,
        ordinary_history_pairs: &mut state.ordinary_history_pairs,
        protected_prefix_len: &mut state.protected_prefix_len,
        definitions: &state.definitions,
        context_byte_budget: state.limits.context_byte_budget,
        request_preflight: state.request_preflight,
        continuation: state.pending_continuation.take(),
        sink: state.sink,
        cancellation: &state.cancellation,
        events: &mut state.events,
        admission,
        effect_admission: state.effect_admission,
    })
    .await
    {
        Ok(turn) => turn,
        Err(failure) => {
            add_usage(&mut state.usage, failure.usage);
            let error = if matches!(&failure.error, AgentError::Cancelled) {
                admission
                    .check(&state.cancellation)
                    .err()
                    .unwrap_or(failure.error)
            } else {
                failure.error
            };
            if matches!(
                &error,
                AgentError::EffectAdmissionDenied | AgentError::EffectOwnerFailure(_)
            ) {
                return Err(error);
            }
            if state.cancellation.reason() == Some(crate::CancellationReason::OwnerFailure) {
                return Err(error);
            }
            match error {
                AgentError::Provider(crate::ProviderError::OutputTruncated { .. })
                    if state
                        .limits
                        .max_continuations
                        .is_some_and(|max| state.continuation_count < max) =>
                {
                    state.continuation_count += 1;
                    state.pending_continuation = Some(receive::ContinuationProgress {
                        index: state.continuation_count as usize,
                        limit: state
                            .limits
                            .max_continuations
                            .expect("the continuation guard checked a finite limit")
                            as usize,
                    });
                    state
                        .messages
                        .push(crate::ChatMessage::text("user", super::CONTINUATION_NOTE));
                    return Ok(None);
                }
                error
                    if admission
                        .stopped_dimension(&error, &state.cancellation)
                        .is_some() =>
                {
                    let dimension = admission
                        .stopped_dimension(&error, &state.cancellation)
                        .expect("guarded budget reason");
                    return Ok(Some(
                        budget_output(
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
                                answer: String::new(),
                            },
                        )
                        .await,
                    ));
                }
                error if state.designation.recovering() => {
                    return Ok(Some(
                        super::designation::failed_follow_up(
                            &state.designation,
                            error,
                            (&mut state.events, state.sink),
                            (
                                state.usage,
                                state.used_bounded_sql_query,
                                std::mem::take(&mut state.tool_metadata),
                            ),
                        )
                        .await?,
                    ));
                }
                AgentError::Provider(crate::ProviderError::OutputTruncated { .. })
                    if state
                        .limits
                        .max_continuations
                        .is_some_and(|max| max > 0 && state.continuation_count >= max) =>
                {
                    // The allowance is spent: this truncation is terminal
                    // here — no budget, cancellation, or designation
                    // recovery arm matched above to own the stop instead.
                    // A disabled continuation (`None`/`Some(0)`) never had
                    // a limit to exhaust and stays silent.
                    emit(
                        &mut state.events,
                        state.sink,
                        AgentEvent::provider_continuation(
                            ProviderContinuationPhase::Exhausted,
                            state.continuation_count,
                            state
                                .limits
                                .max_continuations
                                .expect("the guard checked a finite limit"),
                        ),
                    )
                    .await;
                    return Err(error);
                }
                error => return Err(error),
            }
        }
    };
    // Providers report cumulative counts per response; sum across turns.
    add_usage(&mut state.usage, turn_usage);
    response::handle(state, admission, assistant, reasoning).await
}
