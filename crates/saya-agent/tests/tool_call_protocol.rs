use async_trait::async_trait;
use saya_agent::{
    AgentError, AgentEvent, AgentEventSink, AgentLimits, AgentRequest, AllowReadOnlyApproval,
    CancellationToken, ChatMessage, ChatProvider, ChatRequest, ChatResponse, DESIGNATE_ANSWER_TOOL,
    LocalStateEffect, ProviderRecoveryPhase, ProviderRecoveryReason, REQUEST_CLARIFICATION_TOOL,
    TokenUsage, ToolCall, ToolConcurrency, ToolDefinition, ToolEffect, ToolError, ToolExecutor,
    run_agent_with_sink,
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
    vec![definition("schema_discovery")]
}

fn definitions_with_shortcuts() -> Vec<ToolDefinition> {
    vec![
        definition("schema_discovery"),
        definition(REQUEST_CLARIFICATION_TOOL),
        definition(DESIGNATE_ANSWER_TOOL),
    ]
}

fn definition(name: &str) -> ToolDefinition {
    ToolDefinition {
        name: name.into(),
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
    }
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
async fn invalid_ids_preempt_each_shortcut_before_its_side_actions() {
    for (label, shortcut, expected) in [
        (
            "clarification",
            call(
                "same",
                REQUEST_CLARIFICATION_TOOL,
                serde_json::json!({"question": "Which metric?"}),
            ),
            "clarification",
        ),
        (
            "designation",
            call(
                "same",
                DESIGNATE_ANSWER_TOOL,
                serde_json::json!({"sql": "SELECT 1"}),
            ),
            "designation",
        ),
    ] {
        let (provider, _) = provider(vec![
            ChatResponse::new(assistant(vec![
                shortcut,
                call("same", "schema_discovery", serde_json::json!({})),
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
            definitions_with_shortcuts(),
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
            "{label}: duplicate IDs must reject the batch"
        );
        assert!(
            calls.lock().unwrap().is_empty(),
            "{label}: no tool executes"
        );
        let events = events.lock().unwrap();
        assert!(
            !events
                .iter()
                .any(|event| matches!(event, AgentEvent::ToolRequested { .. })),
            "{label}: no tool request may escape: {events:?}"
        );
        match expected {
            "clarification" => assert!(
                !events
                    .iter()
                    .any(|event| matches!(event, AgentEvent::ClarificationNeeded { .. })),
                "{label}: the clarification must not land: {events:?}"
            ),
            "designation" => assert!(
                !events
                    .iter()
                    .any(|event| matches!(event, AgentEvent::AnswerDesignated { .. })),
                "{label}: the designation must not land: {events:?}"
            ),
            _ => unreachable!("the table fixes the only shortcut cases"),
        }
    }
}

#[tokio::test]
async fn valid_ids_allow_registered_clarification_and_designation_shortcuts() {
    let clarification_events = Arc::new(Mutex::new(Vec::new()));
    let clarification_calls = Arc::new(Mutex::new(Vec::new()));
    let (clarification_provider, clarification_requests) = provider(vec![
        ChatResponse::new(assistant(vec![call(
            "ask",
            REQUEST_CLARIFICATION_TOOL,
            serde_json::json!({"question": "Which metric?"}),
        )])),
        ChatResponse::new(ChatMessage::text("assistant", "unexpected repair")),
    ]);
    let clarification = run_agent_with_sink(
        &clarification_provider,
        &RecordingTools {
            calls: clarification_calls.clone(),
        },
        request(),
        definitions_with_shortcuts(),
        AgentLimits::default(),
        &AllowReadOnlyApproval,
        &RecordingSink {
            events: clarification_events.clone(),
        },
        CancellationToken::new(),
    )
    .await
    .expect("a valid registered clarification lands");
    assert_eq!(
        &*clarification_calls.lock().unwrap(),
        &[REQUEST_CLARIFICATION_TOOL]
    );
    assert!(clarification.events.iter().any(
        |event| matches!(event, AgentEvent::ClarificationNeeded { question, .. } if question == "Which metric?")
    ));
    assert_eq!(clarification_requests.lock().unwrap().len(), 1);

    let designation_events = Arc::new(Mutex::new(Vec::new()));
    let designation_calls = Arc::new(Mutex::new(Vec::new()));
    let (designation_provider, _) = provider(vec![
        ChatResponse::new(ChatMessage {
            role: "assistant".into(),
            content: "The answer is one.".into(),
            tool_calls: vec![call(
                "designate",
                DESIGNATE_ANSWER_TOOL,
                serde_json::json!({"sql": "SELECT 1"}),
            )],
            tool_call_id: None,
        }),
        ChatResponse::new(ChatMessage::text("assistant", "unexpected repair")),
    ]);
    let designation = run_agent_with_sink(
        &designation_provider,
        &RecordingTools {
            calls: designation_calls.clone(),
        },
        request(),
        definitions_with_shortcuts(),
        AgentLimits::default(),
        &AllowReadOnlyApproval,
        &RecordingSink {
            events: designation_events,
        },
        CancellationToken::new(),
    )
    .await
    .expect("a valid registered designation lands");
    assert_eq!(designation.answer_sql.as_deref(), Some("SELECT 1"));
    assert!(designation_calls.lock().unwrap().is_empty());
    assert!(
        designation.events.iter().any(
            |event| matches!(event, AgentEvent::AnswerDesignated { sql } if sql == "SELECT 1")
        )
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
        .filter(|message| message.role == "tool")
        .filter_map(|message| {
            message
                .tool_call_id
                .as_deref()
                .map(|id| (id, &message.content))
        })
        .collect::<Vec<_>>();
    assert_eq!(replies.len(), 2);
    assert!(
        replies
            .iter()
            .any(|(id, content)| *id == "unknown" && content.contains("unknown tool")),
        "the unknown call has one non-executed error reply: {replies:?}"
    );
    assert!(
        replies.iter().any(|(id, content)| {
            *id == "malformed" && content.contains("arguments must be a JSON object")
        }),
        "the malformed call has one non-executed error reply: {replies:?}"
    );
}

#[tokio::test]
async fn salvage_refuses_an_invalid_completed_batch_and_preserves_prior_work() {
    let mut invalid = ChatResponse::new(assistant(vec![
        call("duplicate", "schema_discovery", serde_json::json!({})),
        call("duplicate", "schema_discovery", serde_json::json!({})),
    ]));
    invalid.usage = Some(TokenUsage::new(7, 2));
    let (provider, requests) = provider(vec![
        ChatResponse::new(assistant(vec![call(
            "completed",
            "schema_discovery",
            serde_json::json!({}),
        )])),
        invalid,
        ChatResponse::new(ChatMessage::text("assistant", "unexpected repair")),
    ]);
    let calls = Arc::new(Mutex::new(Vec::new()));
    let events = Arc::new(Mutex::new(Vec::new()));
    let output = run_agent_with_sink(
        &provider,
        &RecordingTools {
            calls: calls.clone(),
        },
        request(),
        definitions(),
        AgentLimits {
            max_turns: Some(1),
            ..AgentLimits::default()
        },
        &AllowReadOnlyApproval,
        &RecordingSink {
            events: events.clone(),
        },
        CancellationToken::new(),
    )
    .await
    .expect("salvage preserves prior work when its completed response is invalid");

    assert!(output.truncated);
    assert!(
        output.answer.is_empty(),
        "invalid salvage content is discarded"
    );
    assert_eq!(&*calls.lock().unwrap(), &["schema_discovery"]);
    assert_eq!(output.usage, TokenUsage::new(7, 2));
    assert!(output.events.iter().any(|event| matches!(
        event,
        AgentEvent::ProviderRecovery {
            phase: ProviderRecoveryPhase::NotRetried,
            reason: ProviderRecoveryReason::ToolCallProtocol,
            ..
        }
    )));
    assert!(output.events.iter().any(|event| matches!(
        event,
        AgentEvent::Usage { usage, .. } if *usage == TokenUsage::new(7, 2)
    )));
    let requests = requests.lock().unwrap();
    assert_eq!(
        requests.len(),
        2,
        "the invalid salvage batch is not retried"
    );
    assert!(requests[1].tools.is_empty(), "salvage does not offer tools");
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
