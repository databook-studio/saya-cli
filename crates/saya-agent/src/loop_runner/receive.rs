use super::{add_usage, attempt, emit};
use crate::{
    AgentEvent, AgentEventSink, CancellationToken, ChatMessage, ChatProvider, ChatRequest,
    ProviderError, ProviderEvent, ProviderRecoveryPhase, ProviderRecoveryReason, TokenUsage,
    ToolDefinition, UsageCall,
};
use futures_util::StreamExt;
use std::time::Duration;

/// Streams one provider turn and returns the assembled assistant message. A
/// failed attempt's latest reported cumulative usage remains billable even
/// though its content is discarded.
pub(super) async fn receive(
    provider: &dyn ChatProvider,
    model: &str,
    messages: &[ChatMessage],
    definitions: &[ToolDefinition],
    sink: &dyn AgentEventSink,
    cancellation: &CancellationToken,
    events: &mut Vec<AgentEvent>,
) -> Result<(ChatMessage, TokenUsage, Option<String>), attempt::ReceiveFailure> {
    let request = ChatRequest {
        model: model.into(),
        messages: messages.into(),
        tools: definitions.into(),
        ..Default::default()
    };
    let delays = crate::providers::default_retry_delays();
    let mut retries = 0_u8;
    let mut recovered_usage = TokenUsage::default();
    loop {
        match stream_attempt(provider, &request, sink, cancellation, events).await {
            attempt::AttemptOutcome::Success {
                message,
                usage,
                reasoning,
            } => {
                if let Some(usage) = usage {
                    emit(events, sink, AgentEvent::usage(UsageCall::Answer, usage)).await;
                    add_usage(&mut recovered_usage, usage);
                }
                return Ok((message, recovered_usage, reasoning));
            }
            attempt::AttemptOutcome::Failed { error, usage } => {
                if let Some(usage) = usage {
                    emit(events, sink, AgentEvent::failed_attempt_usage(usage)).await;
                    add_usage(&mut recovered_usage, usage);
                }
                match error {
                    attempt::AttemptError::Retryable { error: _, reason }
                        if retries < attempt::RETRY_LIMIT =>
                    {
                        let next_attempt = retries.saturating_add(1);
                        emit(
                            events,
                            sink,
                            AgentEvent::provider_recovery(
                                ProviderRecoveryPhase::Retrying,
                                reason,
                                next_attempt,
                                attempt::RETRY_LIMIT,
                            ),
                        )
                        .await;
                        emit(events, sink, AgentEvent::turn_reset()).await;
                        if let Err(error) = wait(delays[usize::from(retries)], cancellation).await {
                            emit(
                                events,
                                sink,
                                AgentEvent::provider_recovery(
                                    ProviderRecoveryPhase::NotRetried,
                                    ProviderRecoveryReason::Cancelled,
                                    next_attempt,
                                    attempt::RETRY_LIMIT,
                                ),
                            )
                            .await;
                            return Err(attempt::ReceiveFailure {
                                error,
                                usage: recovered_usage,
                            });
                        }
                        retries = next_attempt;
                    }
                    attempt::AttemptError::Retryable { error, reason } => {
                        emit(
                            events,
                            sink,
                            AgentEvent::provider_recovery(
                                ProviderRecoveryPhase::Exhausted,
                                reason,
                                retries.saturating_add(1),
                                attempt::RETRY_LIMIT,
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
                                attempt::RETRY_LIMIT,
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

async fn stream_attempt(
    provider: &dyn ChatProvider,
    request: &ChatRequest,
    sink: &dyn AgentEventSink,
    cancellation: &CancellationToken,
    events: &mut Vec<AgentEvent>,
) -> attempt::AttemptOutcome {
    emit(events, sink, AgentEvent::turn_started()).await;
    let mut stream = tokio::select! {
        _ = cancellation.cancelled() => return failed(attempt::AttemptError::Cancelled, None),
        result = provider.stream(request.clone(), cancellation.clone()) => match result {
            Ok(stream) => stream,
            Err(error) => return failed(provider_failure(error), None),
        },
    };
    let (mut content, mut calls, mut complete) = (String::new(), Vec::new(), false);
    let mut usage = None;
    let mut reasoning = None;
    loop {
        let next = tokio::select! {
            _ = cancellation.cancelled() => return failed(attempt::AttemptError::Cancelled, usage),
            next = stream.next() => next,
        };
        let Some(event) = next else {
            break;
        };
        let event = match event {
            Ok(event) => event,
            Err(error) => return failed(provider_failure(error), usage),
        };
        match event {
            ProviderEvent::TextDelta(text) => {
                if content.len().saturating_add(text.len()) > crate::MAX_STREAM_BYTES {
                    return failed(byte_limit_error(), usage);
                }
                content.push_str(&text);
                emit(events, sink, AgentEvent::assistant_text(text)).await;
            }
            ProviderEvent::ReasoningDelta(text) => {
                let accumulated = reasoning.get_or_insert_with(String::new);
                if accumulated.len().saturating_add(text.len()) > crate::MAX_STREAM_BYTES {
                    return failed(byte_limit_error(), usage);
                }
                accumulated.push_str(&text);
            }
            ProviderEvent::ToolCalls(value) => calls.extend(value),
            ProviderEvent::Usage(counts) => usage = Some(counts),
            ProviderEvent::Done => complete = true,
        }
    }
    if !complete {
        return failed(
            attempt::AttemptError::Retryable {
                error: ProviderError::InvalidResponse,
                reason: ProviderRecoveryReason::StreamEnded,
            },
            usage,
        );
    }
    if content.trim().is_empty() && calls.is_empty() {
        return failed(
            attempt::AttemptError::Terminal {
                error: ProviderError::InvalidResponse,
                reason: ProviderRecoveryReason::EmptyResponse,
            },
            usage,
        );
    }
    attempt::AttemptOutcome::Success {
        message: ChatMessage {
            role: "assistant".into(),
            content,
            tool_calls: calls,
            tool_call_id: None,
        },
        usage,
        reasoning,
    }
}

fn byte_limit_error() -> attempt::AttemptError {
    attempt::AttemptError::Retryable {
        error: ProviderError::Request("provider stream exceeded size limit".into()),
        reason: ProviderRecoveryReason::StreamByteLimit,
    }
}

fn provider_failure(error: ProviderError) -> attempt::AttemptError {
    match error {
        error @ ProviderError::OutputTruncated { .. } => attempt::AttemptError::Terminal {
            error,
            reason: ProviderRecoveryReason::OutputTruncated,
        },
        ProviderError::Cancelled => attempt::AttemptError::Cancelled,
        error => attempt::AttemptError::Retryable {
            error,
            reason: ProviderRecoveryReason::ProviderFailure,
        },
    }
}

fn failed(error: attempt::AttemptError, usage: Option<TokenUsage>) -> attempt::AttemptOutcome {
    attempt::AttemptOutcome::Failed { error, usage }
}

#[cfg(test)]
#[path = "receive_tests.rs"]
mod tests;
