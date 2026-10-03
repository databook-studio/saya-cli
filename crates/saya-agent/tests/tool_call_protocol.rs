use async_trait::async_trait;
use saya_agent::{
    AgentError, AgentEvent, AgentEventSink, AgentLimits, AgentRequest, AllowReadOnlyApproval,
    CancellationToken, ChatMessage, ChatProvider, ChatRequest, ChatResponse, LocalStateEffect,
    ProviderRecoveryPhase, ProviderRecoveryReason, TokenUsage, ToolCall, ToolConcurrency,
    ToolDefinition, ToolEffect, ToolError, ToolExecutor, run_agent_with_sink,
};
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};

struct ScriptedProvider {
    requests: Arc<Mutex<Vec<ChatRequest>>>,
    responses: Mutex<VecDeque<ChatResponse>>,
}

#[async_trait]
impl ChatProvider for ScriptedProvider {
    fn name(&self) -> &str {
        "scripted"
    }

    async fn complete(
        &self,
        request: ChatRequest,
    ) -> Result<ChatResponse, saya_agent::ProviderError> {
        self.requests.lock().unwrap().push(request);
        Ok(self
            .responses
            .lock()
            .unwrap()
            .pop_front()
            .expect("the test supplies every provider response"))
    }
}

struct RecordingTools {
    calls: Arc<Mutex<Vec<String>>>,
}

#[async_trait]
impl ToolExecutor for RecordingTools {
    async fn execute(
        &self,
        name: &str,
        _: serde_json::Value,
    ) -> Result<serde_json::Value, ToolError> {
        self.calls.lock().unwrap().push(name.into());
        Ok(serde_json::json!({"ok": true}))
    }
}

struct RecordingSink {
    events: Arc<Mutex<Vec<AgentEvent>>>,
}

#[async_trait]
impl AgentEventSink for RecordingSink {
    async fn emit(&self, event: AgentEvent) {
        self.events.lock().unwrap().push(event);
    }
}

fn request() -> AgentRequest {
    AgentRequest {
        prompt: "inspect the schema".into(),
        profile_names: vec!["analytics".into()],
        model: "mock-model".into(),
        system_prompt: None,
        history: Vec::new(),
        context_blocks: Vec::new(),
    }
}

fn definitions() -> Vec<ToolDefinition> {
    vec![ToolDefinition {
        name: "schema_discovery".into(),
        description: "inspect schema".into(),
        read_only: true,
        parameters: serde_json::json!({"type": "object"}),
        effect: ToolEffect {
            database_data: false,
            external_side_effect: false,
            requires_approval: false,
            local_state: LocalStateEffect::None,
        },
        concurrency: ToolConcurrency::Serial,
        completion: None,
    }]
}

fn call(id: &str, name: &str, arguments: serde_json::Value) -> ToolCall {
    ToolCall {
        id: id.into(),
        name: name.into(),
        arguments,
    }
}

fn assistant(calls: Vec<ToolCall>) -> ChatMessage {
    ChatMessage {
        role: "assistant".into(),
        content: String::new(),
        tool_calls: calls,
        tool_call_id: None,
    }
}

fn provider(responses: Vec<ChatResponse>) -> (ScriptedProvider, Arc<Mutex<Vec<ChatRequest>>>) {
    let requests = Arc::new(Mutex::new(Vec::new()));
    (
        ScriptedProvider {
            requests: requests.clone(),
            responses: Mutex::new(responses.into()),
        },
        requests,
    )
}

#[tokio::test]
async fn blank_or_duplicate_ids_refuse_the_whole_mixed_batch_before_any_effect() {
    for (label, calls) in [
        (
            "blank",
            vec![
                call("valid", "schema_discovery", serde_json::json!({})),
                call("", "schema_discovery", serde_json::json!({})),
            ],
        ),
        (
            "duplicate",
            vec![
                call("same", "schema_discovery", serde_json::json!({})),
                call("same", "schema_discovery", serde_json::json!({})),
            ],
        ),
    ] {
        let mut response = ChatResponse::new(assistant(calls));
        response.usage = Some(TokenUsage::new(11, 2));
        let (provider, requests) = provider(vec![
            response,
            ChatResponse::new(ChatMessage::text("assistant", "unexpected repair")),
        ]);
        let calls = Arc::new(Mutex::new(Vec::new()));
        let events = Arc::new(Mutex::new(Vec::new()));
        let result = run_agent_with_sink(
            &provider,
            &RecordingTools {
                calls: calls.clone(),
            },
            request(),
            definitions(),
            AgentLimits::default(),
            &AllowReadOnlyApproval,
            &RecordingSink {
                events: events.clone(),
            },
            CancellationToken::new(),
        )
        .await;

        assert!(
            matches!(result, Err(AgentError::InvalidToolCall)),
            "{label} IDs must reject the completed batch: {result:?}"
        );
        assert!(
            calls.lock().unwrap().is_empty(),
            "{label}: no call may execute"
        );
        assert_eq!(
            requests.lock().unwrap().len(),
            1,
            "{label}: no repair retry"
        );
        let events = events.lock().unwrap();
        assert!(
            !events
                .iter()
                .any(|event| matches!(event, AgentEvent::ToolRequested { .. })),
            "{label}: no prefix tool request may escape: {events:?}"
        );
        assert!(
            events.iter().any(|event| matches!(
                event,
                AgentEvent::Usage { usage, .. }
                    if *usage == TokenUsage::new(11, 2)
            )),
            "{label}: reported provider usage remains known: {events:?}"
        );
        assert!(
            events.iter().any(|event| matches!(
                event,
                AgentEvent::ProviderRecovery {
                    phase: ProviderRecoveryPhase::NotRetried,
                    reason: ProviderRecoveryReason::ToolCallProtocol,
                    attempt: 1,
                    limit: 4,
                }
            )),
            "{label}: the refusal needs one bounded typed reason: {events:?}"
        );
    }
}

#[tokio::test]
async fn invalid_ids_preempt_clarification_and_designation_actions() {
    let (provider, _) = provider(vec![
        ChatResponse::new(assistant(vec![
            call(
                "same",
                "request_clarification",
                serde_json::json!({"question": "Which metric?"}),
            ),
            call(
                "same",
                "designate_answer",
                serde_json::json!({"sql": "SELECT 1"}),
            ),
        ])),
        ChatResponse::new(ChatMessage::text("assistant", "unexpected repair")),
    ]);
    let calls = Arc::new(Mutex::new(Vec::new()));
    let events = Arc::new(Mutex::new(Vec::new()));
    let result = run_agent_with_sink(
        &provider,
        &RecordingTools {
            calls: calls.clone(),
        },
        request(),
        definitions(),
        AgentLimits::default(),
        &AllowReadOnlyApproval,
        &RecordingSink {
            events: events.clone(),
        },
        CancellationToken::new(),
    )
    .await;

    assert!(matches!(result, Err(AgentError::InvalidToolCall)));
    assert!(
        calls.lock().unwrap().is_empty(),
        "no special tool may execute"
    );
    let events = events.lock().unwrap();
    assert!(
        !events.iter().any(|event| matches!(
            event,
            AgentEvent::ToolRequested { .. }
                | AgentEvent::ClarificationNeeded { .. }
                | AgentEvent::AnswerDesignated { .. }
        )),
        "invalid IDs must stop special paths before their side actions: {events:?}"
    );
}

#[tokio::test]
async fn the_same_id_in_separate_assistant_messages_executes_once_per_message() {
    let (provider, requests) = provider(vec![
        ChatResponse::new(assistant(vec![call(
            "reused",
            "schema_discovery",
            serde_json::json!({}),
        )])),
        ChatResponse::new(assistant(vec![call(
            "reused",
            "schema_discovery",
            serde_json::json!({}),
        )])),
        ChatResponse::new(ChatMessage::text("assistant", "done")),
    ]);
    let calls = Arc::new(Mutex::new(Vec::new()));
    let output = run_agent_with_sink(
        &provider,
        &RecordingTools {
            calls: calls.clone(),
        },
        request(),
        definitions(),
        AgentLimits::default(),
        &AllowReadOnlyApproval,
        &RecordingSink {
            events: Arc::new(Mutex::new(Vec::new())),
        },
        CancellationToken::new(),
    )
    .await
    .expect("IDs are unique within each message, not across turns");

    assert_eq!(output.answer, "done");
    assert_eq!(
        &*calls.lock().unwrap(),
        &["schema_discovery", "schema_discovery"]
    );
    assert_eq!(requests.lock().unwrap().len(), 3);
}

#[tokio::test]
async fn valid_ids_with_invalid_calls_each_reply_once_without_execution() {
    let (provider, requests) = provider(vec![
        ChatResponse::new(assistant(vec![
            call("unknown", "not_registered", serde_json::json!({})),
            call(
                "malformed",
                "schema_discovery",
                serde_json::json!("not an object"),
            ),
        ])),
        ChatResponse::new(ChatMessage::text("assistant", "corrected")),
    ]);
    let calls = Arc::new(Mutex::new(Vec::new()));
    let output = run_agent_with_sink(
        &provider,
        &RecordingTools {
            calls: calls.clone(),
        },
        request(),
        definitions(),
        AgentLimits::default(),
        &AllowReadOnlyApproval,
        &RecordingSink {
            events: Arc::new(Mutex::new(Vec::new())),
        },
        CancellationToken::new(),
    )
    .await
    .expect("valid IDs keep invalid calls recoverable");

    assert_eq!(output.answer, "corrected");
    assert!(
        calls.lock().unwrap().is_empty(),
        "invalid calls never execute"
    );
    let requests = requests.lock().unwrap();
    let replies = requests[1]
        .messages
        .iter()
        .filter_map(|message| message.tool_call_id.as_deref())
        .collect::<Vec<_>>();
    assert_eq!(replies, ["unknown", "malformed"]);
}

#[tokio::test]
async fn completed_effect_is_not_replayed_when_the_next_batch_has_an_invalid_id() {
    let (provider, requests) = provider(vec![
        ChatResponse::new(assistant(vec![call(
            "completed",
            "schema_discovery",
            serde_json::json!({}),
        )])),
        ChatResponse::new(assistant(vec![call(
            "",
            "schema_discovery",
            serde_json::json!({}),
        )])),
        ChatResponse::new(ChatMessage::text("assistant", "unexpected repair")),
    ]);
    let calls = Arc::new(Mutex::new(Vec::new()));
    let result = run_agent_with_sink(
        &provider,
        &RecordingTools {
            calls: calls.clone(),
        },
        request(),
        definitions(),
        AgentLimits::default(),
        &AllowReadOnlyApproval,
        &RecordingSink {
            events: Arc::new(Mutex::new(Vec::new())),
        },
        CancellationToken::new(),
    )
    .await;

    assert!(matches!(result, Err(AgentError::InvalidToolCall)));
    assert_eq!(&*calls.lock().unwrap(), &["schema_discovery"]);
    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 2, "invalid batch must not retry");
    assert!(
        requests[1]
            .messages
            .iter()
            .any(|message| message.tool_call_id.as_deref() == Some("completed")),
        "the completed result remains in the next request rather than being replayed"
    );
}
