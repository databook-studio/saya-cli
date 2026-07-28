use async_trait::async_trait;
use saya_agent::{
    AgentError, AgentLimits, AgentRequest, AllowReadOnlyApproval, ApprovalDecider, ChatMessage,
    ChatProvider, ChatRequest, ChatResponse, ToolCall, ToolDefinition, ToolExecutor, run_agent,
};
use std::sync::{Arc, Mutex};

struct MockProvider {
    responses: Mutex<Vec<ChatResponse>>,
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
        assert_eq!(request.model, "mock-model");
        self.responses.lock().unwrap().remove(0).pipe(Ok)
    }
}

struct MockTools {
    calls: Arc<Mutex<Vec<String>>>,
}

struct DenyApproval;

#[async_trait]
impl ApprovalDecider for DenyApproval {
    async fn approve(&self, _: &ToolDefinition) -> bool {
        false
    }
}

#[async_trait]
impl ToolExecutor for MockTools {
    async fn execute(&self, name: &str, _: serde_json::Value) -> Result<serde_json::Value, String> {
        self.calls.lock().unwrap().push(name.into());
        Ok(serde_json::json!({"rows": 1}))
    }
}

fn definitions() -> Vec<ToolDefinition> {
    vec![ToolDefinition {
        name: "bounded_sql_query".into(),
        description: "read-only query".into(),
        read_only: true,
        parameters: serde_json::json!({"type":"object"}),
        requires_approval: true,
    }]
}

fn request() -> AgentRequest {
    AgentRequest {
        prompt: "show data".into(),
        profile_names: vec!["analytics".into()],
        model: "mock-model".into(),
    }
}

#[tokio::test]
async fn tool_call_round_trip_is_deterministic_and_emits_safe_events() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let provider = MockProvider {
        responses: Mutex::new(vec![
            ChatResponse {
                message: ChatMessage {
                    role: "assistant".into(),
                    content: String::new(),
                    tool_calls: vec![ToolCall {
                        id: "call-1".into(),
                        name: "bounded_sql_query".into(),
                        arguments: serde_json::json!({"sql":"select 1"}),
                    }],
                    tool_call_id: None,
                },
            },
            ChatResponse {
                message: ChatMessage::text("assistant", "There is one result."),
            },
        ]),
    };
    let output = run_agent(
        &provider,
        &MockTools {
            calls: calls.clone(),
        },
        request(),
        definitions(),
        AgentLimits::default(),
        &AllowReadOnlyApproval,
    )
    .await
    .unwrap();
    assert_eq!(output.answer, "There is one result.");
    assert_eq!(&*calls.lock().unwrap(), &["bounded_sql_query"]);
    assert!(output.events.iter().any(|event| matches!(event, saya_agent::AgentEvent::ToolCompleted { summary, .. } if summary.contains("read-only"))));
}

#[tokio::test]
async fn tool_call_limits_stop_run_before_unbounded_execution() {
    let provider = MockProvider {
        responses: Mutex::new(vec![ChatResponse {
            message: ChatMessage {
                role: "assistant".into(),
                content: String::new(),
                tool_calls: vec![ToolCall {
                    id: "call-1".into(),
                    name: "bounded_sql_query".into(),
                    arguments: serde_json::json!({}),
                }],
                tool_call_id: None,
            },
        }]),
    };
    let error = run_agent(
        &provider,
        &MockTools {
            calls: Arc::new(Mutex::new(Vec::new())),
        },
        request(),
        definitions(),
        AgentLimits {
            max_turns: 1,
            max_tool_calls: 0,
        },
        &AllowReadOnlyApproval,
    )
    .await
    .unwrap_err();
    assert!(matches!(error, AgentError::Limit("tool calls")));
}

#[tokio::test]
async fn malformed_or_unsupported_tool_calls_fail_closed() {
    let provider = MockProvider {
        responses: Mutex::new(vec![ChatResponse {
            message: ChatMessage {
                role: "assistant".into(),
                content: String::new(),
                tool_calls: vec![ToolCall {
                    id: "call-1".into(),
                    name: "shell".into(),
                    arguments: serde_json::json!({}),
                }],
                tool_call_id: None,
            },
        }]),
    };
    let error = run_agent(
        &provider,
        &MockTools {
            calls: Arc::new(Mutex::new(Vec::new())),
        },
        request(),
        definitions(),
        AgentLimits::default(),
        &AllowReadOnlyApproval,
    )
    .await
    .unwrap_err();
    assert!(matches!(error, AgentError::InvalidToolCall));
}

#[tokio::test]
async fn empty_provider_response_is_invalid() {
    let provider = MockProvider {
        responses: Mutex::new(vec![ChatResponse {
            message: ChatMessage::text("assistant", ""),
        }]),
    };
    let error = run_agent(
        &provider,
        &MockTools {
            calls: Arc::new(Mutex::new(Vec::new())),
        },
        request(),
        definitions(),
        AgentLimits::default(),
        &AllowReadOnlyApproval,
    )
    .await
    .unwrap_err();
    assert!(matches!(
        error,
        AgentError::Provider(saya_agent::ProviderError::InvalidResponse)
    ));
}

#[tokio::test]
async fn injected_denial_does_not_execute_query_or_persist_rows() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let provider = MockProvider {
        responses: Mutex::new(vec![
            ChatResponse {
                message: ChatMessage {
                    role: "assistant".into(),
                    content: String::new(),
                    tool_calls: vec![ToolCall {
                        id: "call-1".into(),
                        name: "bounded_sql_query".into(),
                        arguments: serde_json::json!({"sql":"select secret"}),
                    }],
                    tool_call_id: None,
                },
            },
            ChatResponse {
                message: ChatMessage::text("assistant", "The query was denied."),
            },
        ]),
    };
    let output = run_agent(
        &provider,
        &MockTools {
            calls: calls.clone(),
        },
        request(),
        definitions(),
        AgentLimits::default(),
        &DenyApproval,
    )
    .await
    .unwrap();
    assert!(calls.lock().unwrap().is_empty());
    assert!(
        output
            .events
            .iter()
            .any(|event| matches!(event, saya_agent::AgentEvent::ToolDenied { .. }))
    );
    assert!(!output.answer.contains("secret"));
}

trait Pipe: Sized {
    fn pipe<T>(self, f: impl FnOnce(Self) -> T) -> T {
        f(self)
    }
}
impl<T> Pipe for T {}
