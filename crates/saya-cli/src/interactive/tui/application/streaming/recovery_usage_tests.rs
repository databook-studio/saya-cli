//! The producer-to-TUI accounting lifecycle for retried provider attempts.

use super::*;
use async_trait::async_trait;
use saya_agent::{
    AgentEventSink, AgentLimits, AgentRequest, AllowReadOnlyApproval, ChatProvider, ChatRequest,
    ChatResponse, ProviderError, ProviderEvent, ProviderStream, ToolError, ToolExecutor,
    run_agent_with_sink,
};
use std::sync::Mutex;

struct NoTools;

#[async_trait]
impl ToolExecutor for NoTools {
    async fn execute(&self, _: &str, _: serde_json::Value) -> Result<serde_json::Value, ToolError> {
        Ok(serde_json::Value::Null)
    }
}

struct RecordingSink(Mutex<Vec<AgentEvent>>);

#[async_trait]
impl AgentEventSink for RecordingSink {
    async fn emit(&self, event: AgentEvent) {
        self.0.lock().unwrap().push(event);
    }
}

struct Attempts(Mutex<Vec<Vec<ProviderEvent>>>);

#[async_trait]
impl ChatProvider for Attempts {
    fn name(&self) -> &str {
        "deterministic-attempts"
    }

    async fn complete(&self, _: ChatRequest) -> Result<ChatResponse, ProviderError> {
        unreachable!("the agent receive path streams")
    }

    async fn stream(
        &self,
        _: ChatRequest,
        _: CancellationToken,
    ) -> Result<ProviderStream, ProviderError> {
        let events = self.0.lock().unwrap().remove(0);
        Ok(Box::pin(futures_util::stream::iter(
            events.into_iter().map(Ok),
        )))
    }
}

struct EstablishmentFailure;

#[async_trait]
impl ChatProvider for EstablishmentFailure {
    fn name(&self) -> &str {
        "establishment-failure"
    }

    async fn complete(&self, _: ChatRequest) -> Result<ChatResponse, ProviderError> {
        unreachable!("the agent receive path streams")
    }

    async fn stream(
        &self,
        _: ChatRequest,
        _: CancellationToken,
    ) -> Result<ProviderStream, ProviderError> {
        Err(ProviderError::Request("refused".into()))
    }
}

fn request() -> AgentRequest {
    AgentRequest {
        prompt: "answer".into(),
        profile_names: Vec::new(),
        model: "test".into(),
        system_prompt: None,
        history: Vec::new(),
        context_blocks: Vec::new(),
    }
}

async fn run(provider: &dyn ChatProvider) -> (Vec<AgentEvent>, Result<AgentOutput, String>) {
    let sink = RecordingSink(Mutex::new(Vec::new()));
    let result = run_agent_with_sink(
        provider,
        &NoTools,
        request(),
        Vec::new(),
        AgentLimits::default(),
        &AllowReadOnlyApproval,
        &sink,
        CancellationToken::new(),
    )
    .await
    .map_err(|error| error.to_string());
    (sink.0.into_inner().unwrap(), result)
}

fn stream(events: Vec<AgentEvent>, result: Result<AgentOutput, String>) -> Stream {
    let (tx, rx) = unbounded_channel();
    for event in events {
        tx.send(StreamMsg::Event(event)).unwrap();
    }
    tx.send(StreamMsg::Done(result)).unwrap();
    Stream {
        rx,
        cancel: CancellationToken::new(),
        prompt: "question".into(),
    }
}

#[tokio::test]
async fn real_retry_events_settle_aggregate_spend_but_keep_successful_context_input() {
    let provider = Attempts(Mutex::new(vec![
        vec![
            ProviderEvent::TextDelta("discarded".into()),
            ProviderEvent::Usage(TokenUsage::new(80_000, 8)),
        ],
        vec![
            ProviderEvent::TextDelta("kept".into()),
            ProviderEvent::Usage(TokenUsage::new(64_000, 6)),
            ProviderEvent::Done,
        ],
    ]));
    let (events, result) = run(&provider).await;
    let output = result.as_ref().expect("the second attempt answers");
    assert_eq!(output.usage, TokenUsage::new(144_000, 14));
    assert!(events.windows(2).any(|pair| matches!(
        pair,
        [AgentEvent::TurnReset, AgentEvent::ProviderRecovery { .. }]
    )));

    let (mut app, mut state) = (idle_app(), SessionState::new("s1", None, "gpt-4o"));
    app.request.stream = Some(stream(events, result));
    assert!(app.drain_stream(&mut state));
    assert_eq!(state.usage.answering.input_tokens, 144_000);
    assert_eq!(state.usage.answering.output_tokens, 14);
    assert_eq!(state.usage.answering.turns, 1, "the aggregate settles once");
    let footer = app
        .transcript
        .blocks()
        .iter()
        .rev()
        .find(|block| block.text.contains("tokens in"))
        .expect("a successful run has a footer");
    assert!(footer.text.contains("144000 tokens in · 14 tokens out"));
    assert!(footer.text.contains("ctx 50% of 128k"), "{}", footer.text);
    assert!(app.request.known_answering_usage.is_none());
}

#[tokio::test]
async fn real_terminal_failure_keeps_known_spend_but_unknown_next_request_adds_nothing() {
    let provider = Attempts(Mutex::new(vec![
        vec![ProviderEvent::Usage(TokenUsage::new(17, 3))],
        vec![ProviderEvent::Usage(TokenUsage::new(17, 3))],
        vec![ProviderEvent::Usage(TokenUsage::new(17, 3))],
        vec![ProviderEvent::Usage(TokenUsage::new(17, 3))],
    ]));
    let (events, result) = run(&provider).await;
    assert!(result.is_err(), "all attempts ended without Done");
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, AgentEvent::FailedAttemptUsage { .. }))
            .count(),
        4
    );
    let (mut app, mut state) = (idle_app(), SessionState::new("s1", None, "gpt-4o"));
    app.request.stream = Some(stream(events, result));
    assert!(app.drain_stream(&mut state));
    assert_eq!(state.usage.answering.input_tokens, 68);
    assert_eq!(state.usage.answering.output_tokens, 12);
    assert_eq!(state.usage.answering.turns, 1, "one terminal receipt fold");
    assert!(app.request.known_answering_usage.is_none());

    let (events, result) = run(&EstablishmentFailure).await;
    assert!(
        events
            .iter()
            .all(|event| !matches!(event, AgentEvent::FailedAttemptUsage { .. }))
    );
    app.request.stream = Some(stream(events, result));
    assert!(app.drain_stream(&mut state));
    assert_eq!(
        state.usage.answering.input_tokens, 68,
        "unknown is not fake zero"
    );
    assert_eq!(
        state.usage.answering.turns, 1,
        "a fresh request cannot reuse receipts"
    );
}
