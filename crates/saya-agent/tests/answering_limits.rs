use async_trait::async_trait;
use saya_agent::{
    AgentError, AgentEvent, AgentLimits, AgentRequest, AgentRunControl, AllowReadOnlyApproval,
    AnsweringLimits, AnsweringObservation, BudgetDimension, CancellationReason, CancellationToken,
    ChatProvider, ChatRequest, LocalStateEffect, ProviderError, ProviderEvent, ProviderStream,
    TokenUsage, ToolCall, ToolConcurrency, ToolDefinition, ToolEffect, ToolError,
    ToolExecutionContext, ToolExecutor, run_agent_with_control,
};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};
use tokio::sync::oneshot;

fn request() -> AgentRequest {
    AgentRequest {
        prompt: "inspect the data".into(),
        profile_names: vec!["analytics".into()],
        model: "test-model".into(),
        system_prompt: None,
        history: Vec::new(),
        context_blocks: Vec::new(),
    }
}

fn definition(name: &str) -> ToolDefinition {
    ToolDefinition {
        name: name.into(),
        description: "bounded test tool".into(),
        read_only: true,
        parameters: serde_json::json!({"type": "object"}),
        effect: ToolEffect {
            database_data: true,
            external_side_effect: false,
            requires_approval: false,
            local_state: LocalStateEffect::None,
        },
        concurrency: ToolConcurrency::Serial,
        completion: None,
    }
}

fn definitions() -> Vec<ToolDefinition> {
    vec![definition("query"), definition("request_clarification")]
}

fn call(id: &str, name: &str, arguments: serde_json::Value) -> ToolCall {
    ToolCall {
        id: id.into(),
        name: name.into(),
        arguments,
    }
}

fn query_call(id: &str) -> ToolCall {
    call(id, "query", serde_json::json!({"sql": "SELECT 1"}))
}

fn designation_call(id: &str) -> ToolCall {
    call(
        id,
        "designate_answer",
        serde_json::json!({"sql": "SELECT 1"}),
    )
}

fn turn(content: &str, calls: Vec<ToolCall>) -> Turn {
    let mut events = Vec::new();
    if !content.is_empty() {
        events.push(Ok(ProviderEvent::TextDelta(content.into())));
    }
    if !calls.is_empty() {
        events.push(Ok(ProviderEvent::ToolCalls(calls)));
    }
    events.push(Ok(ProviderEvent::Done));
    events
}

fn turn_with_usage(content: &str, calls: Vec<ToolCall>, usage: TokenUsage) -> Turn {
    let mut events = turn(content, calls);
    events.insert(events.len() - 1, Ok(ProviderEvent::Usage(usage)));
    events
}

fn answering_limits(
    requests: Option<u64>,
    tools: Option<u64>,
    tokens: Option<u64>,
) -> AnsweringLimits {
    AnsweringLimits {
        max_requests: requests,
        max_tool_calls: tools,
        max_reported_tokens: tokens,
    }
}

struct RunSettings {
    definitions: Vec<ToolDefinition>,
    limits: AgentLimits,
    answering: AnsweringLimits,
}

async fn controlled_run(
    provider: &dyn ChatProvider,
    executor: &dyn ToolExecutor,
    settings: RunSettings,
    approval: &dyn saya_agent::ApprovalDecider,
    sink: &RecordingSink,
    cancellation: CancellationToken,
) -> Result<saya_agent::AgentOutput, AgentError> {
    run_agent_with_control(
        provider,
        executor,
        request(),
        settings.definitions,
        AgentRunControl::new(
            settings.limits,
            settings.answering,
            approval,
            sink,
            cancellation,
        ),
    )
    .await
}

fn stopped(output: &saya_agent::AgentOutput, dimension: BudgetDimension) -> bool {
    output.events.iter().any(|event| {
        matches!(event, AgentEvent::AnsweringBudgetStopped { dimension: seen } if *seen == dimension)
    })
}

#[path = "answering_limits/harness.rs"]
mod harness;

pub(crate) use harness::*;

#[path = "answering_limits/request_admission.rs"]
mod request_admission;
#[path = "answering_limits/shared_scope.rs"]
mod shared_scope;
#[path = "answering_limits/stream_item_limit.rs"]
mod stream_item_limit;
#[path = "answering_limits/tool_admission.rs"]
mod tool_admission;
#[path = "answering_limits/usage_cancellation.rs"]
mod usage_cancellation;
