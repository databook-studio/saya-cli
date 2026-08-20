//! P2d §4: the loop's completion summary reflects a tool's *declared*
//! `read_only`, not its name. A write tool (`read_only: false`, e.g. one that
//! persists a candidate claim) must not read as a "read-only" completion —
//! that would be a false statement in the feature whose pitch is that it does
//! not overstate what it knows. The summary is picked from
//! `ToolDefinition::read_only`, so a future write tool is labelled correctly
//! without a name match.
//!
//! These drive `run_agent_with_sink` with a mock provider that issues one tool
//! call, a mock executor, and a recording sink, then assert the `ToolCompleted`
//! summary for the read-only and write directions (spec P2d §5.5).

use async_trait::async_trait;
use saya_agent::{
    AgentEvent, AgentEventSink, AgentLimits, AgentRequest, AllowReadOnlyApproval, ChatProvider,
    ChatRequest, ChatResponse, LocalStateEffect, ToolCall, ToolDefinition, ToolEffect, ToolError,
    ToolExecutor, run_agent_with_sink,
};
use std::sync::{Arc, Mutex};

/// A provider that emits one tool call on its first `stream` and a plain text
/// answer after, so the run completes instead of looping on the turn limit.
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

/// An executor that always succeeds — the summary for an `Ok` is what §4 fixes.
struct OkExecutor;

#[async_trait]
impl ToolExecutor for OkExecutor {
    async fn execute(&self, _: &str, _: serde_json::Value) -> Result<serde_json::Value, ToolError> {
        Ok(serde_json::json!({"ok": true}))
    }
}

/// An executor that always fails — the failure summary must still name the
/// direction and keep the "failed" substring (`tool_metadata.status` reads it).
struct FailExecutor;

#[async_trait]
impl ToolExecutor for FailExecutor {
    async fn execute(&self, _: &str, _: serde_json::Value) -> Result<serde_json::Value, ToolError> {
        Err(ToolError::QueryFailed)
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

fn read_only_tool() -> ToolDefinition {
    ToolDefinition {
        name: "bounded_sql_query".into(),
        description: "read-only query".into(),
        read_only: true,
        parameters: serde_json::json!({"type":"object"}),
        effect: ToolEffect {
            database_data: true,
            external_side_effect: false,
            requires_approval: false,
            local_state: LocalStateEffect::None,
        },
    }
}

fn write_tool() -> ToolDefinition {
    ToolDefinition {
        name: "contract_propose".into(),
        description: "persists a candidate claim".into(),
        read_only: false,
        parameters: serde_json::json!({"type":"object"}),
        effect: ToolEffect {
            database_data: true,
            external_side_effect: false,
            requires_approval: false,
            local_state: LocalStateEffect::WriteCandidate,
        },
    }
}

fn request() -> AgentRequest {
    AgentRequest {
        prompt: "do something".into(),
        profile_names: vec!["analytics".into()],
        model: "mock-model".into(),
        system_prompt: None,
        history: Vec::new(),
        context_blocks: Vec::new(),
    }
}

/// Runs one turn issuing a call to `tool`, with writes permitted (so a
/// `WriteCandidate` tool is executed, not denied), returning the sink's events.
async fn run_one(tool: ToolDefinition, executor: &dyn ToolExecutor) -> Vec<AgentEvent> {
    let events = Arc::new(Mutex::new(Vec::new()));
    let provider = OneCallProvider {
        call: ToolCall {
            id: "c1".into(),
            name: tool.name.clone(),
            arguments: serde_json::json!({}),
        },
        turn: Mutex::new(0),
    };
    let sink = RecordingSink {
        events: events.clone(),
    };
    let limits = AgentLimits {
        permit_candidate_writes: true,
        ..AgentLimits::default()
    };
    run_agent_with_sink(
        &provider,
        executor,
        request(),
        vec![tool],
        limits,
        &AllowReadOnlyApproval,
        &sink,
        saya_agent::CancellationToken::new(),
    )
    .await
    .expect("run completes");
    std::mem::take(&mut *events.lock().unwrap())
}

fn completed_summary(events: &[AgentEvent]) -> Option<&str> {
    events.iter().find_map(|event| match event {
        AgentEvent::ToolCompleted { summary, .. } => Some(summary.as_str()),
        _ => None,
    })
}

/// A read-only tool that succeeds reports a "read-only" completion (spec P2d
/// §5.5: the read-only direction).
#[tokio::test]
async fn read_only_tool_reports_read_only_completion() {
    let events = run_one(read_only_tool(), &OkExecutor).await;
    let summary = completed_summary(&events).expect("a ToolCompleted was emitted");
    assert!(
        summary.contains("read-only"),
        "a read-only tool reports read-only: got \"{summary}\""
    );
    assert!(
        !summary.contains("write"),
        "a read-only tool must not report a write: got \"{summary}\""
    );
}

/// A write tool that succeeds reports a "local-state write" completion, *not*
/// "read-only" — the fix at the heart of §4 (spec P2d §5.5: the write direction).
#[tokio::test]
async fn write_tool_does_not_report_read_only_completion() {
    let events = run_one(write_tool(), &OkExecutor).await;
    let summary = completed_summary(&events).expect("a ToolCompleted was emitted");
    assert!(
        summary.contains("write"),
        "a write tool reports a write: got \"{summary}\""
    );
    assert!(
        !summary.contains("read-only"),
        "a write tool must not report read-only — that would overstate: got \"{summary}\""
    );
}

/// A write tool that fails reports a write failure whose summary keeps the
/// "failed" substring, so `tool_metadata.status` still reads "failed".
#[tokio::test]
async fn write_tool_failure_summary_keeps_the_failed_substring() {
    let events = run_one(write_tool(), &FailExecutor).await;
    let summary = completed_summary(&events).expect("a ToolCompleted was emitted");
    assert!(
        summary.contains("write"),
        "a failed write tool still names a write: got \"{summary}\""
    );
    assert!(
        summary.contains("failed"),
        "the failure summary keeps \"failed\" so tool_metadata reads failed: got \"{summary}\""
    );
}

/// A read-only tool that fails reports a read-only failure (the read-only
/// direction still holds on the failure path).
#[tokio::test]
async fn read_only_tool_failure_reports_read_only_failure() {
    let events = run_one(read_only_tool(), &FailExecutor).await;
    let summary = completed_summary(&events).expect("a ToolCompleted was emitted");
    assert!(
        summary.contains("read-only"),
        "a failed read-only tool still reports read-only: got \"{summary}\""
    );
    assert!(
        summary.contains("failed"),
        "the failure summary keeps \"failed\": got \"{summary}\""
    );
}
