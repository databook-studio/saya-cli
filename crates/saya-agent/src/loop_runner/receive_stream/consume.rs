use super::RequestDeadline;
use super::{
    super::{attempt, attempt_metrics::AttemptTiming},
    emit,
};
use crate::protocol::tool_collection::admit_tool_calls;
use crate::providers::framing::request_cut;
use crate::{
    AgentEvent, AgentEventSink, AgentMetric, AgentMetricOutcome, AgentMetricUseful,
    CancellationToken, ChatMessage, ProviderError, ProviderEvent, ProviderRecoveryReason,
    ProviderStream, TokenUsage,
};
use futures_util::StreamExt;
use std::time::{Duration, Instant};

/// The failure for a stream that ended before its terminator.
///
/// A stream that delivered nothing is a plain retryable [`ProviderError::StreamEnded`]:
/// a flaky gateway still recovers on the backoff. One that had already
/// delivered content is a cut — a gateway response time limit cuts the
/// identical request at the same point — so the identical request is never
/// re-sent; the loop spends a bounded continuation. Visible text and tool
/// arguments are reported on the cut; they do not size the next ceiling.
/// A time-cut is a clock. `elapsed` is the stream's own clock, not the wall
/// clock of the whole attempt.
fn stream_end(
    delivered_bytes: usize,
    visible_bytes: usize,
    elapsed: Duration,
) -> attempt::AttemptError {
    if delivered_bytes == 0 {
        return retryable(
            ProviderError::StreamEnded,
            ProviderRecoveryReason::StreamEnded,
        );
    }
    // Visible text and tool arguments are reported on the cut. Reasoning is
    // discarded and never replayed. Neither sizes the next ceiling: a
    // time-cut is a clock.
    terminal(
        request_cut(elapsed, visible_bytes),
        ProviderRecoveryReason::StreamCut,
    )
}

/// The failure when a request's total deadline expires. A deadline set by the
/// job's own wall clock is that budget stopping the job — latch the typed
/// reason so the loop returns the timeout class, never a provider failure. The
/// provider's own cap stays a typed cut, answered by a continuation.
pub(super) fn deadline_failure(
    deadline: RequestDeadline,
    cancellation: &CancellationToken,
    elapsed: Duration,
    visible: usize,
) -> attempt::AttemptError {
    if deadline.wall_bound {
        cancellation.cancel_with_reason(crate::CancellationReason::Budget(
            crate::BudgetDimension::ElapsedTime,
        ));
        return attempt::AttemptError::Cancelled;
    }
    terminal(
        request_cut(elapsed, visible),
        ProviderRecoveryReason::StreamCut,
    )
}

pub(super) async fn consume_stream(
    mut stream: ProviderStream,
    sink: &dyn AgentEventSink,
    cancellation: &CancellationToken,
    events: &mut Vec<AgentEvent>,
    timing: &mut AttemptTiming,
    deadline: RequestDeadline,
) -> attempt::AttemptOutcome {
    let (mut content, mut calls, mut complete, mut usage, mut reasoning, mut signature) =
        (String::new(), Vec::new(), false, None, None, None);
    let mut useful_text_seen = false;
    let mut tool_call_bytes = 0usize;
    let stream_started = Instant::now();
    let mut items = crate::protocol::stream_item_limit::StreamItemCount::default();
    loop {
        let read_started = Instant::now();
        let next = tokio::select! {
            _ = cancellation.cancelled() => {
                timing.add_read_elapsed(read_started.elapsed());
                return failed(attempt::AttemptError::Cancelled, usage);
            }
            _ = tokio::time::sleep_until(deadline.at) => {
                timing.add_read_elapsed(read_started.elapsed());
                // The total deadline is a clock: it cuts the stream even when
                // chunks were flowing. A provider cap is answered with the
                // bounded continuation; the job's own wall clock is its budget
                // stopping, and must not be reported as a provider failure.
                return failed(
                    deadline_failure(
                        deadline,
                        cancellation,
                        stream_started.elapsed(),
                        visible_bytes(&content, tool_call_bytes),
                    ),
                    usage,
                );
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
                return failed(
                    match error {
                        // A reported end after content is a cut, exactly as a
                        // terminator-less end is: both mean the stream stopped
                        // mid-flow. `provider_failure` keeps the retryable
                        // reading for the establishment path, where nothing
                        // can have been delivered.
                        ProviderError::StreamEnded => stream_end(
                            delivered_bytes(&content, &reasoning, tool_call_bytes),
                            visible_bytes(&content, tool_call_bytes),
                            stream_started.elapsed(),
                        ),
                        error => provider_failure(error),
                    },
                    usage,
                );
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
            ProviderEvent::ReasoningSignature(value) => signature = Some(value),
            ProviderEvent::Usage(counts) => usage = Some(counts),
            ProviderEvent::Done => complete = true,
        }
    }
    if !complete {
        return failed(
            stream_end(
                delivered_bytes(&content, &reasoning, tool_call_bytes),
                visible_bytes(&content, tool_call_bytes),
                stream_started.elapsed(),
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

            images: Vec::new(),
            reasoning: None,
            reasoning_signature: None,
        },
        usage,
        reasoning: attempt::ReasoningTrace::new(reasoning, signature),
    }
}

/// Content bytes the stream had delivered when it stopped: reasoning, text,
/// and the tool-call arguments already admitted. Any one of them non-zero
/// means the stream was flowing, so the request is not re-sent identically.
fn delivered_bytes(content: &str, reasoning: &Option<String>, tool_call_bytes: usize) -> usize {
    visible_bytes(content, tool_call_bytes)
        .saturating_add(reasoning.as_ref().map_or(0, String::len))
}

fn visible_bytes(content: &str, tool_call_bytes: usize) -> usize {
    content.len().saturating_add(tool_call_bytes)
}

fn byte_limit_error() -> attempt::AttemptError {
    retryable(
        ProviderError::Request("provider stream exceeded size limit".into()),
        ProviderRecoveryReason::StreamByteLimit,
    )
}

pub(super) fn provider_failure(error: ProviderError) -> attempt::AttemptError {
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
        // A total-deadline cut is deterministic: the identical request would
        // spend the same deadline again, so it is never retried. The loop
        // continues with its bounded stream-cut notice instead.
        error @ ProviderError::StreamCut { .. } => {
            terminal(error, ProviderRecoveryReason::StreamCut)
        }
        ProviderError::Cancelled => attempt::AttemptError::Cancelled,
        // Retried like any provider failure, under its own recovery reason.
        ProviderError::StreamEnded => retryable(
            ProviderError::StreamEnded,
            ProviderRecoveryReason::StreamEnded,
        ),
        // A content stall is retryable: the same request may answer on the
        // next attempt, unlike a cut that already delivered content.
        ProviderError::StreamStalled => retryable(
            ProviderError::StreamStalled,
            ProviderRecoveryReason::StreamStalled,
        ),
        error => retryable(error, ProviderRecoveryReason::ProviderFailure),
    }
}

fn retryable(error: ProviderError, reason: ProviderRecoveryReason) -> attempt::AttemptError {
    attempt::AttemptError::Retryable { error, reason }
}

pub(super) fn terminal(
    error: ProviderError,
    reason: ProviderRecoveryReason,
) -> attempt::AttemptError {
    attempt::AttemptError::Terminal { error, reason }
}

fn failed(error: attempt::AttemptError, usage: Option<TokenUsage>) -> attempt::AttemptOutcome {
    attempt::AttemptOutcome::Failed { error, usage }
}
