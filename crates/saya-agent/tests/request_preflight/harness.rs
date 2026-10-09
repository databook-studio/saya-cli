use async_trait::async_trait;
use saya_agent::{
    AgentEvent, AgentEventSink, AgentMetric, AnsweringObservation, ApprovalDecider,
    CancellationToken, ChatProvider, ChatRequest, ChatResponse, ProviderError, ProviderEvent,
    ProviderStream, ToolDefinition, ToolError, ToolExecutionContext, ToolExecutor,
};
use std::{
    collections::VecDeque,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};

pub(super) type Turn = Vec<Result<ProviderEvent, ProviderError>>;

pub(super) struct ScriptedProvider {
    turns: Mutex<VecDeque<Turn>>,
    pub(super) requests: Arc<Mutex<Vec<ChatRequest>>>,
}

impl ScriptedProvider {
    pub(super) fn new(turns: Vec<Turn>) -> (Self, Arc<Mutex<Vec<ChatRequest>>>) {
        let requests = Arc::new(Mutex::new(Vec::new()));
        (
            Self {
                turns: Mutex::new(turns.into()),
                requests: requests.clone(),
            },
            requests,
        )
    }
}

#[async_trait]
impl ChatProvider for ScriptedProvider {
    fn name(&self) -> &str {
        "request-preflight-test"
    }

    async fn complete(&self, _: ChatRequest) -> Result<ChatResponse, ProviderError> {
        unreachable!("the agent loop receives through stream")
    }

    async fn stream(
        &self,
        request: ChatRequest,
        _: CancellationToken,
    ) -> Result<ProviderStream, ProviderError> {
        self.requests.lock().unwrap().push(request);
        let turn = self
            .turns
            .lock()
            .unwrap()
            .pop_front()
            .expect("scripted turn");
        Ok(Box::pin(futures_util::stream::iter(turn)))
    }
}

#[derive(Default)]
pub(super) struct RecordingExecutor {
    pub(super) calls: AtomicUsize,
    pub(super) caps: Mutex<Vec<usize>>,
    result: Mutex<serde_json::Value>,
}

impl RecordingExecutor {
    pub(super) fn with_result(result: serde_json::Value) -> Self {
        Self {
            result: Mutex::new(result),
            ..Self::default()
        }
    }
}

#[async_trait]
impl ToolExecutor for RecordingExecutor {
    async fn execute(&self, _: &str, _: serde_json::Value) -> Result<serde_json::Value, ToolError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(self.result.lock().unwrap().clone())
    }

    async fn execute_with_context(
        &self,
        name: &str,
        arguments: serde_json::Value,
        context: ToolExecutionContext,
    ) -> Result<serde_json::Value, ToolError> {
        self.caps.lock().unwrap().push(context.result_cap);
        self.execute(name, arguments).await
    }
}

#[derive(Default)]
pub(super) struct RecordingApproval(pub(super) AtomicUsize);

#[async_trait]
impl ApprovalDecider for RecordingApproval {
    async fn approve(&self, _: &ToolDefinition, _: &serde_json::Value) -> bool {
        self.0.fetch_add(1, Ordering::SeqCst);
        true
    }
}

#[derive(Default)]
pub(super) struct RecordingSink {
    events: Mutex<Vec<AgentEvent>>,
    pub(super) observations: Mutex<Vec<AnsweringObservation>>,
    pub(super) metrics: Mutex<Vec<AgentMetric>>,
}

#[async_trait]
impl AgentEventSink for RecordingSink {
    async fn emit(&self, event: AgentEvent) {
        self.events.lock().unwrap().push(event);
    }
    async fn observe_answering(&self, observation: AnsweringObservation) {
        self.observations.lock().unwrap().push(observation);
    }
    async fn metric(&self, metric: AgentMetric) {
        self.metrics.lock().unwrap().push(metric);
    }
}
