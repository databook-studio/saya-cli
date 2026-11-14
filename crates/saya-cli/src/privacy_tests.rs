use async_trait::async_trait;
use saya_agent::{
    AgentLimits, AgentRequest, AllowReadOnlyApproval, ChatMessage, ChatProvider, ChatRequest,
    ChatResponse, ToolCall, ToolError, ToolExecutor, ToolMetadata, run_agent,
};
use std::sync::{Arc, Mutex};

struct MockProvider {
    responses: Mutex<Vec<ChatResponse>>,
    requests: Arc<Mutex<Vec<serde_json::Value>>>,
}

#[async_trait]
impl ChatProvider for MockProvider {
    fn name(&self) -> &str {
        "mock"
    }
    async fn complete(
        &self,
        request: ChatRequest,
    ) -> Result<ChatResponse, saya_agent::ProviderError> {
        self.requests
            .lock()
            .unwrap()
            .push(serde_json::to_value(request).unwrap());
        Ok(self.responses.lock().unwrap().remove(0))
    }
}

struct SentinelExecutor;

#[async_trait]
impl ToolExecutor for SentinelExecutor {
    async fn execute(&self, _: &str, _: serde_json::Value) -> Result<serde_json::Value, ToolError> {
        Ok(serde_json::json!({"rows":[["CLOUD_SENTINEL"]]}))
    }
}

fn request() -> AgentRequest {
    AgentRequest {
        prompt: "show data".into(),
        profile_names: vec!["analytics".into()],
        model: "model".into(),
        system_prompt: None,
        history: Vec::new(),
        context_blocks: Vec::new(),
    }
}

fn text_response(value: &str) -> ChatResponse {
    ChatResponse::new(ChatMessage::text("assistant", value))
}

include!("privacy_tests/query_sharing.rs");

include!("privacy_tests/session_history.rs");

include!("privacy_tests/provider_gates.rs");

include!("privacy_tests/endpoint_policy.rs");
