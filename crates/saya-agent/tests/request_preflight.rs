use async_trait::async_trait;
use saya_agent::{
    AgentEvent, AgentEventSink, AgentLimits, AgentMetric, AgentMetricCount, AgentRequest,
    AgentRunControl, AnsweringLimits, AnsweringObservation, ApprovalDecider, BudgetDimension,
    CancellationToken, ChatMessage, ChatProvider, ChatRequest, LocalStateEffect, ProviderError,
    ProviderEvent, ReasoningEffort, RequestPreflightConfig, ResponseFormat, TokenUsage, ToolCall,
    ToolConcurrency, ToolDefinition, ToolEffect, ToolExecutor, run_agent_with_control,
};
use serde::Serialize;
use std::{
    num::NonZeroU64,
    sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};

fn request(prompt: impl Into<String>, system_prompt: Option<String>) -> AgentRequest {
    AgentRequest {
        prompt: prompt.into(),
        profile_names: vec!["analytics".into()],
        model: "preflight-model".into(),
        system_prompt,
        history: Vec::new(),
        context_blocks: Vec::new(),
    }
}

fn definition(name: &str) -> ToolDefinition {
    ToolDefinition {
        name: name.into(),
        description: "read bounded data".into(),
        read_only: true,
        parameters: serde_json::json!({"type":"object","properties":{"sql":{"type":"string"}}}),
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

fn wide_definition(name: &str, bytes: usize) -> ToolDefinition {
    let mut value = definition(name);
    value.description = "schema".repeat(bytes / 6 + 1);
    value.parameters = serde_json::json!({"type":"object","description":"x".repeat(bytes)});
    value
}

fn call(id: impl Into<String>, name: &str, arguments: serde_json::Value) -> ToolCall {
    ToolCall {
        id: id.into(),
        name: name.into(),
        arguments,
    }
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

fn preflight(window: Option<u64>, output_reserve: u64) -> RequestPreflightConfig {
    RequestPreflightConfig::new(
        window.map(|value| NonZeroU64::new(value).unwrap()),
        NonZeroU64::new(output_reserve).unwrap(),
    )
}

fn control<'a>(
    limits: AgentLimits,
    approval: &'a dyn ApprovalDecider,
    sink: &'a dyn AgentEventSink,
    cancellation: CancellationToken,
    preflight: RequestPreflightConfig,
) -> AgentRunControl<'a> {
    AgentRunControl::new(
        limits,
        AnsweringLimits::default(),
        approval,
        sink,
        cancellation,
    )
    .with_request_preflight(preflight)
}

async fn run(
    provider: &dyn ChatProvider,
    executor: &dyn ToolExecutor,
    request: AgentRequest,
    definitions: Vec<ToolDefinition>,
    control: AgentRunControl<'_>,
) -> Result<saya_agent::AgentOutput, saya_agent::AgentError> {
    run_agent_with_control(provider, executor, request, definitions, control).await
}

fn stopped(output: &saya_agent::AgentOutput, dimension: BudgetDimension) -> bool {
    output.events.iter().any(|event| {
        matches!(
            event,
            AgentEvent::AnsweringBudgetStopped { dimension: seen } if *seen == dimension
        )
    })
}

fn provider_projection_bytes(request: &ChatRequest) -> usize {
    #[derive(Serialize)]
    struct Tool<'a> {
        name: &'a str,
        description: &'a str,
        parameters: &'a serde_json::Value,
    }
    #[derive(Serialize)]
    struct Projection<'a> {
        model: &'a str,
        messages: &'a [ChatMessage],
        tools: Vec<Tool<'a>>,
        response_format: ResponseFormat,
        reasoning_effort: ReasoningEffort,
    }
    let projection = Projection {
        model: &request.model,
        messages: &request.messages,
        tools: request
            .tools
            .iter()
            .map(|tool| Tool {
                name: &tool.name,
                description: &tool.description,
                parameters: &tool.parameters,
            })
            .collect(),
        response_format: request.response_format,
        reasoning_effort: request.reasoning_effort,
    };
    serde_json::to_vec(&projection).unwrap().len()
}

#[path = "request_preflight/harness.rs"]
mod harness;

pub(crate) use harness::*;

#[path = "request_preflight/basics.rs"]
mod basics;
#[path = "request_preflight/follow_ons.rs"]
mod follow_ons;
#[path = "request_preflight/history.rs"]
mod history;
#[path = "request_preflight/shortcuts.rs"]
mod shortcuts;
#[path = "request_preflight/tool_batches.rs"]
mod tool_batches;
