use super::super::{
    add_usage, admission::coding, attempt, emit, preflight, receive_stream::stream_attempt,
    tool_protocol,
};
use super::{context::ReceiveContext, retry};
use crate::{
    AgentEvent, AgentMetric, AgentMetricCount, ChatMessage, ChatRequest, ProviderContinuationPhase,
    ProviderRecoveryPhase, ProviderRecoveryReason, TokenUsage, UsageCall,
};

pub(in crate::loop_runner) async fn receive(
    context: ReceiveContext<'_>,
) -> Result<(ChatMessage, TokenUsage, Option<String>), attempt::ReceiveFailure> {
    let ReceiveContext {
        provider,
        model,
        messages,
        ordinary_history_pairs,
        protected_prefix_len,
        definitions,
        context_byte_budget,
        request_preflight,
        continuation,
        sink,
        cancellation,
        events,
        admission,
        effect_admission,
    } = context;
    let mut recovered_usage = TokenUsage::default();
    if let Err(error) = admission.check(cancellation) {
        return Err(attempt::ReceiveFailure {
            error,
            usage: recovered_usage,
        });
    }
    if let Err(error) = preflight::fit_request(
        model,
        messages,
        ordinary_history_pairs,
        protected_prefix_len,
        definitions,
        context_byte_budget,
        request_preflight,
    ) {
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
    let request = ChatRequest {
        model: model.into(),
        messages: messages.as_slice().into(),
        tools: definitions.into(),
        ..Default::default()
    };
    let delays = crate::providers::default_retry_delays();
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
    let mut reported_usage = effect_admission.map(|_| None);
    let mut usage_complete = true;
    let mut response_received = false;
    let receive_result = async {
        loop {
            match stream_attempt(provider, &request, sink, cancellation, events, retries > 0).await
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
                        return Err(attempt::ReceiveFailure {
                            error,
                            usage: recovered_usage,
                        });
                    }
                    if let Err(error) = settlement
                        && !matches!(&error, crate::AgentError::Limit("answering budget stopped"))
                    {
                        return Err(attempt::ReceiveFailure {
                            error,
                            usage: recovered_usage,
                        });
                    }
                    if !tool_protocol::valid_ids(&message.tool_calls) {
                        emit(
                            events,
                            sink,
                            AgentEvent::provider_recovery(
                                ProviderRecoveryPhase::NotRetried,
                                ProviderRecoveryReason::ToolCallProtocol,
                                retries.saturating_add(1),
                                attempt::MAX_ATTEMPTS,
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
                        admission,
                        sink,
                        cancellation,
                        events,
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
    let outcome = if response_received {
        crate::EffectOutcome::Succeeded
    } else {
        crate::EffectOutcome::Unknown
    };
    if let Err(error) = coding::settle_model_request(
        effect_admission,
        permit,
        outcome,
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
