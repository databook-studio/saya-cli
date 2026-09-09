use super::{AgentError, check_cancelled, emit};
use crate::{
    AgentEvent, AgentEventSink, CancellationToken, ChatMessage, ChatProvider, ChatRequest,
    ProviderError, ProviderEvent, TokenUsage, ToolDefinition, UsageCall,
};
use futures_util::StreamExt;
use std::time::Duration;

/// Streams one provider turn: assembles the assistant message, captures the
/// chain-of-thought the model produced, and reports the token usage the
/// provider disclosed for it. Returns `(message, usage, reasoning)`.
///
/// `reasoning` is returned separately and **never** placed on the `ChatMessage`
/// — that message is pushed into `messages` and replayed to the provider as
/// history on the next turn, so reasoning on it would be replayed to the model.
/// The caller (`loop_runner::mod`) forwards the accumulated string onto the
/// event stream as one `AgentEvent::ReasoningText`, so the turn's
/// thinking reaches the CLI; it is not pushed onto `messages`. Absent is not zero
/// is why `receive` keeps reasoning at all: the main loop must not suppress
/// thinking, or SQL quality degrades; a reasoning model's chain-of-thought is
/// held for the turn and forwarded when the stream completes.
///
/// A stream that fails **mid-response** — it drops (ends without `Done`), it
/// stalls past the provider's idle timeout, it is incomplete, or it trips
/// `MAX_STREAM_BYTES` — does not fail the turn: the partial assistant response
/// is discarded, one [`AgentEvent::TurnReset`] tells sinks to replace the text
/// emitted so far, and the turn is retried over the default backoff schedule
/// (`providers::default_retry_delays`, 250ms/500ms/1s). Every attempt re-sends
/// the conversation exactly as it stood at turn start — `messages` is the
/// caller's slice and is not mutated until this returns, so no partial answer
/// can enter a retried request. When the schedule is exhausted, the last
/// failure falls through to the existing error path. A failure while
/// *establishing* the stream is not retried here: `providers/http.rs` already
/// retries until the response is established, and a refused request (bad key,
/// bad model) stays an error. Neither is a stream that **completed** but
/// carried no usable response — the model answered nothing, and every retry
/// would fail the same way. Cancellation is never retried.
/// How one streaming attempt ended. The retry policy in [`receive`] acts on
/// the distinction: a stream that failed **mid-response** — an error event (a
/// stall, a parse failure, a dropped connection), an end without `Done`, or a
/// `MAX_STREAM_BYTES` trip — is retryable, because the transport failed and
/// the same request may succeed. A stream that completed but carried no usable
/// response, or a request that never established, is not: re-sending adds
/// latency without new information, and the existing error path applies.
enum AttemptError {
    /// Cancellation: never retried.
    Cancelled,
    /// The stream failed mid-response; the turn may be retried.
    MidStream(ProviderError),
    /// Not a mid-stream failure; the existing error path applies immediately.
    Fatal(ProviderError),
}

pub(super) async fn receive(
    provider: &dyn ChatProvider,
    model: &str,
    messages: &[ChatMessage],
    definitions: &[ToolDefinition],
    sink: &dyn AgentEventSink,
    cancellation: &CancellationToken,
    events: &mut Vec<AgentEvent>,
) -> Result<(ChatMessage, TokenUsage, Option<String>), AgentError> {
    let request = ChatRequest {
        model: model.into(),
        messages: messages.into(),
        tools: definitions.into(),
        // JSON mode is for the extraction call only. The
        // main loop never sets `response_format`, so it stays `Text`
        // (the default) and a prose answer remains prose.
        ..Default::default()
    };
    let delays = crate::providers::default_retry_delays();
    let mut attempt = 0;
    loop {
        match stream_attempt(provider, &request, sink, cancellation, events).await {
            Ok(turn) => return Ok(turn),
            Err(AttemptError::Cancelled) => return Err(AgentError::Cancelled),
            // Retryable mid-stream failure: tell sinks to discard the partial
            // answer, then back off before the next attempt.
            Err(AttemptError::MidStream(_)) if attempt < delays.len() => {
                emit(events, sink, AgentEvent::turn_reset()).await;
                wait(delays[attempt], cancellation).await?;
                attempt += 1;
            }
            // The schedule is exhausted: the existing error path takes over.
            Err(AttemptError::MidStream(error)) => {
                return Err(AgentError::Provider(error));
            }
            Err(AttemptError::Fatal(error)) => return Err(AgentError::Provider(error)),
        }
    }
}

/// Sleeps `delay`, aborting immediately when the run is cancelled.
async fn wait(delay: Duration, cancellation: &CancellationToken) -> Result<(), AgentError> {
    tokio::select! {
        _ = cancellation.cancelled() => Err(AgentError::Cancelled),
        _ = tokio::time::sleep(delay) => Ok(()),
    }
}

/// One streaming attempt of the turn. The retry policy lives in [`receive`];
/// this only classifies how the attempt ended.
async fn stream_attempt(
    provider: &dyn ChatProvider,
    request: &ChatRequest,
    sink: &dyn AgentEventSink,
    cancellation: &CancellationToken,
    events: &mut Vec<AgentEvent>,
) -> Result<(ChatMessage, TokenUsage, Option<String>), AttemptError> {
    // A failure while establishing the stream is `Fatal`: the HTTP layer
    // already retried establishment, and a refused request (bad key, bad
    // model) would fail identically on every retry.
    let mut stream = provider
        .stream(request.clone(), cancellation.clone())
        .await
        .map_err(AttemptError::Fatal)?;
    let (mut content, mut calls, mut complete) = (String::new(), Vec::new(), false);
    // `None` until the stream emits a `Usage` event, so a provider that reports
    // nothing emits no usage event at all — absence means "unknown", not "this
    // call cost nothing", and the last snapshot wins exactly as the response's
    // own accumulator folds cumulative reports.
    let mut usage = None;
    // `None` until the stream emits reasoning; accumulated under the same
    // `MAX_STREAM_BYTES` bound as content so a hostile endpoint cannot stream
    // unbounded "thinking" into memory.
    let mut reasoning = None;
    while let Some(event) = stream.next().await {
        if check_cancelled(cancellation).is_err() {
            return Err(AttemptError::Cancelled);
        }
        // An error surfaced by the stream itself (stall, dropped connection,
        // unparseable chunk) is the definition of a mid-stream failure.
        let event = event.map_err(AttemptError::MidStream)?;
        match event {
            ProviderEvent::TextDelta(text) => {
                if content.len().saturating_add(text.len()) > crate::MAX_STREAM_BYTES {
                    return Err(AttemptError::MidStream(ProviderError::Request(
                        "provider stream exceeded size limit".into(),
                    )));
                }
                content.push_str(&text);
                emit(events, sink, AgentEvent::AssistantText { text }).await;
            }
            ProviderEvent::ReasoningDelta(text) => {
                let accumulated = reasoning.get_or_insert_with(String::new);
                if accumulated.len().saturating_add(text.len()) > crate::MAX_STREAM_BYTES {
                    return Err(AttemptError::MidStream(ProviderError::Request(
                        "provider stream exceeded size limit".into(),
                    )));
                }
                accumulated.push_str(&text);
            }
            ProviderEvent::ToolCalls(value) => calls.extend(value),
            ProviderEvent::Usage(counts) => usage = Some(counts),
            ProviderEvent::Done => complete = true,
        }
    }
    // A stream that ended without `Done` was dropped or truncated mid-response:
    // a failed attempt, whatever it managed to emit before dying.
    if !complete {
        return Err(AttemptError::MidStream(ProviderError::InvalidResponse));
    }
    // The stream completed but the model produced nothing usable. This is not
    // a transport failure — every attempt would fail the same way — so it is
    // not retried: the existing error path applies.
    if content.trim().is_empty() && calls.is_empty() {
        return Err(AttemptError::Fatal(ProviderError::InvalidResponse));
    }
    // The report crosses the boundary once per call that made one, named as an
    // answering call so a consumer can keep it apart from the extraction call's
    // report (emitted by the CLI runtime, which owns that call).
    if let Some(counts) = usage {
        emit(events, sink, AgentEvent::usage(UsageCall::Answer, counts)).await;
    }
    Ok((
        ChatMessage {
            role: "assistant".into(),
            content,
            tool_calls: calls,
            tool_call_id: None,
        },
        usage.unwrap_or_default(),
        reasoning,
    ))
}

#[cfg(test)]
#[path = "receive_tests.rs"]
mod tests;
