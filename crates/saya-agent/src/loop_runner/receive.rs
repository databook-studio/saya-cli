use super::{AgentError, check_cancelled, emit};
use crate::{
    AgentEvent, AgentEventSink, CancellationToken, ChatMessage, ChatProvider, ChatRequest,
    ProviderError, ProviderEvent, TokenUsage, ToolDefinition,
};
use futures_util::StreamExt;

/// Streams one provider turn: assembles the assistant message and reports
/// the token usage the provider disclosed for it.
pub(super) async fn receive(
    provider: &dyn ChatProvider,
    model: &str,
    messages: &[ChatMessage],
    definitions: &[ToolDefinition],
    sink: &dyn AgentEventSink,
    cancellation: &CancellationToken,
    events: &mut Vec<AgentEvent>,
) -> Result<(ChatMessage, TokenUsage), AgentError> {
    let mut stream = provider
        .stream(
            ChatRequest {
                model: model.into(),
                messages: messages.into(),
                tools: definitions.into(),
                // Invariant 1: JSON mode is for the extraction call only. The
                // main loop never sets `response_format`, so it stays `Text`
                // (the default) and a prose answer remains prose.
                ..Default::default()
            },
            cancellation.clone(),
        )
        .await?;
    let (mut content, mut calls, mut complete) = (String::new(), Vec::new(), false);
    let mut usage = TokenUsage::default();
    while let Some(event) = stream.next().await {
        check_cancelled(cancellation)?;
        match event? {
            ProviderEvent::TextDelta(text) => {
                if content.len().saturating_add(text.len()) > crate::MAX_STREAM_BYTES {
                    return Err(AgentError::Provider(ProviderError::Request(
                        "provider stream exceeded size limit".into(),
                    )));
                }
                content.push_str(&text);
                emit(events, sink, AgentEvent::AssistantText { text }).await;
            }
            ProviderEvent::ToolCalls(value) => calls.extend(value),
            ProviderEvent::Usage(counts) => usage = counts,
            ProviderEvent::Done => complete = true,
        }
    }
    if !complete || (content.trim().is_empty() && calls.is_empty()) {
        return Err(AgentError::Provider(ProviderError::InvalidResponse));
    }
    Ok((
        ChatMessage {
            role: "assistant".into(),
            content,
            tool_calls: calls,
            tool_call_id: None,
        },
        usage,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ChatResponse, ProviderStream, ResponseFormat};
    use async_trait::async_trait;
    use futures_util::stream;
    use std::sync::Mutex;

    /// A provider that records the one `ChatRequest` the main loop sent and
    /// returns a minimal valid stream (a single text delta + Done).
    struct RecordingProvider {
        captured: Mutex<Option<ChatRequest>>,
    }

    #[async_trait]
    impl ChatProvider for RecordingProvider {
        fn name(&self) -> &str {
            "recording"
        }
        async fn complete(&self, _request: ChatRequest) -> Result<ChatResponse, ProviderError> {
            unreachable!("receive uses stream, not complete")
        }
        async fn stream(
            &self,
            request: ChatRequest,
            _cancellation: CancellationToken,
        ) -> Result<ProviderStream, ProviderError> {
            *self.captured.lock().unwrap() = Some(request);
            let events = vec![
                Ok(ProviderEvent::TextDelta("ok".into())),
                Ok(ProviderEvent::Done),
            ];
            Ok(Box::pin(stream::iter(events)))
        }
    }

    /// Invariant 1 (deliverable 4): the main loop's request must NOT carry JSON
    /// mode — a prose answer stays prose. `receive` builds the request with
    /// `..Default::default()`, so `response_format` is `Text`.
    #[tokio::test]
    async fn main_loop_request_does_not_set_json_mode() {
        let provider = RecordingProvider {
            captured: Mutex::new(None),
        };
        let sink = crate::NoopEventSink;
        let mut events = Vec::new();
        let cancellation = CancellationToken::new();
        let messages = vec![ChatMessage::text("user", "hello")];
        receive(
            &provider,
            "m",
            &messages,
            &[],
            &sink,
            &cancellation,
            &mut events,
        )
        .await
        .expect("receive succeeds");
        let sent = provider
            .captured
            .lock()
            .unwrap()
            .take()
            .expect("a request was sent");
        assert_eq!(
            sent.response_format,
            ResponseFormat::Text,
            "the main loop must not set JSON mode (invariant 1)"
        );
    }
}
