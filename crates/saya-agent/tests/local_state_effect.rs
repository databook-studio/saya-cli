//! Phase 3a: the loop's fail-closed guard for `WriteCandidate` tools.
//!
//! A tool declaring `LocalStateEffect::WriteCandidate` must be *denied* (a
//! `ToolDenied` event with a reason, the turn continuing) when the runner was
//! not constructed with candidate writes permitted, and must run normally when
//! the permission is on. Tools declaring `None` or `Read` are unaffected either
//! way. See .claude/specs/spec-3a-local-state-effect.md §3–§4.

use async_trait::async_trait;
use saya_agent::{
    AgentEvent, AgentEventSink, AgentLimits, AgentRequest, AllowReadOnlyApproval, ChatProvider,
    ChatRequest, ChatResponse, LocalStateEffect, ToolCall, ToolDefinition, ToolEffect, ToolError,
    ToolExecutor, run_agent_with_sink,
};
use std::sync::{Arc, Mutex};

/// A provider that emits one tool call on its first `stream` invocation and a
/// plain text answer on every subsequent one, so a run that survives a denial
/// (or executes the tool) completes instead of looping on the turn limit.
/// `stream` is called once per turn, so the counter distinguishes them.
struct OneCallProvider {
    call: ToolCall,
    turn: Mutex<u32>,
}

#[async_trait]
impl ChatProvider for OneCallProvider {
    fn name(&self) -> &str {
        "one-call-mock"
    }
    async fn complete(&self, _: ChatRequest) -> Result<ChatResponse, saya_agent::ProviderError> {
        unreachable!("stream path is used")
    }
    async fn stream(
        &self,
        _: ChatRequest,
        _: saya_agent::CancellationToken,
    ) -> Result<saya_agent::ProviderStream, saya_agent::ProviderError> {
        let first = {
            let mut turn = self.turn.lock().unwrap();
            let was = *turn;
            *turn += 1;
            was == 0
        };
        let events = if first {
            vec![
                Ok(saya_agent::ProviderEvent::ToolCalls(vec![
                    self.call.clone(),
                ])),
                Ok(saya_agent::ProviderEvent::Done),
            ]
        } else {
            vec![
                Ok(saya_agent::ProviderEvent::TextDelta("done".into())),
                Ok(saya_agent::ProviderEvent::Done),
            ]
        };
        Ok(Box::pin(futures_util::stream::iter(events)))
    }
}

struct RecordingExecutor {
    calls: Arc<Mutex<Vec<String>>>,
}

#[async_trait]
impl ToolExecutor for RecordingExecutor {
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

fn candidate_tool() -> ToolDefinition {
    ToolDefinition {
        name: "remember_candidate".into(),
        description: "may persist a candidate claim".into(),
        read_only: false,
        parameters: serde_json::json!({"type": "object"}),
        effect: ToolEffect {
            database_data: false,
            external_side_effect: false,
            requires_approval: false,
            local_state: LocalStateEffect::WriteCandidate,
        },
    }
}

fn request() -> AgentRequest {
    AgentRequest {
        prompt: "remember something".into(),
        profile_names: vec!["analytics".into()],
        model: "mock-model".into(),
        system_prompt: None,
        history: Vec::new(),
        context_blocks: Vec::new(),
    }
}

/// A `WriteCandidate` tool is denied — not executed — when candidate writes
/// are not permitted (the default), surfacing as a `ToolDenied` event with a
/// reason, and the turn continues to completion.
#[tokio::test]
async fn write_candidate_tool_is_denied_by_default_and_does_not_end_the_turn() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let events = Arc::new(Mutex::new(Vec::new()));
    let provider = OneCallProvider {
        call: ToolCall {
            id: "c1".into(),
            name: "remember_candidate".into(),
            arguments: serde_json::json!({}),
        },
        turn: Mutex::new(0),
    };
    let sink = RecordingSink {
        events: events.clone(),
    };
    let token = saya_agent::CancellationToken::new();
    let output = run_agent_with_sink(
        &provider,
        &RecordingExecutor {
            calls: calls.clone(),
        },
        request(),
        vec![candidate_tool()],
        AgentLimits::default(),
        &AllowReadOnlyApproval,
        &sink,
        token,
    )
    .await
    .expect("denial is not a turn-ending error");
    assert!(
        calls.lock().unwrap().is_empty(),
        "the tool must not execute when candidate writes are not permitted"
    );
    let denied = events.lock().unwrap().iter().find_map(|event| match event {
        AgentEvent::ToolDenied { name, reason } => Some((name.clone(), reason.clone())),
        _ => None,
    });
    let (name, reason) = denied.expect("a ToolDenied event must be emitted");
    assert_eq!(name, "remember_candidate");
    assert!(
        !reason.is_empty(),
        "the denial must carry a clear reason, not be a silent skip"
    );
    // The turn continued past the denial to a normal completion.
    assert!(
        output
            .events
            .iter()
            .any(|event| matches!(event, AgentEvent::Complete)),
        "the turn must complete, not end on the denial"
    );
    assert_eq!(output.tool_metadata[0].name, "remember_candidate");
    assert_eq!(output.tool_metadata[0].status, "denied");
}

/// The same `WriteCandidate` tool executes normally when the permission is on.
#[tokio::test]
async fn write_candidate_tool_runs_when_candidate_writes_are_permitted() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let events = Arc::new(Mutex::new(Vec::new()));
    let provider = OneCallProvider {
        call: ToolCall {
            id: "c1".into(),
            name: "remember_candidate".into(),
            arguments: serde_json::json!({}),
        },
        turn: Mutex::new(0),
    };
    let sink = RecordingSink {
        events: events.clone(),
    };
    let token = saya_agent::CancellationToken::new();
    let limits = AgentLimits {
        permit_candidate_writes: true,
        ..AgentLimits::default()
    };
    let output = run_agent_with_sink(
        &provider,
        &RecordingExecutor {
            calls: calls.clone(),
        },
        request(),
        vec![candidate_tool()],
        limits,
        &AllowReadOnlyApproval,
        &sink,
        token,
    )
    .await
    .expect("run completes");
    assert_eq!(&*calls.lock().unwrap(), &["remember_candidate"]);
    assert!(
        !output
            .events
            .iter()
            .any(|event| matches!(event, AgentEvent::ToolDenied { .. })),
        "no denial when candidate writes are permitted"
    );
    assert_eq!(output.tool_metadata[0].status, "completed");
}

/// A read-only local-state tool is unaffected by the permission either way.
#[tokio::test]
async fn read_local_state_tool_is_unaffected_by_the_candidate_permission() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let events = Arc::new(Mutex::new(Vec::new()));
    let provider = OneCallProvider {
        call: ToolCall {
            id: "c1".into(),
            name: "contract_search".into(),
            arguments: serde_json::json!({}),
        },
        turn: Mutex::new(0),
    };
    let sink = RecordingSink {
        events: events.clone(),
    };
    let token = saya_agent::CancellationToken::new();
    let read_tool = ToolDefinition {
        name: "contract_search".into(),
        description: "reads local contracts".into(),
        read_only: true,
        parameters: serde_json::json!({"type": "object"}),
        effect: ToolEffect {
            database_data: false,
            external_side_effect: false,
            requires_approval: false,
            local_state: LocalStateEffect::Read,
        },
    };
    // Default (not permitted) — a Read tool must still run.
    let _ = run_agent_with_sink(
        &provider,
        &RecordingExecutor {
            calls: calls.clone(),
        },
        request(),
        vec![read_tool.clone()],
        AgentLimits::default(),
        &AllowReadOnlyApproval,
        &sink,
        token,
    )
    .await
    .expect("run completes");
    assert_eq!(&*calls.lock().unwrap(), &["contract_search"]);
    assert!(
        !events
            .lock()
            .unwrap()
            .iter()
            .any(|event| matches!(event, AgentEvent::ToolDenied { .. })),
        "a Read tool must not be denied by the candidate-write guard"
    );
}
