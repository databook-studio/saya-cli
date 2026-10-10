use super::super::tool_collection::admit_tool_calls;
use super::physical_attempt::{ControlledProviderError, PhysicalAttemptControl};
use super::{CancellationToken, MAX_STREAM_BYTES, ProviderEvent, ProviderStream};
use crate::{ChatMessage, ChatRequest, ChatResponse, ProviderError};
use async_trait::async_trait;
use futures_util::StreamExt;

#[async_trait]
pub trait ChatProvider: Send + Sync {
    fn name(&self) -> &str;
    async fn complete(&self, request: ChatRequest) -> Result<ChatResponse, ProviderError>;
    async fn stream(
        &self,
        request: ChatRequest,
        cancellation: CancellationToken,
    ) -> Result<ProviderStream, ProviderError> {
        if cancellation.is_cancelled() {
            return Err(ProviderError::Cancelled);
        }
        let response = self.complete(request).await?;
        if cancellation.is_cancelled() {
            return Err(ProviderError::Cancelled);
        }
        // Preserve each complete-response field through the streaming boundary.
        // A provider that reported no usage emits no `Usage` event, so absence
        // keeps meaning "unknown".
        let mut events = Vec::with_capacity(5);
        // `complete()` has already settled, so preserve its paid usage before
        // any content consumer can stop early (for example on invalid JSON).
        if let Some(usage) = response.usage {
            events.push(ProviderEvent::Usage(usage));
        }
        if !response.message.content.is_empty() {
            events.push(ProviderEvent::TextDelta(response.message.content));
        }
        if let Some(reasoning) = response.reasoning {
            events.push(ProviderEvent::ReasoningDelta(reasoning));
        }
        if !response.message.tool_calls.is_empty() {
            events.push(ProviderEvent::ToolCalls(response.message.tool_calls));
        }
        events.push(ProviderEvent::Done);
        Ok(Box::pin(futures_util::stream::iter(
            events.into_iter().map(Ok),
        )))
    }
    /// Streams a response only when the provider exposes every physical send.
    /// Implementations admit before each send, observe cancellation, and settle
    /// successful responses after body use. The default refuses before calling
    /// legacy methods.
    async fn stream_with_physical_attempts(
        &self,
        _request: ChatRequest,
        _cancellation: CancellationToken,
        _attempts: &mut PhysicalAttemptControl<'_>,
    ) -> Result<ProviderStream, ControlledProviderError> {
        Err(ControlledProviderError::UnsupportedCoverage)
    }
    async fn collect(&self, request: ChatRequest) -> Result<ChatResponse, ProviderError> {
        let mut stream = self.stream(request, CancellationToken::new()).await?;
        let mut items = super::super::stream_item_limit::StreamItemCount::default();
        let (mut content, mut tool_calls, mut complete) = (String::new(), Vec::new(), false);
        let mut assembled_bytes = 0usize;
        let mut tool_call_bytes = 0usize;
        // Reasoning is accumulated alongside content: a single string
        // per turn, no ordering relative to text. The bound covers reasoning
        // too — a hostile endpoint streaming unbounded "thinking" must not
        // exhaust memory, so each delta is checked against `MAX_STREAM_BYTES`
        // exactly as `TextDelta` is (as `collect()` requires). `None` when
        // the stream emits no `ReasoningDelta`, so a provider that reports no
        // reasoning stays distinguishable from one that reported an empty
        // string (absent is not zero, mirrored from usage).
        let mut reasoning = None;
        // `None` until the stream emits a `Usage` event; the last event wins
        //. OpenAI emits one trailing usage-only chunk; Anthropic emits
        // cumulative snapshots on `message_start`/`message_delta`, so summing
        // would double-count — the final snapshot is the truth, exactly as the
        // stream's own accumulator already folds them into one running total.
        // Keeping `None` when no event arrives preserves "absent is not zero"
        //: a provider that reports nothing stays distinguishable
        // from one that reported zeros.
        let mut usage = None;
        while let Some(event) = stream.next().await {
            let event = match event {
                Ok(event) => event,
                Err(ProviderError::StreamItemLimit { usage: reported }) => {
                    return Err(ProviderError::StreamItemLimit {
                        usage: reported.or(usage.map(Box::new)),
                    });
                }
                Err(error) => return Err(error),
            };
            if let ProviderEvent::Usage(reported) = &event {
                usage = Some(*reported);
            }
            items.admit(usage)?;
            match event {
                ProviderEvent::TextDelta(value) => {
                    if assembled_bytes.saturating_add(value.len()) > MAX_STREAM_BYTES {
                        return Err(ProviderError::Request(
                            "provider stream exceeded size limit".into(),
                        ));
                    }
                    assembled_bytes += value.len();
                    content.push_str(&value);
                }
                ProviderEvent::ReasoningDelta(value) => {
                    if assembled_bytes.saturating_add(value.len()) > MAX_STREAM_BYTES {
                        return Err(ProviderError::Request(
                            "provider stream exceeded size limit".into(),
                        ));
                    }
                    assembled_bytes += value.len();
                    let accumulated = reasoning.get_or_insert_with(String::new);
                    accumulated.push_str(&value);
                }
                ProviderEvent::ToolCalls(calls) => {
                    let non_tool_bytes = assembled_bytes.saturating_sub(tool_call_bytes);
                    let bytes = admit_tool_calls(
                        &mut tool_calls,
                        &mut tool_call_bytes,
                        calls,
                        MAX_STREAM_BYTES.saturating_sub(non_tool_bytes),
                    )?;
                    assembled_bytes += bytes;
                }
                ProviderEvent::Usage(reported) => usage = Some(reported),
                ProviderEvent::Done => complete = true,
            }
        }
        if !complete || (content.trim().is_empty() && tool_calls.is_empty()) {
            return Err(ProviderError::InvalidResponse);
        }
        Ok(ChatResponse {
            message: ChatMessage {
                role: "assistant".into(),
                content,
                tool_calls,
                tool_call_id: None,
            },
            reasoning,
            usage,
        })
    }
}
