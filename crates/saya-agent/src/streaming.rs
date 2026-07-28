use crate::{ChatMessage, ChatRequest, ChatResponse, ProviderError, ToolCall};
use async_trait::async_trait;
use futures_util::{Stream, StreamExt};
use std::{
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use tokio::sync::Notify;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderEvent {
    TextDelta(String),
    ToolCalls(Vec<ToolCall>),
    Done,
}
pub type ProviderStream = Pin<Box<dyn Stream<Item = Result<ProviderEvent, ProviderError>> + Send>>;

#[derive(Clone, Default)]
pub struct CancellationToken(Arc<CancellationState>);
#[derive(Default)]
struct CancellationState {
    cancelled: AtomicBool,
    notify: Notify,
}
impl CancellationToken {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn cancel(&self) {
        self.0.cancelled.store(true, Ordering::Release);
        self.0.notify.notify_waiters();
    }
    pub fn is_cancelled(&self) -> bool {
        self.0.cancelled.load(Ordering::Acquire)
    }
    pub async fn cancelled(&self) {
        if !self.is_cancelled() {
            self.0.notify.notified().await;
        }
    }
}

#[async_trait]
pub trait ChatProvider: Send + Sync {
    fn name(&self) -> &str;
    async fn complete(&self, request: ChatRequest) -> Result<ChatResponse, ProviderError>;
    async fn stream(
        &self,
        request: ChatRequest,
        cancellation: CancellationToken,
    ) -> Result<ProviderStream, ProviderError> {
        let response = self.complete(request).await?;
        if cancellation.is_cancelled() {
            return Err(ProviderError::Cancelled);
        }
        let events = if response.message.tool_calls.is_empty() {
            vec![
                ProviderEvent::TextDelta(response.message.content),
                ProviderEvent::Done,
            ]
        } else {
            vec![
                ProviderEvent::ToolCalls(response.message.tool_calls),
                ProviderEvent::Done,
            ]
        };
        Ok(Box::pin(futures_util::stream::iter(
            events.into_iter().map(Ok),
        )))
    }
    async fn collect(&self, request: ChatRequest) -> Result<ChatResponse, ProviderError> {
        let mut stream = self.stream(request, CancellationToken::new()).await?;
        let (mut content, mut tool_calls, mut complete) = (String::new(), Vec::new(), false);
        while let Some(event) = stream.next().await {
            match event? {
                ProviderEvent::TextDelta(value) => content.push_str(&value),
                ProviderEvent::ToolCalls(calls) => tool_calls.extend(calls),
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
        })
    }
}
