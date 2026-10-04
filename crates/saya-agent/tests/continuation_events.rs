//! C4-1: the typed provider-continuation transition on the real event stream.
//! `Continuing` is emitted once per admitted continuation — after the request
//! preflight and admission accepted the re-ask; `Exhausted` only when the run
//! stops because the continuation allowance is spent — never on a disabled
//! continuation, a preflight refusal, a context-window budget stop, or a
//! turn-ceiling stop.

use async_trait::async_trait;
use saya_agent::{
    AgentError, AgentEvent, AgentEventSink, AgentLimits, AgentMetric, AgentMetricCount,
    AgentRequest, AgentRunControl, AllowReadOnlyApproval, AnsweringLimits, BudgetDimension,
    CancellationToken, ChatProvider, ChatRequest, ChatResponse, LocalStateEffect,
    ProviderContinuationPhase, ProviderError, ProviderEvent, ProviderStream,
    RequestPreflightConfig, ToolConcurrency, ToolDefinition, ToolEffect, ToolError, ToolExecutor,
    run_agent_with_control, run_agent_with_sink,
};
use std::{
    num::NonZeroU64,
    sync::{Arc, Mutex},
};

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

/// (a) An output cap that is admitted as a continuation and then answers
/// naturally: exactly one `Continuing { index: 1, limit: 3 }` on the real
/// stream, and the run is NOT truncated — the transition is visible although
/// `truncated` is false.
#[tokio::test]
async fn admitted_continuation_emits_one_continuing_and_the_run_finishes_untruncated() {
    let (provider, requests) = continuation_script(false);
    let sink = RecordingSink::default();
    let output = run_agent_with_sink(
        &provider,
        &NoTools,
        request(),
        definitions(),
        AgentLimits::default(),
        &AllowReadOnlyApproval,
        &sink,
        CancellationToken::new(),
    )
    .await
    .expect("the run recovers after one truncation");
    assert_eq!(
        requests.lock().unwrap().len(),
        2,
        "one truncated turn plus one cooperative turn"
    );
    assert!(
        !output.truncated,
        "the natural answer must not mark the run truncated"
    );
    assert_eq!(
        sink.continuation_events(),
        vec![(ProviderContinuationPhase::Continuing, 1, 3)],
        "exactly one Continuing at the admitted continuation: {:?}",
        output.events
    );
    assert_eq!(
        sink.metric_count(&AgentMetric::Count(AgentMetricCount::AdmittedContinuation)),
        1,
        "the event shares the admitted-continuation point"
    );
}

/// (b) With the allowance spent the run still returns the original truncation
/// error, and the stream carries one `Exhausted { index: 3, limit: 3 }` after
/// exactly the three admitted `Continuing` events — never a fourth `Continuing`.
#[tokio::test]
async fn spent_allowance_emits_exhausted_after_exactly_three_continuings() {
    let (provider, requests) = continuation_script(true);
    let sink = RecordingSink::default();
    let error = run_agent_with_sink(
        &provider,
        &NoTools,
        request(),
        definitions(),
        AgentLimits {
            max_continuations: Some(3),
            ..AgentLimits::default()
        },
        &AllowReadOnlyApproval,
        &sink,
        CancellationToken::new(),
    )
    .await
    .unwrap_err();
    assert_eq!(
        requests.lock().unwrap().len(),
        4,
        "one initial turn plus three continuations"
    );
    assert!(
        matches!(
            error,
            AgentError::Provider(ProviderError::OutputTruncated { .. })
        ),
        "the original truncation error still surfaces: {error:?}"
    );
    assert_eq!(
        sink.continuation_events(),
        vec![
            (ProviderContinuationPhase::Continuing, 1, 3),
            (ProviderContinuationPhase::Continuing, 2, 3),
            (ProviderContinuationPhase::Continuing, 3, 3),
            (ProviderContinuationPhase::Exhausted, 3, 3),
        ],
        "the spent allowance ends the run with one Exhausted: {:?}",
        sink.events.lock().unwrap()
    );
}

/// (c) A continuation the request preflight refuses (the follow_ons fixture:
/// the re-ask no longer fits the known window) never reaches admission, so no
/// `Continuing` is emitted — and no `Exhausted` either: the run stops on the
/// typed context-window budget, not on the continuation limit.
#[tokio::test]
async fn preflight_refused_continuation_emits_no_continuation_transition() {
    let (provider, requests) = continuation_script(false);
    let sink = RecordingSink::default();
    let prompt_request = AgentRequest {
        prompt: "same user question".into(),
        profile_names: vec!["analytics".into()],
        model: "preflight-model".into(),
        system_prompt: None,
        history: Vec::new(),
        context_blocks: Vec::new(),
    };
    let control = AgentRunControl::new(
        AgentLimits::default(),
        AnsweringLimits::default(),
        &AllowReadOnlyApproval,
        &sink,
        CancellationToken::new(),
    )
    .with_request_preflight(RequestPreflightConfig::new(
        Some(NonZeroU64::new(1024 + 64 + 400).unwrap()),
        NonZeroU64::new(64).unwrap(),
    ));
    let output = run_agent_with_control(&provider, &NoTools, prompt_request, Vec::new(), control)
        .await
        .expect("the preflight stop is a bounded outcome, not an error");
    assert_eq!(
        requests.lock().unwrap().len(),
        1,
        "the re-ask was refused before reaching the provider"
    );
    assert_eq!(
        sink.metric_count(&AgentMetric::Count(AgentMetricCount::OutputTruncated)),
        1,
        "the fixture is live: the first turn really was truncated"
    );
    assert_eq!(
        sink.metric_count(&AgentMetric::Count(AgentMetricCount::AdmittedContinuation)),
        0,
        "a preflight-refused continuation is not admitted"
    );
    assert!(
        sink.continuation_events().is_empty(),
        "no continuation transition may be emitted for a refused re-ask: {:?}",
        sink.events.lock().unwrap()
    );
    assert!(
        output.events.iter().any(|event| matches!(
            event,
            AgentEvent::AnsweringBudgetStopped {
                dimension: BudgetDimension::RequestContextWindow
            }
        )),
        "the run stopped on the context-window budget: {:?}",
        output.events
    );
}

/// (d) A typed stop that owns termination while a continuation is pending —
/// the turn-ceiling salvage (mirroring `max_turns_ceiling_wins_over_continuations`)
/// — emits no `Exhausted`: the run did not stop because the allowance ran out.
#[tokio::test]
async fn turn_ceiling_stop_with_pending_continuation_emits_no_exhausted() {
    let (provider, requests) = continuation_script(true);
    let sink = RecordingSink::default();
    let output = run_agent_with_sink(
        &provider,
        &NoTools,
        request(),
        definitions(),
        AgentLimits {
            max_turns: Some(2),
            ..AgentLimits::default()
        },
        &AllowReadOnlyApproval,
        &sink,
        CancellationToken::new(),
    )
    .await
    .expect("the turn ceiling salvages instead of erroring");
    assert!(
        output.truncated,
        "the run stopped at the turn ceiling through salvage"
    );
    assert_eq!(
        requests.lock().unwrap().len(),
        3,
        "two ceiling turns plus the salvage call"
    );
    assert_eq!(
        sink.continuation_events(),
        vec![(ProviderContinuationPhase::Continuing, 1, 3)],
        "the one admitted continuation is visible and no Exhausted is emitted: {:?}",
        sink.events.lock().unwrap()
    );
}

/// (e) A disabled continuation (`None` and `Some(0)`) surfaces the truncation
/// immediately and emits neither transition: no continuation ever ran, so no
/// allowance was spent — `Exhausted` is reserved for a finite limit that ran
/// out.
#[tokio::test]
async fn disabled_continuation_emits_neither_transition() {
    for max_continuations in [None, Some(0)] {
        let (provider, requests) = continuation_script(true);
        let sink = RecordingSink::default();
        let error = run_agent_with_sink(
            &provider,
            &NoTools,
            request(),
            definitions(),
            AgentLimits {
                max_continuations,
                ..AgentLimits::default()
            },
            &AllowReadOnlyApproval,
            &sink,
            CancellationToken::new(),
        )
        .await
        .unwrap_err();
        assert_eq!(
            requests.lock().unwrap().len(),
            1,
            "max_continuations={max_continuations:?}: the truncation surfaces immediately"
        );
        assert!(
            matches!(
                error,
                AgentError::Provider(ProviderError::OutputTruncated { .. })
            ),
            "max_continuations={max_continuations:?}: the error surfaces: {error:?}"
        );
        assert!(
            sink.continuation_events().is_empty(),
            "max_continuations={max_continuations:?}: neither transition may be emitted: {:?}",
            sink.events.lock().unwrap()
        );
    }
}
