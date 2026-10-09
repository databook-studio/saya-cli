use super::{
    CancellationToken, ChatProvider, MAX_STREAM_BYTES, ProviderEvent, ProviderStream, TokenUsage,
};
use crate::{ChatMessage, ChatRequest, ChatResponse, ProviderError};
use async_trait::async_trait;
use futures_util::{StreamExt, stream};
use std::time::Duration;

/// `Some(0)` is a *report* of zero and must survive
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

/// `TokenUsage` stays `Copy`: assigning rebinds a value, not
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

/// A minimal provider whose `stream()` replays a canned event list, so the
/// default `collect()` can be exercised without a network. `complete()`
/// forwards to `collect()` exactly as the three real providers do.
struct CannedProvider {
    events: Vec<Result<ProviderEvent, ProviderError>>,
}

#[async_trait]
impl ChatProvider for CannedProvider {
    fn name(&self) -> &str {
        "canned"
    }
    async fn complete(&self, request: ChatRequest) -> Result<ChatResponse, ProviderError> {
        self.collect(request).await
    }
    async fn stream(
        &self,
        _request: ChatRequest,
        _cancellation: CancellationToken,
    ) -> Result<ProviderStream, ProviderError> {
        Ok(Box::pin(stream::iter(self.events.clone())))
    }
}

#[path = "tests/provider.rs"]
mod provider;
#[path = "tests/reasoning.rs"]
mod reasoning;
#[path = "tests/usage.rs"]
mod usage;
