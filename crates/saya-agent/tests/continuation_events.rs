//! C4-1: the typed provider-continuation transition on the real event stream.
//! `Continuing` is emitted once per admitted continuation — after the request
//! preflight and admission accepted the re-ask; `Exhausted` only when the run
//! stops because the continuation allowance is spent — never on a disabled
//! continuation, a preflight refusal, a context-window budget stop, or a
//! turn-ceiling stop.

use async_trait::async_trait;
use saya_agent::{
    AgentEvent, AgentEventSink, AgentMetric, AgentRequest, CancellationToken, ChatProvider,
    ChatRequest, ChatResponse, LocalStateEffect, ProviderContinuationPhase, ProviderError,
    ProviderEvent, ProviderStream, ToolConcurrency, ToolDefinition, ToolEffect, ToolError,
    ToolExecutor,
};
use std::sync::{Arc, Mutex};

/// A scripted stream provider mirroring agent_loop.rs's `continuation_script`:
/// the first turn (or every turn when `always_truncate`) streams partial text
/// and caps mid-answer with `OutputTruncated`; later turns answer naturally.
struct ContinuationScriptProvider {
    requests: Arc<Mutex<Vec<ChatRequest>>>,
    calls: Mutex<usize>,
    always_truncate: bool,
    partial_text: String,
}

#[async_trait]
impl ChatProvider for ContinuationScriptProvider {
    fn name(&self) -> &str {
        "continuation-events-script"
    }
    async fn complete(&self, _: ChatRequest) -> Result<ChatResponse, ProviderError> {
        unreachable!("the main loop drives the provider through stream")
    }
    async fn stream(
        &self,
        request: ChatRequest,
        _: CancellationToken,
    ) -> Result<ProviderStream, ProviderError> {
        self.requests.lock().unwrap().push(request);
        let mut calls = self.calls.lock().unwrap();
        *calls += 1;
        let turn = *calls;
        drop(calls);
        if !self.always_truncate && turn > 1 {
            return Ok(Box::pin(futures_util::stream::iter(vec![
                Ok(ProviderEvent::TextDelta("cooperative answer".into())),
                Ok(ProviderEvent::Done),
            ])));
        }
        Ok(Box::pin(futures_util::stream::iter(vec![
            Ok(ProviderEvent::TextDelta(self.partial_text.clone())),
            Err(ProviderError::output_truncated(
                self.partial_text.clone(),
                Vec::new(),
            )),
        ])))
    }
}

fn continuation_script(
    always_truncate: bool,
) -> (ContinuationScriptProvider, Arc<Mutex<Vec<ChatRequest>>>) {
    let requests = Arc::new(Mutex::new(Vec::new()));
    (
        ContinuationScriptProvider {
            requests: requests.clone(),
            calls: Mutex::new(0),
            always_truncate,
            partial_text: "TRUNCATED-PROSE-C41".into(),
        },
        requests,
    )
}

struct NoTools;

#[async_trait]
impl ToolExecutor for NoTools {
    async fn execute(&self, _: &str, _: serde_json::Value) -> Result<serde_json::Value, ToolError> {
        Ok(serde_json::json!({"rows": 1}))
    }
}

#[derive(Default)]
struct RecordingSink {
    events: Mutex<Vec<AgentEvent>>,
    metrics: Mutex<Vec<AgentMetric>>,
}

#[async_trait]
impl AgentEventSink for RecordingSink {
    async fn emit(&self, event: AgentEvent) {
        self.events.lock().unwrap().push(event);
    }
    async fn metric(&self, metric: AgentMetric) {
        self.metrics.lock().unwrap().push(metric);
    }
}

impl RecordingSink {
    /// The real sink-observed continuation transitions in emission order.
    fn continuation_events(&self) -> Vec<(ProviderContinuationPhase, u32, u32)> {
        self.events
            .lock()
            .unwrap()
            .iter()
            .filter_map(|event| match event {
                AgentEvent::ProviderContinuation {
                    phase,
                    index,
                    limit,
                } => Some((*phase, *index, *limit)),
                _ => None,
            })
            .collect()
    }

    fn metric_count(&self, metric: &AgentMetric) -> usize {
        self.metrics
            .lock()
            .unwrap()
            .iter()
            .filter(|seen| *seen == metric)
            .count()
    }
}

fn request() -> AgentRequest {
    AgentRequest {
        prompt: "show data".into(),
        profile_names: vec!["analytics".into()],
        model: "mock-model".into(),
        system_prompt: None,
        history: Vec::new(),
        context_blocks: Vec::new(),
    }
}

fn definitions() -> Vec<ToolDefinition> {
    vec![ToolDefinition {
        name: "bounded_sql_query".into(),
        description: "read-only query".into(),
        read_only: true,
        parameters: serde_json::json!({"type":"object"}),
        effect: ToolEffect {
            database_data: true,
            external_side_effect: false,
            requires_approval: true,
            local_state: LocalStateEffect::None,
        },
        concurrency: ToolConcurrency::Serial,
        completion: None,
    }]
}

#[path = "continuation_events/admitted_transitions.rs"]
mod admitted_transitions;
#[path = "continuation_events/refused_transitions.rs"]
mod refused_transitions;
