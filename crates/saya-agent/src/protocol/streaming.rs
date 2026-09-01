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

/// Ceiling on one provider response's accumulated bytes. A misbehaving or
/// hostile endpoint must not be able to stream unbounded data into memory.
pub const MAX_STREAM_BYTES: usize = 2 * 1024 * 1024;

/// Token counts reported by a provider for one response. Providers that do
/// not report usage simply never emit it.
///
/// Every field beyond `input_tokens`/`output_tokens` is `Option<u64>`: a
/// provider that does not report a number leaves it `None` (invariant: absent
/// is not zero — a cache hit rate computed over `None` is *unknown*, not 0%).
/// The two existing fields keep their meaning and type (`u64`) so callers
/// that sum or copy them are unaffected. The struct stays `Copy` because every
/// field is `Copy`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TokenUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    /// Input tokens served from the provider's prompt cache. **Inclusive of
    /// `input_tokens`** (a subset of the prompt): a 90% cache hit means 90% of
    /// `input_tokens` was read from cache, not added on top. Wire sources:
    /// OpenAI `usage.prompt_tokens_details.cached_tokens`,
    /// Anthropic `usage.cache_read_input_tokens`,
    /// Gemini `usageMetadata.cachedContentTokenCount`. `None` when the provider
    /// does not report cache reads (Ollama; any provider that omits the field).
    pub cached_input_tokens: Option<u64>,
    /// Input tokens spent *creating* a cache entry (Anthropic only). Inclusive
    /// of `input_tokens`, and kept distinct from `cached_input_tokens` because
    /// creation bills differently from reads. Wire source: Anthropic
    /// `usage.cache_creation_input_tokens`. `None` for providers with no
    /// cache-write concept (OpenAI, Gemini, Ollama).
    pub cache_creation_input_tokens: Option<u64>,
    /// Tokens spent on chain-of-thought reasoning. The inclusive/separate
    /// split is per-provider and documented here because a normalising layer
    /// that guesses wrong is worse than the raw number: on **OpenAI** this is
    /// `usage.completion_tokens_details.reasoning_tokens`, which sits *inside*
    /// `completion_tokens` and is therefore **inclusive of `output_tokens`**
    /// — do not add it to `output_tokens`. On **Gemini** it is
    /// `usageMetadata.thoughtsTokenCount`, reported *separately* from
    /// `candidatesTokenCount` (not part of `output_tokens`). Anthropic counts
    /// reasoning inside its output tokens (no separate field) and Ollama
    /// reports none; both leave this `None`.
    pub reasoning_tokens: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderEvent {
    TextDelta(String),
    ToolCalls(Vec<ToolCall>),
    /// The provider's cumulative token counts so far for this response.
    Usage(TokenUsage),
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
        let notified = self.0.notify.notified();
        tokio::pin!(notified);
        notified.as_mut().enable();
        if !self.is_cancelled() {
            notified.await;
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
                ProviderEvent::TextDelta(value) => {
                    if content.len().saturating_add(value.len()) > MAX_STREAM_BYTES {
                        return Err(ProviderError::Request(
                            "provider stream exceeded size limit".into(),
                        ));
                    }
                    content.push_str(&value);
                }
                ProviderEvent::ToolCalls(calls) => tool_calls.extend(calls),
                ProviderEvent::Usage(_) => {}
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

#[cfg(test)]
mod tests {
    use super::{CancellationToken, TokenUsage};
    use std::time::Duration;

    /// Deliverable 5: `Some(0)` is a *report* of zero and must survive
    /// distinct from `None` (an unreported number). If this cannot be written,
    /// the type is wrong — here the two compare unequal and `Some(0)` survives
    /// a `Copy`.
    #[test]
    fn some_zero_usage_is_distinct_from_absent() {
        let reported_zero = TokenUsage {
            cached_input_tokens: Some(0),
            ..Default::default()
        };
        let unreported = TokenUsage::default();
        assert_ne!(reported_zero, unreported);
        let copy = reported_zero;
        assert_eq!(copy.cached_input_tokens, Some(0));
        assert_eq!(unreported.cached_input_tokens, None);
    }

    /// `TokenUsage` stays `Copy` (invariant 3): assigning rebinds a value, not
    /// a borrow, and the new fields default to `None` without disturbing the
    /// two existing counters.
    #[test]
    fn token_usage_is_copy_and_new_fields_default_to_none() {
        let mut usage = TokenUsage {
            input_tokens: 3,
            output_tokens: 7,
            ..Default::default()
        };
        assert_eq!(usage.cached_input_tokens, None);
        assert_eq!(usage.cache_creation_input_tokens, None);
        assert_eq!(usage.reasoning_tokens, None);
        let copy = usage;
        usage.cached_input_tokens = Some(5);
        // The copy is independent: assigning to `usage` did not mutate `copy`.
        assert_eq!(copy.cached_input_tokens, None);
        assert_eq!(usage.cached_input_tokens, Some(5));
    }

    #[tokio::test]
    async fn cancellation_waiter_does_not_miss_a_notification() {
        for _ in 0..64 {
            let token = CancellationToken::new();
            let waiter = token.clone();
            let task = tokio::spawn(async move { waiter.cancelled().await });
            tokio::task::yield_now().await;
            token.cancel();
            tokio::time::timeout(Duration::from_millis(100), task)
                .await
                .unwrap()
                .unwrap();
        }
    }
}
