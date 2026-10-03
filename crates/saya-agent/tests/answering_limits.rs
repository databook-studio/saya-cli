use async_trait::async_trait;
use saya_agent::{
    AgentError, AgentEvent, AgentEventSink, AgentLimits, AgentRequest, AgentRunControl,
    AllowReadOnlyApproval, AnsweringLimits, AnsweringObservation, BudgetDimension,
    CancellationReason, CancellationToken, ChatProvider, ChatRequest, LocalStateEffect,
    ProviderError, ProviderEvent, ProviderStream, TokenUsage, ToolCall, ToolConcurrency,
    ToolDefinition, ToolEffect, ToolError, ToolExecutionContext, ToolExecutor,
    run_agent_with_control,
};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};
use tokio::sync::oneshot;

type Turn = Vec<Result<ProviderEvent, ProviderError>>;

struct ScriptedProvider {
    attempts: Mutex<Vec<Turn>>,
    requests: Arc<Mutex<Vec<ChatRequest>>>,
}

impl ScriptedProvider {
    fn new(attempts: Vec<Turn>) -> (Self, Arc<Mutex<Vec<ChatRequest>>>) {
        let requests = Arc::new(Mutex::new(Vec::new()));
        (
            Self {
                attempts: Mutex::new(attempts),
                requests: requests.clone(),
            },
            requests,
        )
    }
}

#[async_trait]
impl ChatProvider for ScriptedProvider {
    fn name(&self) -> &str {
        "scripted-answering-limits"
    }

    async fn complete(&self, _: ChatRequest) -> Result<saya_agent::ChatResponse, ProviderError> {
        unreachable!("the agent loop receives through stream")
    }

    async fn stream(
        &self,
        request: ChatRequest,
        _: CancellationToken,
    ) -> Result<ProviderStream, ProviderError> {
        self.requests.lock().unwrap().push(request);
        let attempt = self.attempts.lock().unwrap().remove(0);
        Ok(Box::pin(futures_util::stream::iter(attempt)))
    }
}

#[derive(Clone, Copy)]
enum CancelOnObservation {
    Request,
    ToolBatch,
}

struct RecordingSink {
    events: Mutex<Vec<AgentEvent>>,
    observations: Mutex<Vec<AnsweringObservation>>,
    cancellation: CancellationToken,
    cancel_on: Option<CancelOnObservation>,
}

impl RecordingSink {
    fn new(cancellation: CancellationToken) -> Self {
        Self {
            events: Mutex::new(Vec::new()),
            observations: Mutex::new(Vec::new()),
            cancellation,
            cancel_on: None,
        }
    }

    fn cancelling_on(cancellation: CancellationToken, cancel_on: CancelOnObservation) -> Self {
        Self {
            cancel_on: Some(cancel_on),
            ..Self::new(cancellation)
        }
    }
}

#[async_trait]
impl AgentEventSink for RecordingSink {
    async fn emit(&self, event: AgentEvent) {
        self.events.lock().unwrap().push(event);
    }

    async fn observe_answering(&self, observation: AnsweringObservation) {
        self.observations.lock().unwrap().push(observation);
        let should_cancel = matches!(
            (self.cancel_on, observation),
            (
                Some(CancelOnObservation::Request),
                AnsweringObservation::RequestAdmitted
            ) | (
                Some(CancelOnObservation::ToolBatch),
                AnsweringObservation::ToolBatchObserved { .. }
            )
        );
        if should_cancel {
            self.cancellation
                .cancel_with_reason(CancellationReason::OwnerFailure);
        }
    }
}

#[derive(Default)]
struct RecordingExecutor {
    calls: Mutex<Vec<String>>,
}

#[async_trait]
impl ToolExecutor for RecordingExecutor {
    async fn execute(
        &self,
        name: &str,
        _: serde_json::Value,
    ) -> Result<serde_json::Value, ToolError> {
        self.calls.lock().unwrap().push(name.into());
        Ok(serde_json::json!({"row_count": 1, "columns": ["n"]}))
    }
}

struct DenyApproval;

#[async_trait]
impl saya_agent::ApprovalDecider for DenyApproval {
    async fn approve(&self, _: &ToolDefinition, _: &serde_json::Value) -> bool {
        false
    }
}

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

#[path = "answering_limits/request_admission.rs"]
mod request_admission;
#[path = "answering_limits/tool_admission.rs"]
mod tool_admission;
#[path = "answering_limits/usage_cancellation.rs"]
mod usage_cancellation;
