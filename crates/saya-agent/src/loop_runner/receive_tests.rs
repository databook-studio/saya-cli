//! Tests for `receive` — the single-attempt request shape and (via the
//! `agent_loop.rs` integration suite) the mid-stream retry policy.

use super::*;
use crate::{ChatResponse, ProviderStream, ReasoningEffort, ResponseFormat};
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

/// The main loop's request must NOT carry JSON
/// mode — a prose answer stays prose. `receive` builds the request with
/// `..Default::default()`, so `response_format` is `Text` and
/// `reasoning_effort` is `Default` (send nothing): the main loop keeps real
/// reasoning, leaving effort to the endpoint — only mechanical call sites
/// request less.
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
    assert_eq!(
        sent.reasoning_effort,
        ReasoningEffort::Default,
        "the main loop must not request less effort"
    );
}
