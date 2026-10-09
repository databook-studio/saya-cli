//! P2d §4: the loop's completion summary reflects a tool's *declaration*,
//! not its name: the definition's own `completion` text when it carries one,
//! otherwise the generic wording keyed on the declared `read_only`. A write
//! tool (`read_only: false`, e.g. one that persists a candidate claim) must
//! not read as a "read-only" completion — that would be a false statement in
//! the feature whose pitch is that it does not overstate what it knows. The
//! summary is picked from the `ToolDefinition`, so a future write tool is
//! labelled correctly without a name match.
//!
//! These drive `run_agent_with_sink` with a mock provider that issues one tool
//! call, a mock executor, and a recording sink, then assert the `ToolCompleted`
//! summary for the read-only and write directions, for a definition carrying
//! its own completion text, and for the invariants the summary feeds.

use async_trait::async_trait;
use saya_agent::{
    AgentEvent, AgentEventSink, AgentLimits, AgentRequest, ChatProvider, ChatRequest, ChatResponse,
    LocalStateEffect, ToolCall, ToolDefinition, ToolEffect, ToolError, ToolExecutor,
    run_agent_with_sink,
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
        concurrency: saya_agent::ToolConcurrency::Serial,
        completion: None,
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
        concurrency: saya_agent::ToolConcurrency::Serial,
        completion: None,
    }
}

/// A tool that is neither a "read-only database tool" nor a "local-state
/// write" — it writes a file and opens a browser — states what it actually
/// did instead of borrowing a generic label that would be false.
fn custom_completion_tool() -> ToolDefinition {
    ToolDefinition {
        name: "render_chart".into(),
        description: "writes a chart file and opens it".into(),
        read_only: true,
        parameters: serde_json::json!({"type":"object"}),
        effect: ToolEffect {
            database_data: false,
            external_side_effect: true,
            requires_approval: true,
            local_state: LocalStateEffect::None,
        },
        concurrency: saya_agent::ToolConcurrency::Serial,
        completion: Some("chart written and opened".into()),
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
/// These tests are about the *completion text* a tool reports, not about who
/// may run it. `AllowReadOnlyApproval` now denies side-effecting tools — which
/// is correct, and would silently turn every case below into "the tool never
/// ran" rather than a statement about summaries. Approving unconditionally
/// keeps each test measuring the one thing it names.
struct ApproveAll;

#[async_trait::async_trait]
impl saya_agent::ApprovalDecider for ApproveAll {
    async fn approve(&self, _: &ToolDefinition, _: &serde_json::Value) -> bool {
        true
    }
}

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
        &ApproveAll,
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

#[path = "completion_summary/failed_completions.rs"]
mod failed_completions;
#[path = "completion_summary/successful_completions.rs"]
mod successful_completions;
