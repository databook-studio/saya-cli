//! The test's doubles: a scripted stream provider that counts requests, a
//! no-op tool executor that counts executions, the fixed episode request,
//! and the provider event scripts — one settled success and the terminal
//! tool-collection-limit floods.

use std::sync::Mutex;
use std::{collections::VecDeque, sync::atomic::AtomicUsize};

use async_trait::async_trait;
use saya_agent::{
    AgentRequest, CancellationToken, ChatProvider, ChatRequest, ChatResponse, ProviderError,
    ProviderEvent, ProviderStream, TokenUsage, ToolError, ToolExecutor,
};

pub(super) struct ScriptedProvider {
    streams: Mutex<VecDeque<Vec<ProviderEvent>>>,
    pub(super) requests: AtomicUsize,
}

#[async_trait]
impl ChatProvider for ScriptedProvider {
    fn name(&self) -> &str {
        "receipt-source"
    }

    async fn complete(&self, _: ChatRequest) -> Result<ChatResponse, ProviderError> {
        unreachable!("the test drives provider streams")
    }

    async fn stream(
        &self,
        _: ChatRequest,
        _: CancellationToken,
    ) -> Result<ProviderStream, ProviderError> {
        self.requests
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let events = self.streams.lock().unwrap().pop_front().unwrap();
        Ok(Box::pin(futures_util::stream::iter(
            events.into_iter().map(Ok),
        )))
    }
}

/// The scripted streams the receipt claim rides: a settled success, then a
/// terminal attempt whose 257-tool-call flood trips the collection limit
/// with a reported receipt, then the same flood with no receipt at all, then
/// the resumed turn's answer.
pub(super) fn scripted_receipt_provider() -> ScriptedProvider {
    ScriptedProvider {
        streams: Mutex::new(
            vec![
                success(TokenUsage::new(30, 0).with_cached_input(Some(5))),
                terminal_collection_limit(Some(
                    TokenUsage::new(14, 0).with_cache_creation(Some(0)),
                )),
                terminal_collection_limit(None),
                vec![
                    ProviderEvent::TextDelta("resumed".into()),
                    ProviderEvent::Done,
                ],
            ]
            .into(),
        ),
        requests: AtomicUsize::new(0),
    }
}

#[derive(Default)]
pub(super) struct NoTools(pub(super) AtomicUsize);

#[async_trait]
impl ToolExecutor for NoTools {
    async fn execute(&self, _: &str, _: serde_json::Value) -> Result<serde_json::Value, ToolError> {
        self.0.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        Ok(serde_json::Value::Null)
    }
}

pub(super) fn request() -> AgentRequest {
    AgentRequest {
        prompt: "bounded receipt test".into(),
        profile_names: Vec::new(),
        model: "scripted".into(),
        system_prompt: None,
        history: Vec::new(),
        context_blocks: Vec::new(),
    }
}

fn success(usage: TokenUsage) -> Vec<ProviderEvent> {
    vec![
        ProviderEvent::Usage(usage),
        ProviderEvent::TextDelta("done".into()),
        ProviderEvent::Done,
    ]
}

fn terminal_collection_limit(usage: Option<TokenUsage>) -> Vec<ProviderEvent> {
    let mut events = usage
        .into_iter()
        .map(ProviderEvent::Usage)
        .collect::<Vec<_>>();
    events.push(ProviderEvent::ToolCalls(
        (0..257)
            .map(|id| saya_agent::ToolCall {
                id: format!("call-{id}"),
                name: "unexecuted".into(),
                arguments: serde_json::Value::Null,
            })
            .collect(),
    ));
    events.push(ProviderEvent::Done);
    events
}
