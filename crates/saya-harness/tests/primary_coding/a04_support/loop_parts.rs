use std::sync::{Arc, atomic::Ordering};

use async_trait::async_trait;
use saya_agent::{
    CancellationToken, ChatProvider, ChatRequest, ChatResponse, LocalStateEffect, ProviderError,
    ProviderEvent, ProviderStream, ToolCall, ToolConcurrency, ToolDefinition, ToolEffect,
    ToolError, ToolExecutor,
};

use super::Stats;

pub(super) fn inspection_tool() -> ToolDefinition {
    ToolDefinition {
        name: "inspect".to_owned(),
        description: "inspect one assigned source path".to_owned(),
        read_only: true,
        parameters: serde_json::json!({"type": "object"}),
        effect: ToolEffect {
            database_data: false,
            external_side_effect: false,
            requires_approval: false,
            local_state: LocalStateEffect::Read,
        },
        concurrency: ToolConcurrency::Serial,
        completion: Some("Inspected assigned source".to_owned()),
    }
}

pub(super) struct ScriptedProvider {
    pub(super) stats: Arc<Stats>,
    pub(super) first_turn_tool_calls: usize,
    pub(super) text_deltas: usize,
    pub(super) turn_calls: std::sync::atomic::AtomicUsize,
}

#[async_trait]
impl ChatProvider for ScriptedProvider {
    fn name(&self) -> &str {
        "a04-test-provider"
    }

    async fn complete(&self, _: ChatRequest) -> Result<ChatResponse, ProviderError> {
        unreachable!("the child runner uses the streaming agent path")
    }

    async fn stream(
        &self,
        _: ChatRequest,
        _: CancellationToken,
    ) -> Result<ProviderStream, ProviderError> {
        self.stats.provider_calls.fetch_add(1, Ordering::SeqCst);
        let turn = self.turn_calls.fetch_add(1, Ordering::SeqCst);
        let events = if turn == 0 && self.first_turn_tool_calls > 0 {
            let calls = (0..self.first_turn_tool_calls)
                .map(|index| ToolCall {
                    id: format!("tool-{index}"),
                    name: "inspect".to_owned(),
                    arguments: serde_json::json!({
                        "path": "src/owned.rs",
                        "secret": "PRIVATE-TOOL-ARG-SENTINEL"
                    }),
                })
                .collect();
            vec![Ok(ProviderEvent::ToolCalls(calls)), Ok(ProviderEvent::Done)]
        } else if turn == 0 && self.text_deltas > 0 {
            let mut events = (0..self.text_deltas)
                .map(|_| Ok(ProviderEvent::TextDelta("x".to_owned())))
                .collect::<Vec<_>>();
            events.push(Ok(ProviderEvent::Done));
            events
        } else {
            vec![
                Ok(ProviderEvent::TextDelta("child answer".to_owned())),
                Ok(ProviderEvent::Done),
            ]
        };
        Ok(Box::pin(futures_util::stream::iter(events)))
    }
}

pub(super) struct CountingExecutor(pub(super) Arc<Stats>);

#[async_trait]
impl ToolExecutor for CountingExecutor {
    async fn execute(&self, _: &str, _: serde_json::Value) -> Result<serde_json::Value, ToolError> {
        self.0.executor_calls.fetch_add(1, Ordering::SeqCst);
        Ok(serde_json::json!({
            "status": "inspected",
            "row": "PRIVATE-TOOL-RESULT-SENTINEL"
        }))
    }
}
