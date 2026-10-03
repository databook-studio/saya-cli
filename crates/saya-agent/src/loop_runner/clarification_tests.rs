//! Tests for the clarification arm (B3c): a `request_clarification` call ends
//! the turn with a `ClarificationNeeded` event and a short tool result — no
//! further provider call — while a malformed call feeds a tool error and the
//! turn continues.

use super::super::clarification_args::{REQUEST_CLARIFICATION_TOOL, parse};
use crate::{
    AgentEvent, AgentEventSink, AgentLimits, AgentRequest, AllowReadOnlyApproval, ApprovalDecider,
    CancellationToken, ChatMessage, ChatProvider, ChatRequest, ChatResponse, LocalStateEffect,
    ProviderError, ProviderEvent, ProviderStream, ToolCall, ToolDefinition, ToolEffect, ToolError,
    ToolExecutor, run_agent_with_sink,
};
use async_trait::async_trait;
use std::sync::{Arc, Mutex};

/// A provider that pops one scripted response per `stream` call and counts
/// the requests it received — the no-further-provider-call assertion reads
/// the count.
struct ScriptedProvider {
    turns: Mutex<Vec<ChatMessage>>,
    requests: Mutex<usize>,
}

#[async_trait]
impl ChatProvider for ScriptedProvider {
    fn name(&self) -> &str {
        "scripted"
    }
    async fn complete(&self, _: ChatRequest) -> Result<ChatResponse, ProviderError> {
        panic!("the loop must drive providers through stream()")
    }
    async fn stream(
        &self,
        _: ChatRequest,
        _: CancellationToken,
    ) -> Result<ProviderStream, ProviderError> {
        *self.requests.lock().unwrap() += 1;
        let message = self.turns.lock().unwrap().remove(0);
        let events = if message.tool_calls.is_empty() {
            vec![
                Ok(ProviderEvent::TextDelta(message.content)),
                Ok(ProviderEvent::Done),
            ]
        } else {
            vec![
                Ok(ProviderEvent::ToolCalls(message.tool_calls)),
                Ok(ProviderEvent::Done),
            ]
        };
        Ok(Box::pin(futures_util::stream::iter(events)))
    }
}

/// Records every executed tool name; answers anything with a fixed marker.
struct RecordingTools {
    calls: Arc<Mutex<Vec<String>>>,
    fail_names: Vec<String>,
}

#[async_trait]
impl ToolExecutor for RecordingTools {
    async fn execute(
        &self,
        name: &str,
        _: serde_json::Value,
    ) -> Result<serde_json::Value, ToolError> {
        self.calls.lock().unwrap().push(name.into());
        if self.fail_names.iter().any(|failed| failed == name) {
            return Err(ToolError::UnsupportedTool);
        }
        Ok(serde_json::json!({"asked": true}))
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

fn clarification_definition() -> ToolDefinition {
    ToolDefinition {
        name: REQUEST_CLARIFICATION_TOOL.into(),
        description: "ask one focused question".into(),
        read_only: true,
        parameters: serde_json::json!({"type": "object"}),
        effect: ToolEffect {
            database_data: false,
            external_side_effect: false,
            requires_approval: false,
            local_state: LocalStateEffect::None,
        },
        concurrency: crate::ToolConcurrency::Serial,
        completion: Some("question asked — the turn pauses for the user's answer".into()),
    }
}

fn definitions() -> Vec<ToolDefinition> {
    vec![
        clarification_definition(),
        ToolDefinition {
            name: "schema_discovery".into(),
            description: "schema discovery".into(),
            read_only: true,
            parameters: serde_json::json!({"type": "object"}),
            effect: ToolEffect {
                database_data: false,
                external_side_effect: false,
                requires_approval: false,
                local_state: LocalStateEffect::None,
            },
            concurrency: crate::ToolConcurrency::Serial,
            completion: None,
        },
    ]
}

fn request() -> AgentRequest {
    AgentRequest {
        prompt: "how many active users".into(),
        profile_names: vec!["analytics".into()],
        model: "mock-model".into(),
        system_prompt: None,
        history: Vec::new(),
        context_blocks: Vec::new(),
    }
}

fn clarification_call(
    question: impl Into<serde_json::Value>,
    options: impl Into<serde_json::Value>,
) -> ToolCall {
    ToolCall {
        id: "call-1".into(),
        name: REQUEST_CLARIFICATION_TOOL.into(),
        arguments: serde_json::json!({"question": question.into(), "options": options.into()}),
    }
}

struct CustomDenyApproval;

#[async_trait]
impl ApprovalDecider for CustomDenyApproval {
    async fn approve(&self, _: &ToolDefinition, _: &serde_json::Value) -> bool {
        false
    }

    fn refusal_detail(&self, _: &ToolDefinition, _: &serde_json::Value) -> Option<String> {
        Some("questions require an approved investigation plan".into())
    }
}

/// The core invariant: a valid `request_clarification` call emits
/// `ClarificationNeeded`, feeds a short tool result, and ENDS the turn — the
/// scripted provider is never called a second time, and the run returns
/// `Ok` rather than an error.
#[tokio::test]
async fn a_clarification_call_ends_the_turn_and_emits_the_event() {
    let provider = ScriptedProvider {
        turns: Mutex::new(vec![ChatMessage {
            role: "assistant".into(),
            content: String::new(),
            tool_calls: vec![clarification_call(
                "Which metric should \"active users\" use?",
                serde_json::json!([
                    "sessions in the last 30 days",
                    "purchases in the last 90 days"
                ]),
            )],
            tool_call_id: None,
        }]),
        requests: Mutex::new(0),
    };
    let calls = Arc::new(Mutex::new(Vec::new()));
    let captured = Arc::new(Mutex::new(Vec::new()));
    let output = run_agent_with_sink(
        &provider,
        &RecordingTools {
            calls: calls.clone(),
            fail_names: Vec::new(),
        },
        request(),
        definitions(),
        AgentLimits::default(),
        &AllowReadOnlyApproval,
        &RecordingSink {
            events: captured.clone(),
        },
        CancellationToken::new(),
    )
    .await
    .expect("the turn ends on the clarification, never in an error");
    assert_eq!(
        *provider.requests.lock().unwrap(),
        1,
        "no further provider call after the clarification"
    );
    let events = captured.lock().unwrap();
    assert!(
        events.iter().any(|event| matches!(
            event,
            AgentEvent::ClarificationNeeded { question, options }
                if question == "Which metric should \"active users\" use?"
                    && *options == vec![
                        "sessions in the last 30 days".to_string(),
                        "purchases in the last 90 days".to_string(),
                    ]
        )),
        "the question and its options must be carried on the event: {events:?}"
    );
    assert!(
        events
            .iter()
            .any(|event| matches!(event, AgentEvent::ToolRequested { name, .. } if name == REQUEST_CLARIFICATION_TOOL)),
        "the ask is surfaced like any tool call: {events:?}"
    );
    assert!(
        events
            .iter()
            .any(|event| matches!(event, AgentEvent::Complete)),
        "the stream terminates: {events:?}"
    );
    assert_eq!(
        &*calls.lock().unwrap(),
        &[REQUEST_CLARIFICATION_TOOL],
        "the ask is executed through the executor (the short tool result), nothing else"
    );
    assert!(
        output
            .tool_metadata
            .iter()
            .any(|item| item.name == REQUEST_CLARIFICATION_TOOL && item.status == "completed"),
        "the completed ask is recorded: {:?}",
        output.tool_metadata
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, AgentEvent::ClarificationNeeded { .. }))
            .count(),
        1,
        "exactly one clarification event: {events:?}"
    );
}

/// A malformed call — a question over the character bound — is a tool error:
/// the model sees the reason, the executor never runs, and the turn
/// continues to a normal answer.
#[tokio::test]
async fn an_invalid_clarification_call_feeds_an_error_and_the_turn_continues() {
    let provider = ScriptedProvider {
        turns: Mutex::new(vec![
            ChatMessage {
                role: "assistant".into(),
                content: String::new(),
                tool_calls: vec![clarification_call("x".repeat(301), serde_json::Value::Null)],
                tool_call_id: None,
            },
            ChatMessage::text("assistant", "assumed nothing, answered"),
        ]),
        requests: Mutex::new(0),
    };
    let calls = Arc::new(Mutex::new(Vec::new()));
    let captured = Arc::new(Mutex::new(Vec::new()));
    let output = run_agent_with_sink(
        &provider,
        &RecordingTools {
            calls: calls.clone(),
            fail_names: Vec::new(),
        },
        request(),
        definitions(),
        AgentLimits::default(),
        &AllowReadOnlyApproval,
        &RecordingSink {
            events: captured.clone(),
        },
        CancellationToken::new(),
    )
    .await
    .expect("a malformed call is recoverable, never fatal");
    assert_eq!(output.answer, "assumed nothing, answered");
    assert_eq!(
        *provider.requests.lock().unwrap(),
        2,
        "the turn continued to a second provider call"
    );
    assert!(
        calls.lock().unwrap().is_empty(),
        "a malformed ask must not execute: {:?}",
        *calls.lock().unwrap()
    );
    assert!(
        output
            .tool_metadata
            .iter()
            .any(|item| item.name == REQUEST_CLARIFICATION_TOOL && item.status == "failed"),
        "the failed call is recorded: {:?}",
        output.tool_metadata
    );
    let events = captured.lock().unwrap();
    assert!(
        events
            .iter()
            .any(|event| matches!(event, AgentEvent::ToolCompleted { summary, .. } if summary.contains("failed validation"))),
        "the tool error is surfaced: {events:?}"
    );
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, AgentEvent::ClarificationNeeded { .. })),
        "no clarification event for a malformed ask: {events:?}"
    );
}

/// A refused ask does not land: it keeps the decider's custom reason, never
/// executes, and lets the provider answer normally on the following turn.
#[tokio::test]
async fn a_denied_clarification_keeps_the_custom_refusal_and_continues() {
    let provider = ScriptedProvider {
        turns: Mutex::new(vec![
            ChatMessage {
                role: "assistant".into(),
                content: String::new(),
                tool_calls: vec![clarification_call(
                    "Which metric should active users use?",
                    serde_json::Value::Null,
                )],
                tool_call_id: None,
            },
            ChatMessage::text("assistant", "answered without asking"),
        ]),
        requests: Mutex::new(0),
    };
    let calls = Arc::new(Mutex::new(Vec::new()));
    let events = Arc::new(Mutex::new(Vec::new()));
    let mut definition = clarification_definition();
    definition.effect.requires_approval = true;
    let output = run_agent_with_sink(
        &provider,
        &RecordingTools {
            calls: calls.clone(),
            fail_names: Vec::new(),
        },
        request(),
        vec![definition],
        AgentLimits::default(),
        &CustomDenyApproval,
        &RecordingSink {
            events: events.clone(),
        },
        CancellationToken::new(),
    )
    .await
    .expect("a denied ask feeds a result and the turn continues");

    assert_eq!(output.answer, "answered without asking");
    assert_eq!(output.tool_metadata[0].status, "denied");
    assert!(
        calls.lock().unwrap().is_empty(),
        "a denied ask must not execute"
    );
    let events = events.lock().unwrap();
    assert!(events.iter().any(|event| matches!(
        event,
        AgentEvent::ToolDenied { reason, .. }
            if reason == "questions require an approved investigation plan"
    )));
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, AgentEvent::ClarificationNeeded { .. })),
        "a refused ask must not ask the user: {events:?}"
    );
}

/// One question per turn, and siblings do not ride along: a message pairing a
/// valid ask with other calls ends the turn on the ask; the siblings are
/// answered without executing.
#[tokio::test]
async fn a_valid_ask_refuses_its_sibling_calls_and_ends_the_turn() {
    let provider = ScriptedProvider {
        turns: Mutex::new(vec![ChatMessage {
            role: "assistant".into(),
            content: String::new(),
            tool_calls: vec![
                ToolCall {
                    id: "call-a".into(),
                    name: "schema_discovery".into(),
                    arguments: serde_json::json!({}),
                },
                clarification_call("Which table holds active users?", serde_json::Value::Null),
            ],
            tool_call_id: None,
        }]),
        requests: Mutex::new(0),
    };
    let calls = Arc::new(Mutex::new(Vec::new()));
    let captured = Arc::new(Mutex::new(Vec::new()));
    let output = run_agent_with_sink(
        &provider,
        &RecordingTools {
            calls: calls.clone(),
            fail_names: Vec::new(),
        },
        request(),
        definitions(),
        AgentLimits::default(),
        &AllowReadOnlyApproval,
        &RecordingSink {
            events: captured.clone(),
        },
        CancellationToken::new(),
    )
    .await
    .expect("the turn ends on the clarification");
    assert_eq!(
        &*calls.lock().unwrap(),
        &[REQUEST_CLARIFICATION_TOOL],
        "the sibling must not execute once the turn pauses: {:?}",
        *calls.lock().unwrap()
    );
    assert!(
        output
            .events
            .iter()
            .any(|event| matches!(event, AgentEvent::ClarificationNeeded { .. })),
        "the question is carried: {:?}",
        output.events
    );
}

/// The executor refusing the ask (an executor without the arm) degrades to a
/// tool error the turn continues from — never a silent end, never a fabricated
/// question.
#[tokio::test]
async fn an_executor_refusal_on_the_ask_keeps_the_turn_going() {
    let provider = ScriptedProvider {
        turns: Mutex::new(vec![
            ChatMessage {
                role: "assistant".into(),
                content: String::new(),
                tool_calls: vec![clarification_call(
                    "Which metric should \"active users\" use?",
                    serde_json::Value::Null,
                )],
                tool_call_id: None,
            },
            ChatMessage::text("assistant", "answered without the ask"),
        ]),
        requests: Mutex::new(0),
    };
    let captured = Arc::new(Mutex::new(Vec::new()));
    let output = run_agent_with_sink(
        &provider,
        &RecordingTools {
            calls: Arc::new(Mutex::new(Vec::new())),
            fail_names: vec![REQUEST_CLARIFICATION_TOOL.into()],
        },
        request(),
        definitions(),
        AgentLimits::default(),
        &AllowReadOnlyApproval,
        &RecordingSink {
            events: captured.clone(),
        },
        CancellationToken::new(),
    )
    .await
    .expect("an executor refusal degrades to a tool error");
    assert_eq!(output.answer, "answered without the ask");
    assert!(
        !output
            .events
            .iter()
            .any(|event| matches!(event, AgentEvent::ClarificationNeeded { .. })),
        "no question event when the ask did not land: {:?}",
        output.events
    );
}

/// The event serializes under its `clarification_needed` type tag with the
/// question and options carried, so JSON/NDJSON consumers read it structurally.
#[test]
fn clarification_needed_serializes_under_its_type_tag() {
    let event = AgentEvent::clarification_needed(
        "Which metric should \"active users\" use?",
        vec!["sessions in the last 30 days".into()],
    );
    let json = serde_json::to_string(&event).expect("serializes");
    assert!(
        json.contains(r#""type":"clarification_needed""#),
        "type tag: {json}"
    );
    assert!(json.contains("active users"), "question carried: {json}");
    assert!(
        json.contains("sessions in the last 30 days"),
        "options carried: {json}"
    );
}

/// The argument bounds: exactly the shape `request_clarification` accepts —
/// a non-empty question up to 300 characters, and up to six options of up to
/// 80 characters each. Over any bound the call is refused with a named reason.
#[test]
fn the_argument_bounds_are_enforced_with_named_reasons() {
    let ok = parse(&serde_json::json!({
        "question": "Which metric should \"active users\" use?",
        "options": ["a", "b", "c", "d", "e", "f"]
    }))
    .expect("six options are the bound, not past it");
    assert_eq!(ok.options.len(), 6);
    assert!(parse(&serde_json::json!({"question": "which one"})).is_ok());

    let over = parse(&serde_json::json!({"question": "x".repeat(301)}))
        .expect_err("a question over the bound is refused");
    assert!(over.to_string().contains("300"), "{over}");
    let empty =
        parse(&serde_json::json!({"question": "   "})).expect_err("an empty question is refused");
    assert!(empty.to_string().contains("empty"), "{empty}");
    let not_string =
        parse(&serde_json::json!({"question": 7})).expect_err("a non-string question is refused");
    assert!(not_string.to_string().contains("string"), "{not_string}");
    let many = parse(&serde_json::json!({
        "question": "which one",
        "options": ["a", "b", "c", "d", "e", "f", "g"]
    }))
    .expect_err("a seventh option is refused");
    assert!(many.to_string().contains("6"), "{many}");
    let long_option = parse(&serde_json::json!({
        "question": "which one",
        "options": ["x".repeat(81)]
    }))
    .expect_err("an option over the bound is refused");
    assert!(long_option.to_string().contains("80"), "{long_option}");
    let not_array = parse(&serde_json::json!({
        "question": "which one",
        "options": "a"
    }))
    .expect_err("non-array options are refused");
    assert!(not_array.to_string().contains("array"), "{not_array}");
    let not_object = parse(&serde_json::json!("question")).expect_err("non-object args refused");
    assert!(not_object.to_string().contains("object"), "{not_object}");
}

/// Sanitisation: control characters never ride the event — the question and
/// each option are cleaned before the loop emits anything.
#[test]
fn the_question_and_options_are_sanitised_and_trimmed() {
    let parsed = parse(&serde_json::json!({
        "question": "\u{001b}[31mwhich metric\u{0007} is right?  ",
        "options": ["  \u{0002}sessions in 30 days  ", "purchases\u{007f}"]
    }))
    .expect("sanitised input still parses");
    assert_eq!(parsed.question, "[31mwhich metric is right?");
    assert_eq!(parsed.options[0], "sessions in 30 days");
    assert_eq!(parsed.options[1], "purchases");
}
