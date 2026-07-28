use async_trait::async_trait;
use saya_agent::{
    AgentLimits, AgentRequest, AllowReadOnlyApproval, ChatMessage, ChatProvider, ChatRequest,
    ChatResponse, ToolCall, ToolExecutor, ToolMetadata, run_agent,
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
        history: Vec::new(),
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

#[test]
fn changing_provider_clears_the_previous_provider_endpoint_in_both_directions() {
    let ollama = saya_config::ResolvedAi {
        provider: saya_config::AiProvider::Ollama,
        model: "model".into(),
        base_url: Some("http://ollama.invalid".into()),
        api_key: None,
        allow_data_sharing: false,
    };
    let to_openai = crate::agent_runtime::PromptOverrides {
        provider: Some(saya_config::AiProvider::OpenaiCompatible),
        ..Default::default()
    };
    assert_eq!(
        crate::agent_runtime::effective_ai(&ollama, &to_openai).base_url,
        None
    );

    let openai = saya_config::ResolvedAi {
        provider: saya_config::AiProvider::OpenaiCompatible,
        base_url: Some("https://cloud.invalid/v1".into()),
        ..ollama
    };
    let to_ollama = crate::agent_runtime::PromptOverrides {
        provider: Some(saya_config::AiProvider::Ollama),
        ..Default::default()
    };
    assert_eq!(
        crate::agent_runtime::effective_ai(&openai, &to_ollama).base_url,
        None
    );
}

#[test]
fn clear_removes_canonical_turns_and_visible_messages() {
    let mut state = crate::SessionState::new("session", None, "model");
    state.record_turn(
        "first prompt",
        "first answer",
        true,
        vec![ToolMetadata {
            name: "bounded_sql_query".into(),
            status: "completed".into(),
        }],
    );
    assert!(state.provider_history().len() == 2);
    state.apply(crate::SlashCommand::Clear, &[]);
    assert!(state.messages.is_empty());
    assert!(state.turns.is_empty());
    assert!(state.provider_history().is_empty());
}

#[test]
fn canonical_redacted_turns_do_not_duplicate_legacy_messages_or_tool_payloads() {
    let mut state = crate::SessionState::new("session", None, "model");
    state.record_turn(
        "prompt ROW_SENTINEL",
        "answer ROW_SENTINEL",
        true,
        vec![ToolMetadata {
            name: "bounded_sql_query".into(),
            status: "completed".into(),
        }],
    );
    let saved = state.redacted();
    assert!(saved.messages.is_empty());
    assert_eq!(saved.turns.len(), 1);
    assert!(
        !serde_json::to_string(&saved)
            .unwrap()
            .contains("raw tool arguments")
    );
}

#[test]
fn cloud_privacy_omits_only_database_derived_turns() {
    let mut state = crate::SessionState::new("session", None, "model");
    state.provider = "openai".into();
    state.record_turn("safe prompt", "safe answer", false, Vec::new());
    state.record_turn("row prompt", "CLOUD_ROW_SENTINEL", true, Vec::new());
    let history = state.provider_history();
    assert_eq!(history.len(), 2);
    assert_eq!(history[0].content, "safe prompt");
    assert!(
        !serde_json::to_string(&history)
            .unwrap()
            .contains("CLOUD_ROW_SENTINEL")
    );
    state.allow_data_sharing = true;
    assert!(
        serde_json::to_string(&state.provider_history())
            .unwrap()
            .contains("CLOUD_ROW_SENTINEL")
    );
}
