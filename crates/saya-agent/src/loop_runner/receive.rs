use super::{
    add_usage, admission::Admission, attempt, emit, preflight, receive_stream::stream_attempt,
    tool_protocol,
};
use crate::{
    AgentEvent, AgentEventSink, CancellationToken, ChatMessage, ChatProvider, ChatRequest,
    ProviderRecoveryPhase, ProviderRecoveryReason, TokenUsage, ToolDefinition, UsageCall,
};
use std::time::Duration;

/// Streams one provider turn and returns the assembled assistant message. A
/// failed attempt's latest reported cumulative usage remains billable even
/// though its content is discarded.
pub(super) struct ReceiveContext<'a> {
    pub(super) provider: &'a dyn ChatProvider,
    pub(super) model: &'a str,
    pub(super) messages: &'a mut Vec<ChatMessage>,
    pub(super) ordinary_history_pairs: &'a mut usize,
    pub(super) protected_prefix_len: &'a mut usize,
    pub(super) definitions: &'a [ToolDefinition],
    pub(super) context_byte_budget: usize,
    pub(super) request_preflight: Option<super::preflight::RequestPreflightConfig>,
    pub(super) sink: &'a dyn AgentEventSink,
    pub(super) cancellation: &'a CancellationToken,
    pub(super) events: &'a mut Vec<AgentEvent>,
    pub(super) admission: &'a mut Admission,
}

pub(super) async fn receive(
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
        sink,
        cancellation,
        events,
        admission,
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
    loop {
        match stream_attempt(provider, &request, sink, cancellation, events).await {
            attempt::AttemptOutcome::Success {
                message,
                usage,
                reasoning,
            } => {
                let observation = admission
                    .observe_tools(message.tool_calls.len() as u64, sink, cancellation)
                    .await;
                if let Some(usage) = usage {
                    emit(events, sink, AgentEvent::usage(UsageCall::Answer, usage)).await;
                    add_usage(&mut recovered_usage, usage);
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
                return Ok((message, recovered_usage, reasoning));
            }
            attempt::AttemptOutcome::Failed { error, usage } => {
                if let Some(usage) = usage {
                    emit(events, sink, AgentEvent::failed_attempt_usage(usage)).await;
                    add_usage(&mut recovered_usage, usage);
                    if let Err(error) = admission.settle(Some(usage), cancellation) {
                        return Err(attempt::ReceiveFailure {
                            error,
                            usage: recovered_usage,
                        });
                    }
                }
                match error {
                    attempt::AttemptError::Retryable { error: _, reason }
                        if retries < attempt::RETRY_LIMIT =>
                    {
                        let next_attempt = retries.saturating_add(2);
                        emit(events, sink, AgentEvent::turn_reset()).await;
                        emit(
                            events,
                            sink,
                            AgentEvent::provider_recovery(
                                ProviderRecoveryPhase::Retrying,
                                reason,
                                next_attempt,
                                attempt::MAX_ATTEMPTS,
                            ),
                        )
                        .await;
                        if let Err(error) = wait(delays[usize::from(retries)], cancellation).await {
                            emit(
                                events,
                                sink,
                                AgentEvent::provider_recovery(
                                    ProviderRecoveryPhase::NotRetried,
                                    ProviderRecoveryReason::Cancelled,
                                    retries.saturating_add(1),
                                    attempt::MAX_ATTEMPTS,
                                ),
                            )
                            .await;
                            return Err(attempt::ReceiveFailure {
                                error,
                                usage: recovered_usage,
                            });
                        }
                        retries = retries.saturating_add(1);
                    }
                    attempt::AttemptError::Retryable { error, reason } => {
                        emit(
                            events,
                            sink,
                            AgentEvent::provider_recovery(
                                ProviderRecoveryPhase::Exhausted,
                                reason,
                                retries.saturating_add(1),
                                attempt::MAX_ATTEMPTS,
                            ),
                        )
                        .await;
                        return Err(attempt::ReceiveFailure {
                            error: crate::AgentError::Provider(error),
                            usage: recovered_usage,
                        });
                    }
                    error => {
                        let reason = error.reason();
                        let error = error.error();
                        emit(
                            events,
                            sink,
                            AgentEvent::provider_recovery(
                                ProviderRecoveryPhase::NotRetried,
                                reason,
                                retries.saturating_add(1),
                                attempt::MAX_ATTEMPTS,
                            ),
                        )
                        .await;
                        return Err(attempt::ReceiveFailure {
                            error,
                            usage: recovered_usage,
                        });
                    }
                }
            }
        }
    }
}

/// Sleeps `delay`, aborting locally when the run is cancelled.
async fn wait(delay: Duration, cancellation: &CancellationToken) -> Result<(), crate::AgentError> {
    tokio::select! {
        _ = cancellation.cancelled() => Err(crate::AgentError::Cancelled),
        _ = tokio::time::sleep(delay) => Ok(()),
    }
}

#[cfg(test)]
#[path = "receive_tests.rs"]
mod tests;
