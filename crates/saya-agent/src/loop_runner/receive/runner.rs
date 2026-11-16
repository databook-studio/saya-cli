use super::super::{
    add_usage,
    admission::coding,
    attempt, emit, preflight,
    receive_stream::{StreamAttempt, stream_attempt},
    tool_diagnostics, tool_protocol,
};
use super::{context::ReceiveContext, retry};
use crate::{
    AgentEvent, AgentMetric, AgentMetricCount, AgentMetricOutcome, AgentMetricPhase, ChatMessage,
    ChatRequest, PhysicalRequestCorrelation, ProviderContinuationPhase, ProviderRecoveryPhase,
    ProviderRecoveryReason, TokenUsage, UsageCall,
};

// The failure carries the provider usage settled with it, so the error side is
// larger than the success tuple; boxing it would only move the allocation onto
// an already-failing path.
#[allow(clippy::result_large_err)]
pub(in crate::loop_runner) async fn receive(
    context: ReceiveContext<'_>,
) -> Result<(ChatMessage, TokenUsage, Option<attempt::ReasoningTrace>), attempt::ReceiveFailure> {
    let ReceiveContext {
        provider,
        model,
        messages,
        checkpoint,
        ordinary_history_pairs,
        protected_prefix_len,
        definitions,
        context_byte_budget,
        request_preflight,
        continuation,
        reasoning_effort,
        remaining_wall,
        sink,
        cancellation,
        events,
        admission,
        effect_admission,
        require_physical_effect_admission,
        call_kind,
        conversation_correlation,
        invocation_correlation,
        actor_correlation,
        request_ordinal,
    } = context;
    let mut recovered_usage = TokenUsage::default();
    if let Err(error) = admission.check(cancellation) {
        return Err(attempt::ReceiveFailure {
            error,
            usage: recovered_usage,
        });
    }
    let projection_started = std::time::Instant::now();
    let fitted = preflight::fit_request(
        model,
        messages,
        ordinary_history_pairs,
        protected_prefix_len,
        definitions,
        (context_byte_budget, request_preflight),
        preflight::CompactionContext { sink, checkpoint },
    );
    sink.metric(AgentMetric::Phase {
        phase: AgentMetricPhase::PromptProjection,
        elapsed_ns: super::super::attempt_metrics::duration_ns(projection_started.elapsed()),
        outcome: if fitted.is_ok() {
            AgentMetricOutcome::Succeeded
        } else {
            AgentMetricOutcome::Failed
        },
    })
    .await;
    let projection_size_bytes = match fitted {
        Ok(size) => size,
        Err(error) => {
            let error = match error {
                preflight::PreflightError::Budget(dimension) => {
                    admission.stop_preflight(cancellation, dimension)
                }
                preflight::PreflightError::Serialization => {
                    crate::AgentError::Limit("request projection serialization failed")
                }
            };
            return Err(attempt::ReceiveFailure {
                error,
                usage: recovered_usage,
            });
        }
    };
    let request_correlation = require_physical_effect_admission.then(|| {
        PhysicalRequestCorrelation::new(
            conversation_correlation,
            invocation_correlation,
            actor_correlation,
            request_ordinal,
            crate::protocol::streaming::next_projection_revision(),
            projection_size_bytes,
        )
    });
    let request = ChatRequest {
        model: model.into(),
        messages: messages.as_slice().into(),
        tools: definitions.into(),
        reasoning_effort,
        ..Default::default()
    };
    let delays = super::delays::turn_retry_delays(&provider.retry_delays());
    let mut retries = 0_u8;
    if let Err(error) = admission.request(sink, cancellation).await {
        return Err(attempt::ReceiveFailure {
            error,
            usage: recovered_usage,
        });
    }
    if let Some(progress) = continuation {
        debug_assert!(progress.index > 0 && progress.index <= progress.limit);
        sink.metric(AgentMetric::Count(AgentMetricCount::AdmittedContinuation))
            .await;
        emit(
            events,
            sink,
            AgentEvent::provider_continuation(
                ProviderContinuationPhase::Continuing,
                progress.index as u32,
                progress.limit as u32,
            ),
        )
        .await;
        if let Err(error) = admission.check(cancellation) {
            return Err(attempt::ReceiveFailure {
                error,
                usage: recovered_usage,
            });
        }
    }
    let permit = match coding::admit_model_request(effect_admission, cancellation).await {
        Ok(permit) => permit,
        Err(error) => {
            return Err(attempt::ReceiveFailure {
                error,
                usage: recovered_usage,
            });
        }
    };
    let mut physical_attempts = super::physical::control(
        require_physical_effect_admission,
        effect_admission,
        permit,
        call_kind,
        request_correlation,
        cancellation,
    )
    .map_err(|error| attempt::ReceiveFailure {
        error,
        usage: recovered_usage,
    })?;
    let mut reported_usage = effect_admission.map(|_| None);
    let mut usage_complete = true;
    let mut response_received = false;
    // The job's wall clock, anchored once: every attempt and every backoff
    // spends the same budget instead of resetting it.
    let wall_deadline = remaining_wall.map(|remaining| tokio::time::Instant::now() + remaining);
    let receive_result = async {
        loop {
            // Recomputed per attempt. A retry gets what the job has left less
            // the reserve its own backoff and the next attempt need; the
            // first attempt is the job's own work and spends the clock it has,
            // so a job budgeted at or below the reserve can still start.
            let attempt_wall = wall_deadline.map(|deadline| {
                let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
                if retries > 0 {
                    remaining.saturating_sub(retry::RETRY_RESERVE)
                } else {
                    remaining
                }
            });
            let deadline = super::super::receive_stream::RequestDeadline::new(
                provider.request_timeout(),
                attempt_wall,
            );
            match stream_attempt(
                StreamAttempt {
                    provider,
                    request: &request,
                    sink,
                    cancellation,
                    retry: retries > 0,
                    deadline,
                },
                events,
                physical_attempts.as_mut(),
            )
            .await
            {
                attempt::AttemptOutcome::Success {
                    message,
                    usage,
                    reasoning,
                } => {
                    response_received = true;
                    let observation = admission
                        .observe_tools(message.tool_calls.len() as u64, sink, cancellation)
                        .await;
                    if let Some(usage) = usage {
                        emit(events, sink, AgentEvent::usage(UsageCall::Answer, usage)).await;
                        add_usage(&mut recovered_usage, usage);
                        if let Some(reported_usage) = reported_usage.as_mut() {
                            add_reported_usage(reported_usage, usage);
                        }
                    } else if effect_admission.is_some() {
                        usage_complete = false;
                    }
                    let settlement = admission.settle(usage, cancellation);
                    if let Err(error) = observation {
                        tool_diagnostics::refused(&message.tool_calls, definitions, sink).await;
                        return Err(attempt::ReceiveFailure {
                            error,
                            usage: recovered_usage,
                        });
                    }
                    if let Err(error) = settlement
                        && !matches!(&error, crate::AgentError::Limit("answering budget stopped"))
                    {
                        tool_diagnostics::refused(&message.tool_calls, definitions, sink).await;
                        return Err(attempt::ReceiveFailure {
                            error,
                            usage: recovered_usage,
                        });
                    }
                    if !tool_protocol::valid_ids(&message.tool_calls, definitions, sink).await {
                        emit(
                            events,
                            sink,
                            AgentEvent::provider_recovery(
                                ProviderRecoveryPhase::NotRetried,
                                ProviderRecoveryReason::ToolCallProtocol,
                                retries.saturating_add(1),
                                super::delays::max_attempts(&delays),
                            ),
                        )
                        .await;
                        return Err(attempt::ReceiveFailure {
                            error: crate::AgentError::InvalidToolCall,
                            usage: recovered_usage,
                        });
                    }
                    break Ok((message, recovered_usage, reasoning));
                }
                attempt::AttemptOutcome::Failed { error, usage } => {
                    let error = match error.into_control_failure() {
                        Ok(error) => error,
                        Err(error) => {
                            break Err(attempt::ReceiveFailure {
                                error,
                                usage: recovered_usage,
                            });
                        }
                    };
                    // The provider answered (with a truncated result): the
                    // logical request is a received response, not an unknown.
                    if super::super::receive_stream::is_known_truncation(&error) {
                        response_received = true;
                    }
                    if let Some(usage) = usage
                        && let Some(reported_usage) = reported_usage.as_mut()
                    {
                        add_reported_usage(reported_usage, usage);
                    } else if effect_admission.is_some() {
                        usage_complete = false;
                    }
                    if let Some(failure) = retry::settle_failed_usage(
                        &error,
                        usage,
                        admission,
                        sink,
                        cancellation,
                        events,
                        &mut recovered_usage,
                    )
                    .await
                    {
                        break Err(failure);
                    }
                    match retry::plan_retry(
                        error,
                        retries,
                        &delays,
                        wall_deadline,
                        retry::RetryContext {
                            admission: &mut *admission,
                            sink,
                            cancellation,
                            events: &mut *events,
                        },
                    )
                    .await
                    {
                        retry::RetryOutcome::Retry(next) => retries = next,
                        retry::RetryOutcome::GiveUp(error) => {
                            break Err(attempt::ReceiveFailure {
                                error,
                                usage: recovered_usage,
                            });
                        }
                    }
                }
            }
        }
    }
    .await;
    if let Err(error) = super::physical::settle_request(
        effect_admission,
        permit,
        response_received,
        &physical_attempts,
        reported_usage.flatten(),
        usage_complete,
        cancellation,
    )
    .await
    {
        let usage = match &receive_result {
            Ok((_, usage, _)) => *usage,
            Err(failure) => failure.usage,
        };
        return Err(attempt::ReceiveFailure { error, usage });
    }
    receive_result
}

fn add_reported_usage(total: &mut Option<TokenUsage>, turn: TokenUsage) {
    match total {
        Some(total) => add_usage(total, turn),
        None => *total = Some(turn),
    }
}
