//! Tests for `receive`'s main-loop request shape.

use super::{ReceiveContext, receive};
use crate::loop_runner::admission::Admission;
use crate::{
    AgentLimits, AnsweringLimits, CancellationToken, ChatMessage, ChatProvider, ChatRequest,
    ChatResponse, CorrelationId, ProviderError, ProviderEvent, ProviderStream, ReasoningEffort,
    ResponseFormat,
};
use async_trait::async_trait;
use futures_util::stream;
use std::sync::Mutex;

/// A provider that records the one `ChatRequest` the main loop sent and
/// returns a minimal valid stream (a single text delta + Done).
struct RecordingProvider {
    captured: Mutex<Option<ChatRequest>>,
    configured: Option<ReasoningEffort>,
}

#[async_trait]
impl ChatProvider for RecordingProvider {
    fn name(&self) -> &str {
        "recording"
    }
    fn configured_reasoning_effort(&self) -> Option<ReasoningEffort> {
        self.configured
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

/// The main loop's request must NOT carry JSON mode — a prose answer stays
/// prose. Response format stays `Text`; the reasoning effort is the adaptive
/// policy's decision, not the request default.
#[tokio::test]
async fn main_loop_request_does_not_set_json_mode() {
    let provider = RecordingProvider {
        captured: Mutex::new(None),
        configured: None,
    };
    let sink = crate::NoopEventSink;
    let mut events = Vec::new();
    let cancellation = CancellationToken::new();
    let mut admission = Admission::new(AnsweringLimits::default(), false);
    let mut messages = vec![ChatMessage::text("user", "hello")];
    let mut ordinary_history_pairs = 0;
    let mut protected_prefix_len = 1;
    receive(ReceiveContext {
        provider: &provider,
        model: "m",
        messages: &mut messages,
        checkpoint: &Default::default(),
        ordinary_history_pairs: &mut ordinary_history_pairs,
        protected_prefix_len: &mut protected_prefix_len,
        definitions: &[],
        context_byte_budget: AgentLimits::default().context_byte_budget,
        request_preflight: None,
        continuation: None,
        reasoning_effort: ReasoningEffort::High,
        remaining_wall: None,
        sink: &sink,
        cancellation: &cancellation,
        events: &mut events,
        admission: &mut admission,
        effect_admission: None,
        require_physical_effect_admission: false,
        call_kind: crate::EffectCallKind::Answer,
        conversation_correlation: CorrelationId::next(),
        invocation_correlation: CorrelationId::next(),
        actor_correlation: None,
        request_ordinal: 1,
    })
    .await
    .expect("receive succeeds");
    let sent = provider
        .captured
        .lock()
        .unwrap()
        .take()
        .expect("a request was sent");
    assert_eq!(sent.response_format, ResponseFormat::Text);
    assert_eq!(
        sent.reasoning_effort,
        ReasoningEffort::High,
        "the first request of a run thinks hard"
    );
}

/// Runs one `receive` and returns the request the provider saw.
async fn captured_request(
    provider: &RecordingProvider,
    messages: Vec<ChatMessage>,
    first_request: bool,
) -> ChatRequest {
    let sink = crate::NoopEventSink;
    let mut events = Vec::new();
    let cancellation = CancellationToken::new();
    let mut admission = Admission::new(AnsweringLimits::default(), false);
    let mut messages = messages;
    let mut ordinary_history_pairs = 0;
    let mut protected_prefix_len = messages.len();
    let reasoning_effort = super::super::reasoning_policy::decide(
        &messages,
        first_request,
        false,
        provider.configured_reasoning_effort(),
    )
    .effort;
    receive(ReceiveContext {
        provider,
        model: "m",
        messages: &mut messages,
        checkpoint: &Default::default(),
        ordinary_history_pairs: &mut ordinary_history_pairs,
        protected_prefix_len: &mut protected_prefix_len,
        definitions: &[],
        context_byte_budget: AgentLimits::default().context_byte_budget,
        request_preflight: None,
        continuation: None,
        reasoning_effort,
        remaining_wall: None,
        sink: &sink,
        cancellation: &cancellation,
        events: &mut events,
        admission: &mut admission,
        effect_admission: None,
        require_physical_effect_admission: false,
        call_kind: crate::EffectCallKind::Answer,
        conversation_correlation: CorrelationId::next(),
        invocation_correlation: CorrelationId::next(),
        actor_correlation: None,
        request_ordinal: if first_request { 1 } else { 2 },
    })
    .await
    .expect("receive succeeds");
    provider
        .captured
        .lock()
        .unwrap()
        .take()
        .expect("a request was sent")
}

/// After a clean tool result, the loop asks for less thinking.
#[tokio::test]
async fn a_clean_follow_up_request_thinks_lightly() {
    let provider = RecordingProvider {
        captured: Mutex::new(None),
        configured: None,
    };
    let messages = vec![
        ChatMessage::text("user", "hello"),
        ChatMessage::text("assistant", "working"),
        ChatMessage {
            role: "tool".into(),
            content: r#"{"exit_code":0}"#.into(),
            tool_call_id: Some("call-1".into()),
            reasoning: None,
            reasoning_signature: None,
            ..Default::default()
        },
    ];
    let sent = captured_request(&provider, messages, false).await;
    assert_eq!(sent.reasoning_effort, ReasoningEffort::Low);
}

/// A pinned effort is sent on every request, even the first.
#[tokio::test]
async fn a_configured_effort_pins_the_request() {
    let provider = RecordingProvider {
        captured: Mutex::new(None),
        configured: Some(ReasoningEffort::Medium),
    };
    let sent = captured_request(&provider, vec![ChatMessage::text("user", "hello")], true).await;
    assert_eq!(sent.reasoning_effort, ReasoningEffort::Medium);
}
