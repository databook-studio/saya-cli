//! C0 — name the call: each completion line carries the call's key fact.
//!
//! The summary half of the properties, placed beside the summaries it pins
//! (integration over the loop's public surface, per the testing standard):
//! a `workspace_write` completion names the file, a `run_command`
//! completion names the program and its outcome, a failed call still reads
//! as failed, and an uncovered tool keeps today's text byte-exact. The
//! request half (shared detail seam, both adapters) lives beside the seam
//! in `saya-cli/src/agent/tools/tool_calls.rs`.

use async_trait::async_trait;
use saya_agent::{
    AgentEvent, AgentEventSink, AgentLimits, AgentRequest, ChatProvider, ChatRequest, ChatResponse,
    LocalStateEffect, ToolCall, ToolDefinition, ToolEffect, ToolError, ToolExecutor,
    run_agent_with_sink,
};
use std::sync::{Arc, Mutex};

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

struct OkExecutor {
    result: serde_json::Value,
}

#[async_trait]
impl ToolExecutor for OkExecutor {
    async fn execute(&self, _: &str, _: serde_json::Value) -> Result<serde_json::Value, ToolError> {
        Ok(self.result.clone())
    }
}

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

struct ApproveAll;

#[async_trait::async_trait]
impl saya_agent::ApprovalDecider for ApproveAll {
    async fn approve(&self, _: &ToolDefinition, _: &serde_json::Value) -> bool {
        true
    }
}

fn workspace_write_tool() -> ToolDefinition {
    ToolDefinition {
        name: "workspace_write".into(),
        description: "write one file".into(),
        read_only: false,
        parameters: serde_json::json!({"type":"object"}),
        effect: ToolEffect {
            database_data: false,
            external_side_effect: false,
            requires_approval: false,
            local_state: LocalStateEffect::WriteWorkspace,
        },
        concurrency: saya_agent::ToolConcurrency::Serial,
        completion: Some("workspace file written".into()),
    }
}

fn run_command_tool() -> ToolDefinition {
    ToolDefinition {
        name: "run_command".into(),
        description: "run one program".into(),
        read_only: false,
        parameters: serde_json::json!({"type":"object"}),
        effect: ToolEffect {
            database_data: false,
            external_side_effect: true,
            requires_approval: false,
            local_state: LocalStateEffect::WriteWorkspace,
        },
        concurrency: saya_agent::ToolConcurrency::Serial,
        completion: Some("host command ran".into()),
    }
}

fn uncovered_tool() -> ToolDefinition {
    ToolDefinition {
        name: "workspace_list".into(),
        description: "list one directory".into(),
        read_only: true,
        parameters: serde_json::json!({"type":"object"}),
        effect: ToolEffect {
            database_data: false,
            external_side_effect: false,
            requires_approval: false,
            local_state: LocalStateEffect::Read,
        },
        concurrency: saya_agent::ToolConcurrency::Serial,
        completion: Some("workspace directory listed".into()),
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

async fn run_one(
    tool: ToolDefinition,
    arguments: serde_json::Value,
    executor: &dyn ToolExecutor,
) -> Vec<AgentEvent> {
    let events = Arc::new(Mutex::new(Vec::new()));
    let provider = OneCallProvider {
        call: ToolCall {
            id: "c1".into(),
            name: tool.name.clone(),
            arguments,
        },
        turn: Mutex::new(0),
    };
    let sink = RecordingSink {
        events: events.clone(),
    };
    let limits = AgentLimits {
        permit_candidate_writes: true,
        permit_workspace_writes: true,
        permit_external_effects: true,
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

fn tool_status(output: &saya_agent::AgentOutput) -> &str {
    output.tool_metadata[0].status.as_str()
}

#[path = "c0_name_the_call/failure_summaries.rs"]
mod failure_summaries;
#[path = "c0_name_the_call/success_summaries.rs"]
mod success_summaries;
