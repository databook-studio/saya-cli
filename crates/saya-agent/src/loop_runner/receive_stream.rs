use super::{attempt, emit};
use crate::protocol::streaming::admit_tool_calls;
use crate::{
    AgentEvent, AgentEventSink, CancellationToken, ChatMessage, ChatProvider, ChatRequest,
    ProviderError, ProviderEvent, ProviderRecoveryReason, TokenUsage,
};
use futures_util::StreamExt;

pub(super) async fn stream_attempt(
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
            Err(error) => return failed(establishment_failure(error), None),
        },
    };
    let (mut content, mut calls, mut complete, mut usage, mut reasoning) =
        (String::new(), Vec::new(), false, None, None);
    let mut tool_call_bytes = 0usize;
    loop {
        let next = tokio::select! {
            _ = cancellation.cancelled() => return failed(attempt::AttemptError::Cancelled, usage),
            next = stream.next() => next,
        };
        let Some(event) = next else { break };
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
