use super::{AgentError, check_cancelled, emit};
use crate::{
    AgentEvent, AgentEventSink, CancellationToken, ChatMessage, ChatProvider, ChatRequest,
    ProviderError, ProviderEvent, ToolDefinition,
};
use futures_util::StreamExt;

pub(super) async fn receive(
    provider: &dyn ChatProvider,
    model: &str,
    messages: &[ChatMessage],
    definitions: &[ToolDefinition],
    sink: &dyn AgentEventSink,
    cancellation: &CancellationToken,
    events: &mut Vec<AgentEvent>,
) -> Result<ChatMessage, AgentError> {
    let mut stream = provider
        .stream(
            ChatRequest {
                model: model.into(),
                messages: messages.into(),
                tools: definitions.into(),
            },
            cancellation.clone(),
        )
        .await?;
    let (mut content, mut calls, mut complete) = (String::new(), Vec::new(), false);
    while let Some(event) = stream.next().await {
        check_cancelled(cancellation)?;
        match event? {
            ProviderEvent::TextDelta(text) => {
                content.push_str(&text);
                emit(events, sink, AgentEvent::AssistantText { text }).await;
            }
            ProviderEvent::ToolCalls(value) => calls.extend(value),
            ProviderEvent::Done => complete = true,
        }
    }
    if !complete || (content.trim().is_empty() && calls.is_empty()) {
        return Err(AgentError::Provider(ProviderError::InvalidResponse));
    }
    Ok(ChatMessage {
        role: "assistant".into(),
        content,
        tool_calls: calls,
        tool_call_id: None,
    })
}
