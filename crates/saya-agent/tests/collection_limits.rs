use async_trait::async_trait;
use saya_agent::{
    AgentError, AgentEvent, AgentEventSink, AgentLimits, AgentRequest, AllowReadOnlyApproval,
    CancellationToken, ChatProvider, ChatRequest, ChatResponse, LocalStateEffect, ProviderError,
    ProviderEvent, ProviderRecoveryReason, ProviderStream, TokenUsage, ToolCall, ToolConcurrency,
    ToolDefinition, ToolEffect, ToolError, ToolExecutor, run_agent_with_sink,
};
use std::sync::{Arc, Mutex};

struct ScriptedProvider {
    responses: Mutex<Vec<Vec<Result<ProviderEvent, ProviderError>>>>,
    calls: Arc<Mutex<usize>>,
}

#[async_trait]
impl ChatProvider for ScriptedProvider {
    fn name(&self) -> &str {
        "scripted"
    }

    async fn complete(&self, _: ChatRequest) -> Result<ChatResponse, ProviderError> {
        unreachable!("the test uses stream")
    }

    async fn stream(
        &self,
        _: ChatRequest,
        _: CancellationToken,
    ) -> Result<ProviderStream, ProviderError> {
        *self.calls.lock().unwrap() += 1;
        Ok(Box::pin(futures_util::stream::iter(
            self.responses.lock().unwrap().remove(0),
        )))
    }
}

struct RecordingTools(Arc<Mutex<usize>>);

#[async_trait]
impl ToolExecutor for RecordingTools {
    async fn execute(&self, _: &str, _: serde_json::Value) -> Result<serde_json::Value, ToolError> {
        *self.0.lock().unwrap() += 1;
        Ok(serde_json::json!({"ok": true}))
    }
}

struct RecordingSink(Arc<Mutex<Vec<AgentEvent>>>);

#[async_trait]
impl AgentEventSink for RecordingSink {
    async fn emit(&self, event: AgentEvent) {
        self.0.lock().unwrap().push(event);
    }
}

fn definition() -> ToolDefinition {
    ToolDefinition {
        name: "schema_discovery".into(),
        description: "schema".into(),
        read_only: true,
        parameters: serde_json::json!({"type":"object"}),
        effect: ToolEffect {
            database_data: false,
            external_side_effect: false,
            requires_approval: false,
            local_state: LocalStateEffect::None,
        },
        concurrency: ToolConcurrency::Serial,
        completion: None,
    }
}

fn request() -> AgentRequest {
    AgentRequest {
        prompt: "hello".into(),
        profile_names: Vec::new(),
        model: "m".into(),
        system_prompt: None,
        history: Vec::new(),
        context_blocks: Vec::new(),
    }
}

fn call(id: usize, arguments: serde_json::Value) -> ToolCall {
    ToolCall {
        id: format!("call-{id}"),
        name: "schema_discovery".into(),
        arguments,
    }
}

async fn run(
    responses: Vec<Vec<Result<ProviderEvent, ProviderError>>>,
) -> (Result<String, AgentError>, usize, usize, Vec<AgentEvent>) {
    let provider_calls = Arc::new(Mutex::new(0));
    let tool_calls = Arc::new(Mutex::new(0));
    let events = Arc::new(Mutex::new(Vec::new()));
    let output = run_agent_with_sink(
        &ScriptedProvider {
            responses: Mutex::new(responses),
            calls: provider_calls.clone(),
        },
        &RecordingTools(tool_calls.clone()),
        request(),
        vec![definition()],
        AgentLimits::default(),
        &AllowReadOnlyApproval,
        &RecordingSink(events.clone()),
        CancellationToken::new(),
    )
    .await
    .map(|output| output.answer);
    (
        output,
        *provider_calls.lock().unwrap(),
        *tool_calls.lock().unwrap(),
        events.lock().unwrap().clone(),
    )
}

fn natural_answer() -> Vec<Result<ProviderEvent, ProviderError>> {
    vec![
        Ok(ProviderEvent::TextDelta("done".into())),
        Ok(ProviderEvent::Done),
    ]
}

#[tokio::test]
async fn count_limit_is_terminal_before_an_eligible_tool_effect() {
    let (output, provider_calls, tool_calls, events) = run(vec![
        vec![
            Ok(ProviderEvent::Usage(TokenUsage::new(5, 8))),
            Ok(ProviderEvent::ToolCalls(
                (0..128).map(|id| call(id, serde_json::json!({}))).collect(),
            )),
            Ok(ProviderEvent::ToolCalls(
                (128..257)
                    .map(|id| call(id, serde_json::json!({})))
                    .collect(),
            )),
            Ok(ProviderEvent::Done),
        ],
        natural_answer(),
    ])
    .await;
    assert_eq!(tool_calls, 0);
    assert_eq!(provider_calls, 1);
    assert!(matches!(
        output,
        Err(AgentError::Provider(ProviderError::ToolCollectionLimit))
    ));
    assert!(events.iter().any(|event| matches!(event, AgentEvent::FailedAttemptUsage { usage } if *usage == TokenUsage::new(5, 8))));
    assert!(events.iter().any(|event| matches!(
        event,
        AgentEvent::ProviderRecovery {
            reason: ProviderRecoveryReason::ToolCollectionLimit,
            ..
        }
    )));
}

#[tokio::test]
async fn oversized_object_arguments_are_terminal_before_an_eligible_tool_effect() {
    let (output, provider_calls, tool_calls, events) = run(vec![
        vec![
            Ok(ProviderEvent::Usage(TokenUsage::new(3, 2))),
            Ok(ProviderEvent::ToolCalls(vec![call(
                1,
                serde_json::json!({"payload": "x".repeat(saya_agent::MAX_STREAM_BYTES)}),
            )])),
            Ok(ProviderEvent::Done),
        ],
        natural_answer(),
    ])
    .await;
    assert_eq!(tool_calls, 0);
    assert_eq!(provider_calls, 1);
    assert!(matches!(
        output,
        Err(AgentError::Provider(ProviderError::ToolCollectionLimit))
    ));
    assert!(events.iter().any(|event| matches!(event, AgentEvent::FailedAttemptUsage { usage } if *usage == TokenUsage::new(3, 2))));
}

#[tokio::test]
async fn eligible_tool_batches_execute_exactly_once_per_call() {
    for count in [1, 256] {
        let (output, provider_calls, tool_calls, _) = run(vec![
            vec![
                Ok(ProviderEvent::ToolCalls(
                    (0..count)
                        .map(|id| call(id, serde_json::json!({})))
                        .collect(),
                )),
                Ok(ProviderEvent::Done),
            ],
            natural_answer(),
        ])
        .await;
        assert_eq!(output.unwrap(), "done", "count {count}");
        assert_eq!(provider_calls, 2, "count {count}");
        assert_eq!(tool_calls, count, "count {count}");
    }
}
