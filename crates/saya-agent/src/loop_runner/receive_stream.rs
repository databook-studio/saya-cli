use super::{attempt, attempt_metrics::AttemptTiming, emit};
use crate::protocol::tool_collection::admit_tool_calls;
use crate::{
    AgentEvent, AgentEventSink, AgentMetric, AgentMetricCount, AgentMetricOutcome,
    AgentMetricUseful, CancellationToken, ChatMessage, ChatProvider, ChatRequest, ProviderError,
    ProviderEvent, ProviderRecoveryReason, TokenUsage,
};
use futures_util::StreamExt;
use std::time::Instant;

pub(super) async fn stream_attempt(
    provider: &dyn ChatProvider,
    request: &ChatRequest,
    sink: &dyn AgentEventSink,
    cancellation: &CancellationToken,
    events: &mut Vec<AgentEvent>,
    retry: bool,
) -> attempt::AttemptOutcome {
    let mut timing = AttemptTiming::new();
    let outcome = if retry {
        sink.metric(AgentMetric::Count(AgentMetricCount::RetryAttemptStarted))
            .await;
        if cancellation.is_cancelled() {
            failed(attempt::AttemptError::Cancelled, None)
        } else {
            stream_attempt_inner(provider, request, sink, cancellation, events, &mut timing).await
        }
    } else {
        stream_attempt_inner(provider, request, sink, cancellation, events, &mut timing).await
    };
    timing.publish(sink, &outcome).await;
    outcome
}

async fn stream_attempt_inner(
    provider: &dyn ChatProvider,
    request: &ChatRequest,
    sink: &dyn AgentEventSink,
    cancellation: &CancellationToken,
    events: &mut Vec<AgentEvent>,
    timing: &mut AttemptTiming,
) -> attempt::AttemptOutcome {
    emit(events, sink, AgentEvent::turn_started()).await;
    if cancellation.is_cancelled() {
        return failed(attempt::AttemptError::Cancelled, None);
    }
    let provider_request = request.clone();
    let provider_cancellation = cancellation.clone();
    let establishment_start = timing.begin_establishment();
    let establishment = tokio::select! {
        _ = cancellation.cancelled() => {
            timing.finish_establishment(establishment_start, AgentMetricOutcome::Cancelled);
            return failed(attempt::AttemptError::Cancelled, None);
        },
        result = provider.stream(provider_request, provider_cancellation) => result,
    };
    let mut stream = match establishment {
        Ok(stream) => {
            timing.finish_establishment(establishment_start, AgentMetricOutcome::Succeeded);
            stream
        }
        Err(error) => {
            let phase_outcome = if matches!(&error, ProviderError::Cancelled) {
                AgentMetricOutcome::Cancelled
            } else {
                AgentMetricOutcome::Failed
            };
            timing.finish_establishment(establishment_start, phase_outcome);
            return failed(establishment_failure(error), None);
        }
    };
    let (mut content, mut calls, mut complete, mut usage, mut reasoning) =
        (String::new(), Vec::new(), false, None, None);
    let mut useful_text_seen = false;
    let mut tool_call_bytes = 0usize;
    let mut items = crate::protocol::stream_item_limit::StreamItemCount::default();
    loop {
        let read_started = Instant::now();
        let next = tokio::select! {
            _ = cancellation.cancelled() => {
                timing.add_read_elapsed(read_started.elapsed());
                return failed(attempt::AttemptError::Cancelled, usage);
            }
            next = stream.next() => next,
        };
        timing.add_read_elapsed(read_started.elapsed());
        let Some(event) = next else { break };
        let event = match event {
            Ok(event) => {
                timing.first_event(AgentMetricOutcome::Succeeded);
                event
            }
            Err(error) => {
                timing.first_event(AgentMetricOutcome::Failed);
                let usage = match &error {
                    ProviderError::StreamItemLimit {
                        usage: Some(reported),
                    } => Some(**reported),
                    _ => usage,
                };
                return failed(provider_failure(error), usage);
            }
        };
        if let ProviderEvent::Usage(counts) = &event {
            usage = Some(*counts);
        }
        if let Err(error) = items.admit(usage) {
            return failed(provider_failure(error), usage);
        }
        match event {
            ProviderEvent::TextDelta(text) => {
                if !useful_text_seen && !text.trim().is_empty() {
                    useful_text_seen = true;
                    sink.metric(AgentMetric::Useful(AgentMetricUseful::NonWhitespaceText))
                        .await;
                    if cancellation.is_cancelled() {
                        return failed(attempt::AttemptError::Cancelled, usage);
                    }
                }
                if content.len().saturating_add(text.len()) > crate::MAX_STREAM_BYTES {
                    return failed(byte_limit_error(), usage);
                }
                content.push_str(&text);
                if !text.is_empty() {
                    emit(events, sink, AgentEvent::assistant_text(text)).await;
                }
            }
            ProviderEvent::ReasoningDelta(text) => {
                let accumulated = reasoning.get_or_insert_with(String::new);
                if accumulated.len().saturating_add(text.len()) > crate::MAX_STREAM_BYTES {
                    return failed(byte_limit_error(), usage);
                }
                accumulated.push_str(&text);
            }
            ProviderEvent::ToolCalls(value) => {
                if let Err(error) = admit_tool_calls(
                    &mut calls,
                    &mut tool_call_bytes,
                    value,
                    crate::MAX_STREAM_BYTES,
                ) {
                    return failed(provider_failure(error), usage);
                }
            }
            ProviderEvent::Usage(counts) => usage = Some(counts),
            ProviderEvent::Done => complete = true,
        }
    }
    if !complete {
        return failed(
            retryable(
                ProviderError::InvalidResponse,
                ProviderRecoveryReason::StreamEnded,
            ),
            usage,
        );
    }
    if content.trim().is_empty() && calls.is_empty() {
        return failed(
            terminal(
                ProviderError::InvalidResponse,
                ProviderRecoveryReason::EmptyResponse,
            ),
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
    retryable(
        ProviderError::Request("provider stream exceeded size limit".into()),
        ProviderRecoveryReason::StreamByteLimit,
    )
}

fn provider_failure(error: ProviderError) -> attempt::AttemptError {
    match error {
        error @ ProviderError::OutputTruncated { .. } => {
            terminal(error, ProviderRecoveryReason::OutputTruncated)
        }
        ProviderError::ToolCollectionLimit => terminal(
            ProviderError::ToolCollectionLimit,
            ProviderRecoveryReason::ToolCollectionLimit,
        ),
        error @ ProviderError::StreamItemLimit { .. } => {
            terminal(error, ProviderRecoveryReason::StreamItemLimit)
        }
        ProviderError::Cancelled => attempt::AttemptError::Cancelled,
        error => retryable(error, ProviderRecoveryReason::ProviderFailure),
    }
}

fn establishment_failure(error: ProviderError) -> attempt::AttemptError {
    match provider_failure(error) {
        attempt::AttemptError::Retryable { error, reason } => terminal(error, reason),
        error => error,
    }
}

fn retryable(error: ProviderError, reason: ProviderRecoveryReason) -> attempt::AttemptError {
    attempt::AttemptError::Retryable { error, reason }
}

fn terminal(error: ProviderError, reason: ProviderRecoveryReason) -> attempt::AttemptError {
    attempt::AttemptError::Terminal { error, reason }
}

fn failed(error: attempt::AttemptError, usage: Option<TokenUsage>) -> attempt::AttemptOutcome {
    attempt::AttemptOutcome::Failed { error, usage }
}
