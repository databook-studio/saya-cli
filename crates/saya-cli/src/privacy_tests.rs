use async_trait::async_trait;
use saya_agent::{
    AgentLimits, AgentRequest, AllowReadOnlyApproval, ChatMessage, ChatProvider, ChatRequest,
    ChatResponse, ToolCall, ToolExecutor, run_agent,
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
    async fn execute(&self, _: &str, _: serde_json::Value) -> Result<serde_json::Value, String> {
        Ok(serde_json::json!({"rows":[["CLOUD_SENTINEL"]]}))
    }
}

fn request() -> AgentRequest {
    AgentRequest {
        prompt: "show data".into(),
        profile_names: vec!["analytics".into()],
        model: "model".into(),
    }
}

fn text_response(value: &str) -> ChatResponse {
    ChatResponse {
        message: ChatMessage::text("assistant", value),
    }
}

#[tokio::test]
async fn cloud_without_sharing_hides_sql_and_never_sends_rows() {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let provider = MockProvider {
        responses: Mutex::new(vec![text_response("schema only")]),
        requests: requests.clone(),
    };
    assert!(!crate::agent_runtime::query_data_allowed(
        saya_config::AiProvider::OpenaiCompatible,
        false
    ));
    let tools = crate::agent_tools::DatabaseTools::definitions(
        crate::agent_runtime::query_data_allowed(saya_config::AiProvider::OpenaiCompatible, false),
    );
    run_agent(
        &provider,
        &SentinelExecutor,
        request(),
        tools,
        AgentLimits::default(),
        &AllowReadOnlyApproval,
    )
    .await
    .unwrap();
    let sent = serde_json::to_string(&requests.lock().unwrap()[0]).unwrap();
    assert!(!sent.contains("bounded_sql_query"));
    assert!(!sent.contains("CLOUD_SENTINEL"));
}

#[tokio::test]
async fn cloud_with_sharing_exposes_sql_and_sends_bounded_rows_to_model_only() {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let provider = MockProvider {
        responses: Mutex::new(vec![
            ChatResponse {
                message: ChatMessage {
                    role: "assistant".into(),
                    content: String::new(),
                    tool_calls: vec![ToolCall {
                        id: "call".into(),
                        name: "bounded_sql_query".into(),
                        arguments: serde_json::json!({"sql":"select 1"}),
                    }],
                    tool_call_id: None,
                },
            },
            text_response("done"),
        ]),
        requests: requests.clone(),
    };
    assert!(crate::agent_runtime::query_data_allowed(
        saya_config::AiProvider::OpenaiCompatible,
        true
    ));
    let tools = crate::agent_tools::DatabaseTools::definitions(
        crate::agent_runtime::query_data_allowed(saya_config::AiProvider::OpenaiCompatible, true),
    );
    run_agent(
        &provider,
        &SentinelExecutor,
        request(),
        tools,
        AgentLimits::default(),
        &AllowReadOnlyApproval,
    )
    .await
    .unwrap();
    let sent = serde_json::to_string(&*requests.lock().unwrap()).unwrap();
    assert!(sent.contains("bounded_sql_query"));
    assert!(sent.contains("CLOUD_SENTINEL"));
}

#[tokio::test]
async fn cloud_without_sharing_blocks_dispatch_even_for_direct_malicious_call() {
    let tools = crate::agent_tools::DatabaseTools::new(None, 10, false);
    let error = tools
        .execute("bounded_sql_query", serde_json::json!({"sql":"select 1"}))
        .await
        .unwrap_err();
    assert!(error.contains("data sharing is disabled"));
}

#[test]
fn session_slash_state_becomes_the_next_prompt_override() {
    let mut state = crate::SessionState::new("session", None, "initial-model");
    state.apply(
        crate::SlashCommand::Provider(Some("openai_compatible".into())),
        &[],
    );
    state.apply(crate::SlashCommand::Model(Some("next-model".into())), &[]);
    state.apply(crate::SlashCommand::Privacy(Some(true)), &[]);
    state.apply(
        crate::SlashCommand::Connect("analytics".into()),
        &["analytics".into()],
    );
    let overrides = state.prompt_overrides();
    assert_eq!(
        overrides.provider,
        Some(saya_config::AiProvider::OpenaiCompatible)
    );
    assert_eq!(overrides.model.as_deref(), Some("next-model"));
    assert_eq!(overrides.allow_data_sharing, Some(true));
    assert_eq!(overrides.profile.as_deref(), Some("analytics"));
}
