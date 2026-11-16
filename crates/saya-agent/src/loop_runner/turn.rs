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
    state.request_ordinal = state.request_ordinal.saturating_add(1);
    // A truncation is deterministic — re-sending the identical request fails
    // identically, so `receive` never retries it. Within budget the loop
    // instead re-instructs: the partial stays discarded, one user-role note
    // tells the model to resume compactly, and the turn is spent — the
    // The shared admission owner has already charged this turn, so `max_turns` keeps binding.
    let continuation = state.pending_continuation.take();
    let call_kind = if continuation.is_some() {
        crate::EffectCallKind::Continuation
    } else {
        crate::EffectCallKind::Answer
    };
    // Per request: a pinned effort wins; otherwise the first request and the
    // first request after a diagnostic failure think hard, a second
    // consecutive diagnostic failure thinks medium, and the rest lightly.
    let decision = super::reasoning_policy::decide(
        &state.messages,
        state.first_request(),
        state.previous_diagnostic_failure,
        state.provider.configured_reasoning_effort(),
    );
    state.previous_diagnostic_failure = decision.diagnostic_failure;
    let remaining_wall = state
        .progress
        .as_ref()
        .and_then(super::progress::ProgressClassifier::remaining_wall);
    // The job's clock can already be spent before a request is opened. Opening
    // it would only produce a cut, and that cut would spend an allowance the
    // job can never use. Latch the typed stop instead: the run returns the
    // timeout class without calling the provider.
    if remaining_wall == Some(std::time::Duration::ZERO) {
        state
            .cancellation
            .cancel_with_reason(crate::CancellationReason::Budget(
                crate::BudgetDimension::ElapsedTime,
            ));
    }
    let (assistant, turn_usage, reasoning) = match receive::receive(receive::ReceiveContext {
        provider: state.provider,
        model: &state.request.model,
        messages: &mut state.messages,
        checkpoint: &state.checkpoint,
        ordinary_history_pairs: &mut state.ordinary_history_pairs,
        protected_prefix_len: &mut state.protected_prefix_len,
        definitions: &state.definitions,
        context_byte_budget: state.limits.context_byte_budget,
        request_preflight: state.request_preflight,
        continuation,
        reasoning_effort: decision.effort,
        remaining_wall,
        sink: state.sink,
        cancellation: &state.cancellation,
        events: &mut state.events,
        admission,
        effect_admission: state.effect_admission,
        require_physical_effect_admission: state.require_physical_effect_admission,
        call_kind,
        conversation_correlation: state.conversation_correlation,
        invocation_correlation: state.invocation_correlation,
        actor_correlation: state.actor_correlation,
        request_ordinal: state.request_ordinal,
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
                AgentError::EffectAdmissionDenied
                    | AgentError::EffectOwnerFailure(_)
                    | AgentError::UnsupportedPhysicalCoverage
            ) {
                return Err(error);
            }
            if state.cancellation.reason() == Some(crate::CancellationReason::OwnerFailure) {
                return Err(error);
            }
            match error {
                AgentError::Provider(crate::ProviderError::OutputTruncated {
                    partial_text,
                    partial_tool_json,
                }) if state
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
                    // A reply that spent its whole budget on reasoning produced
                    // nothing to append to; tell the model to act, not to resume.
                    let note = if partial_text.trim().is_empty() && partial_tool_json.is_empty() {
                        super::REASONING_ONLY_CONTINUATION_NOTE
                    } else {
                        super::CONTINUATION_NOTE
                    };
                    state.messages.push(crate::ChatMessage::text("user", note));
                    return Ok(None);
                }
                // A cut that delivered content is deterministic too: the
                // identical request would be cut at the same point, so it is
                // never re-sent. It is answered with the cut's own note under
                // its own consecutive bound: a cut is a clock, not an output
                // window, so it must not spend the truncation allowance.
                AgentError::Provider(crate::ProviderError::StreamCut {
                    elapsed_secs,
                    estimated_tokens,
                }) if state.limits.max_continuations.is_some_and(|max| max > 0)
                    && state.consecutive_stream_cuts + 1
                        < super::stream_cut::MAX_CONSECUTIVE_STREAM_CUTS =>
                {
                    state.consecutive_stream_cuts += 1;
                    state.pending_continuation = Some(receive::ContinuationProgress {
                        index: state.consecutive_stream_cuts as usize,
                        limit: super::stream_cut::MAX_CONSECUTIVE_STREAM_CUTS as usize,
                    });
                    // A time-cut is a clock, not an output window. Visible
                    // scraps do not lower later requests (that pinned a run
                    // to 1,024). The event still names the ceiling continued
                    // under: the operator's, unchanged.
                    let configured = state.provider.max_output_tokens();
                    let ceiling =
                        super::stream_cut::sticky_ceiling_after_cut(estimated_tokens, configured)
                            .or(configured)
                            .unwrap_or(0);
                    // Never silent: the operator sees the cut's own clock and
                    // estimate, and the ceiling the run continues under.
                    emit(
                        &mut state.events,
                        state.sink,
                        AgentEvent::stream_cut_recovered(elapsed_secs, estimated_tokens, ceiling),
                    )
                    .await;
                    state.messages.push(crate::ChatMessage::text(
                        "user",
                        super::stream_cut::continuation_note(elapsed_secs),
                    ));
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
                    // The truncation allowance is spent: this truncation is
                    // terminal here — no budget, cancellation, or designation
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
                AgentError::Provider(crate::ProviderError::StreamCut { .. })
                    if state.limits.max_continuations.is_some_and(|max| max > 0)
                        && state.consecutive_stream_cuts + 1
                            >= super::stream_cut::MAX_CONSECUTIVE_STREAM_CUTS =>
                {
                    // Consecutive cuts never resumed: the job ends under the
                    // cut's own class. The allowance is named so the operator
                    // can tell this from a single cut or an output limit.
                    state.consecutive_stream_cuts += 1;
                    emit(
                        &mut state.events,
                        state.sink,
                        AgentEvent::provider_continuation(
                            ProviderContinuationPhase::Exhausted,
                            state.consecutive_stream_cuts,
                            super::stream_cut::MAX_CONSECUTIVE_STREAM_CUTS,
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
