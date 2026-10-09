use async_trait::async_trait;
use saya_agent::{
    AgentEvent, AgentEventSink, AnsweringObservation, CancellationReason, CancellationToken,
    ChatProvider, ChatRequest, ProviderError, ProviderEvent, ProviderStream, ToolDefinition,
    ToolError, ToolExecutor,
};
use std::sync::{Arc, Mutex};

pub(super) type Turn = Vec<Result<ProviderEvent, ProviderError>>;

pub(super) struct ScriptedProvider {
    attempts: Mutex<Vec<Turn>>,
    requests: Arc<Mutex<Vec<ChatRequest>>>,
}

impl ScriptedProvider {
    pub(super) fn new(attempts: Vec<Turn>) -> (Self, Arc<Mutex<Vec<ChatRequest>>>) {
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
pub(super) enum CancelOnObservation {
    Request,
    ToolBatch,
}

pub(super) struct RecordingSink {
    pub(super) events: Mutex<Vec<AgentEvent>>,
    pub(super) observations: Mutex<Vec<AnsweringObservation>>,
    cancellation: CancellationToken,
    cancel_on: Option<CancelOnObservation>,
}

impl RecordingSink {
    pub(super) fn new(cancellation: CancellationToken) -> Self {
        Self {
            events: Mutex::new(Vec::new()),
            observations: Mutex::new(Vec::new()),
            cancellation,
            cancel_on: None,
        }
    }

    pub(super) fn cancelling_on(
        cancellation: CancellationToken,
        cancel_on: CancelOnObservation,
    ) -> Self {
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
pub(super) struct RecordingExecutor {
    pub(super) calls: Mutex<Vec<String>>,
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

pub(super) struct DenyApproval;

#[async_trait]
impl saya_agent::ApprovalDecider for DenyApproval {
    async fn approve(&self, _: &ToolDefinition, _: &serde_json::Value) -> bool {
        false
    }
}
