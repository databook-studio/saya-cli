//! Tests for `receive` — the single-attempt request shape and (via the
//! `agent_loop.rs` integration suite) the mid-stream retry policy.

use super::*;
use crate::{
    AgentError, AgentEvent, AgentEventSink, AgentLimits, AgentRequest, AllowReadOnlyApproval,
    ChatResponse, ProviderStream, ReasoningEffort, ResponseFormat, ToolCall, ToolError,
    ToolExecutor, run_agent_with_sink,
};
use crate::{ProviderError, ProviderEvent, ProviderRecoveryReason, TokenUsage};
use async_trait::async_trait;
use futures_util::stream;
use std::sync::{Arc, Mutex};

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

struct OverflowProvider {
    calls: Arc<Mutex<usize>>,
}

#[async_trait]
impl ChatProvider for OverflowProvider {
    fn name(&self) -> &str {
        "overflow"
    }

    async fn complete(&self, _: ChatRequest) -> Result<ChatResponse, ProviderError> {
        unreachable!("the loop streams")
    }

    async fn stream(
        &self,
        _: ChatRequest,
        _: CancellationToken,
    ) -> Result<ProviderStream, ProviderError> {
        *self.calls.lock().unwrap() += 1;
        let call = |id| ToolCall {
            id: format!("call-{id}"),
            name: "schema_discovery".into(),
            arguments: serde_json::json!({}),
        };
        let events = vec![
            Ok(ProviderEvent::Usage(TokenUsage::new(5, 8))),
            Ok(ProviderEvent::ToolCalls((0..128).map(call).collect())),
            Ok(ProviderEvent::ToolCalls((128..257).map(call).collect())),
            Ok(ProviderEvent::Done),
        ];
        Ok(Box::pin(stream::iter(events)))
    }
}

struct CountingTools(Arc<Mutex<usize>>);

#[async_trait]
impl ToolExecutor for CountingTools {
    async fn execute(&self, _: &str, _: serde_json::Value) -> Result<serde_json::Value, ToolError> {
        *self.0.lock().unwrap() += 1;
        Ok(serde_json::json!({}))
    }
}

struct Events(Arc<Mutex<Vec<AgentEvent>>>);

#[async_trait]
impl AgentEventSink for Events {
    async fn emit(&self, event: AgentEvent) {
        self.0.lock().unwrap().push(event);
    }
}

#[tokio::test]
async fn collection_limit_is_terminal_before_any_tool_effect() {
    let provider_calls = Arc::new(Mutex::new(0));
    let tool_calls = Arc::new(Mutex::new(0));
    let events = Arc::new(Mutex::new(Vec::new()));
    let error = run_agent_with_sink(
        &OverflowProvider {
            calls: provider_calls.clone(),
        },
        &CountingTools(tool_calls.clone()),
        AgentRequest {
            prompt: "hello".into(),
            profile_names: Vec::new(),
            model: "m".into(),
            system_prompt: None,
            history: Vec::new(),
            context_blocks: Vec::new(),
        },
        Vec::new(),
        AgentLimits::default(),
        &AllowReadOnlyApproval,
        &Events(events.clone()),
        CancellationToken::new(),
    )
    .await
    .unwrap_err();
    assert!(matches!(
        error,
        AgentError::Provider(ProviderError::ToolCollectionLimit)
    ));
    assert_eq!(
        *provider_calls.lock().unwrap(),
        1,
        "collection failure is terminal"
    );
    assert_eq!(
        *tool_calls.lock().unwrap(),
        0,
        "no tool effect precedes rejection"
    );
    assert!(events.lock().unwrap().iter().any(|event| {
        matches!(event, AgentEvent::FailedAttemptUsage { usage } if *usage == TokenUsage::new(5, 8))
    }));
    assert!(events.lock().unwrap().iter().any(|event| {
        matches!(
            event,
            AgentEvent::ProviderRecovery {
                reason: ProviderRecoveryReason::ToolCollectionLimit,
                ..
            }
        )
    }));
}

struct OversizedArgumentsProvider {
    calls: Arc<Mutex<usize>>,
}

#[async_trait]
impl ChatProvider for OversizedArgumentsProvider {
    fn name(&self) -> &str {
        "oversized-arguments"
    }

    async fn complete(&self, _: ChatRequest) -> Result<ChatResponse, ProviderError> {
        unreachable!("the loop streams")
    }

    async fn stream(
        &self,
        _: ChatRequest,
        _: CancellationToken,
    ) -> Result<ProviderStream, ProviderError> {
        *self.calls.lock().unwrap() += 1;
        let events = vec![
            Ok(ProviderEvent::Usage(TokenUsage::new(3, 2))),
            Ok(ProviderEvent::ToolCalls(vec![ToolCall {
                id: "call-oversized".into(),
                name: "schema_discovery".into(),
                arguments: serde_json::Value::String("x".repeat(crate::MAX_STREAM_BYTES)),
            }])),
        ];
        Ok(Box::pin(stream::iter(events)))
    }
}

#[tokio::test]
async fn oversized_tool_arguments_are_terminal_before_any_tool_effect() {
    let provider_calls = Arc::new(Mutex::new(0));
    let tool_calls = Arc::new(Mutex::new(0));
    let events = Arc::new(Mutex::new(Vec::new()));
    let error = run_agent_with_sink(
        &OversizedArgumentsProvider {
            calls: provider_calls.clone(),
        },
        &CountingTools(tool_calls.clone()),
        AgentRequest {
            prompt: "hello".into(),
            profile_names: Vec::new(),
            model: "m".into(),
            system_prompt: None,
            history: Vec::new(),
            context_blocks: Vec::new(),
        },
        Vec::new(),
        AgentLimits::default(),
        &AllowReadOnlyApproval,
        &Events(events.clone()),
        CancellationToken::new(),
    )
    .await
    .unwrap_err();
    assert!(matches!(
        error,
        AgentError::Provider(ProviderError::ToolCollectionLimit)
    ));
    assert_eq!(*provider_calls.lock().unwrap(), 1);
    assert_eq!(*tool_calls.lock().unwrap(), 0);
    assert!(events.lock().unwrap().iter().any(|event| {
        matches!(event, AgentEvent::FailedAttemptUsage { usage } if *usage == TokenUsage::new(3, 2))
    }));
}
