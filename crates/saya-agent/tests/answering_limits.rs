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

#[tokio::test]
async fn one_request_ceiling_covers_ordinary_recovery_continuation_and_salvage() {
    struct Case {
        name: &'static str,
        attempts: Vec<Turn>,
        limits: AgentLimits,
        definitions: Vec<ToolDefinition>,
        expected_batches: Vec<u64>,
        expected_effects: usize,
    }

    let salvage_limits = AgentLimits {
        max_tool_calls: Some(0),
        ..AgentLimits::default()
    };
    let cases = vec![
        Case {
            name: "ordinary",
            attempts: vec![
                turn("", vec![query_call("q1")]),
                turn("must not be requested", Vec::new()),
            ],
            limits: AgentLimits::default(),
            definitions: definitions(),
            expected_batches: vec![1],
            expected_effects: 1,
        },
        Case {
            name: "continuation",
            attempts: vec![
                vec![Err(ProviderError::output_truncated(
                    "partial".into(),
                    Vec::new(),
                ))],
                turn("must not be requested", Vec::new()),
            ],
            limits: AgentLimits::default(),
            definitions: definitions(),
            expected_batches: Vec::new(),
            expected_effects: 0,
        },
        Case {
            name: "clarification recovery",
            attempts: vec![
                turn(
                    "",
                    vec![call("ask", "request_clarification", serde_json::json!({}))],
                ),
                turn("must not be requested", Vec::new()),
            ],
            limits: AgentLimits::default(),
            definitions: definitions(),
            expected_batches: vec![1],
            expected_effects: 0,
        },
        Case {
            name: "designation recovery",
            attempts: vec![turn(
                "",
                vec![call(
                    "designation",
                    "designate_answer",
                    serde_json::json!({"sql": ""}),
                )],
            )],
            limits: AgentLimits::default(),
            definitions: definitions(),
            expected_batches: vec![1],
            expected_effects: 0,
        },
        Case {
            name: "legacy tool-ceiling salvage",
            attempts: vec![
                turn("", vec![query_call("q1")]),
                turn("must not be requested", Vec::new()),
            ],
            limits: salvage_limits,
            definitions: definitions(),
            expected_batches: vec![1],
            expected_effects: 0,
        },
    ];

    for case in cases {
        let (provider, requests) = ScriptedProvider::new(case.attempts);
        let cancellation = CancellationToken::new();
        let sink = RecordingSink::new(cancellation.clone());
        let executor = RecordingExecutor::default();
        let output = controlled_run(
            &provider,
            &executor,
            RunSettings {
                definitions: case.definitions,
                limits: case.limits,
                answering: answering_limits(Some(1), None, None),
            },
            &AllowReadOnlyApproval,
            &sink,
            cancellation,
        )
        .await
        .unwrap_or_else(|error| panic!("{}: {error}", case.name));

        assert_eq!(requests.lock().unwrap().len(), 1, "{}", case.name);
        assert_eq!(
            executor.calls.lock().unwrap().len(),
            case.expected_effects,
            "{}",
            case.name
        );
        assert!(
            stopped(&output, BudgetDimension::LogicalRequests),
            "{}",
            case.name
        );
        let observations = sink.observations.lock().unwrap();
        assert_eq!(
            observations.first(),
            Some(&AnsweringObservation::Initialized)
        );
        assert_eq!(
            observations
                .iter()
                .filter_map(|observation| match observation {
                    AnsweringObservation::ToolBatchObserved { requested } => Some(*requested),
                    _ => None,
                })
                .collect::<Vec<_>>(),
            case.expected_batches,
            "{}",
            case.name
        );
        assert_eq!(
            observations
                .iter()
                .filter(|observation| matches!(observation, AnsweringObservation::RequestAdmitted))
                .count(),
            1,
            "{}",
            case.name
        );
    }
}

#[tokio::test]
async fn an_established_stream_retry_is_one_logical_request() {
    let (provider, requests) = ScriptedProvider::new(vec![
        vec![
            Ok(ProviderEvent::TextDelta("discarded".into())),
            Err(ProviderError::Request("stream interrupted".into())),
        ],
        turn("answered after retry", Vec::new()),
    ]);
    let cancellation = CancellationToken::new();
    let sink = RecordingSink::new(cancellation.clone());
    let output = controlled_run(
        &provider,
        &RecordingExecutor::default(),
        RunSettings {
            definitions: definitions(),
            limits: AgentLimits::default(),
            answering: answering_limits(Some(1), None, None),
        },
        &AllowReadOnlyApproval,
        &sink,
        cancellation,
    )
    .await
    .expect("the internal retry is part of one admitted request");

    assert_eq!(output.answer, "answered after retry");
    assert_eq!(requests.lock().unwrap().len(), 2, "both attempts ran");
    assert_eq!(
        sink.observations
            .lock()
            .unwrap()
            .iter()
            .filter(|observation| matches!(observation, AnsweringObservation::RequestAdmitted))
            .count(),
        1
    );
}

#[tokio::test]
async fn over_headroom_special_batches_are_refused_before_shortcuts() {
    for requested in [
        designation_call("designation"),
        call(
            "ask",
            "request_clarification",
            serde_json::json!({"question": "Which table?"}),
        ),
    ] {
        let name = requested.name.clone();
        let (provider, requests) =
            ScriptedProvider::new(vec![turn("partial answer", vec![requested])]);
        let cancellation = CancellationToken::new();
        let sink = RecordingSink::new(cancellation.clone());
        let executor = RecordingExecutor::default();
        let output = controlled_run(
            &provider,
            &executor,
            RunSettings {
                definitions: definitions(),
                limits: AgentLimits::default(),
                answering: answering_limits(Some(2), Some(0), None),
            },
            &AllowReadOnlyApproval,
            &sink,
            cancellation,
        )
        .await
        .expect("an over-headroom batch returns its bounded answer");

        assert_eq!(requests.lock().unwrap().len(), 1, "{name}");
        assert!(executor.calls.lock().unwrap().is_empty(), "{name}");
        assert!(output.truncated, "{name}");
        assert!(
            stopped(&output, BudgetDimension::RequestedToolCalls),
            "{name}"
        );
        assert!(
            !output.events.iter().any(|event| matches!(
                event,
                AgentEvent::AnswerDesignated { .. } | AgentEvent::ClarificationNeeded { .. }
            )),
            "shortcut ran before batch admission: {name}"
        );
        assert_eq!(
            sink.observations.lock().unwrap().last(),
            Some(&AnsweringObservation::ToolBatchObserved { requested: 1 }),
            "{name}"
        );
    }
}

#[tokio::test]
async fn invalid_whole_batch_is_observed_before_protocol_refusal_without_effects() {
    let (provider, requests) = ScriptedProvider::new(vec![turn(
        "",
        vec![query_call("valid-prefix"), query_call(" ")],
    )]);
    let cancellation = CancellationToken::new();
    let sink = RecordingSink::new(cancellation.clone());
    let executor = RecordingExecutor::default();
    let error = controlled_run(
        &provider,
        &executor,
        RunSettings {
            definitions: definitions(),
            limits: AgentLimits::default(),
            answering: answering_limits(Some(1), Some(0), None),
        },
        &AllowReadOnlyApproval,
        &sink,
        cancellation,
    )
    .await
    .expect_err("invalid IDs remain a terminal protocol refusal");

    assert!(matches!(error, AgentError::InvalidToolCall));
    assert_eq!(requests.lock().unwrap().len(), 1);
    assert!(executor.calls.lock().unwrap().is_empty());
    assert_eq!(
        sink.observations.lock().unwrap().last(),
        Some(&AnsweringObservation::ToolBatchObserved { requested: 2 })
    );
}

#[tokio::test]
async fn denied_calls_are_counted_and_a_complete_empty_batch_is_explicit() {
    let mut denied_query = definition("query");
    denied_query.effect.requires_approval = true;
    let (provider, requests) = ScriptedProvider::new(vec![
        turn("", vec![query_call("denied")]),
        turn("finished", Vec::new()),
    ]);
    let cancellation = CancellationToken::new();
    let sink = RecordingSink::new(cancellation.clone());
    let executor = RecordingExecutor::default();
    let output = controlled_run(
        &provider,
        &executor,
        RunSettings {
            definitions: vec![denied_query, definition("request_clarification")],
            limits: AgentLimits::default(),
            answering: answering_limits(Some(2), Some(1), None),
        },
        &DenyApproval,
        &sink,
        cancellation,
    )
    .await
    .expect("denial is fed back and the answer completes");

    assert_eq!(output.answer, "finished");
    assert_eq!(requests.lock().unwrap().len(), 2);
    assert!(executor.calls.lock().unwrap().is_empty());
    assert!(output.events.iter().any(|event| matches!(
        event,
        AgentEvent::ToolDenied { name, .. } if name == "query"
    )));
    assert_eq!(
        sink.observations
            .lock()
            .unwrap()
            .iter()
            .filter_map(|observation| match observation {
                AnsweringObservation::ToolBatchObserved { requested } => Some(*requested),
                _ => None,
            })
            .collect::<Vec<_>>(),
        vec![1, 0]
    );
}

#[tokio::test]
async fn latest_failed_and_successful_usage_share_the_known_token_ceiling() {
    let (provider, requests) = ScriptedProvider::new(vec![
        vec![
            Ok(ProviderEvent::Usage(TokenUsage::new(9, 0))),
            Ok(ProviderEvent::Usage(TokenUsage::new(2, 1))),
            Err(ProviderError::Request("stream interrupted".into())),
        ],
        turn_with_usage(
            "partial answer",
            vec![query_call("must-not-run")],
            TokenUsage::new(3, 2),
        ),
    ]);
    let cancellation = CancellationToken::new();
    let sink = RecordingSink::new(cancellation.clone());
    let executor = RecordingExecutor::default();
    let output = controlled_run(
        &provider,
        &executor,
        RunSettings {
            definitions: definitions(),
            limits: AgentLimits::default(),
            answering: answering_limits(Some(3), Some(5), Some(8)),
        },
        &AllowReadOnlyApproval,
        &sink,
        cancellation,
    )
    .await
    .expect("the settled answer is retained while its tool effects are stopped");

    assert_eq!(requests.lock().unwrap().len(), 2, "one retry, one request");
    assert_eq!(output.answer, "partial answer");
    assert!(output.truncated);
    assert!(stopped(&output, BudgetDimension::ReportedTokens));
    assert!(executor.calls.lock().unwrap().is_empty());
    assert_eq!(output.usage, TokenUsage::new(5, 3));
    assert_eq!(
        output
            .events
            .iter()
            .filter_map(|event| match event {
                AgentEvent::FailedAttemptUsage { usage } => Some(*usage),
                _ => None,
            })
            .collect::<Vec<_>>(),
        vec![TokenUsage::new(2, 1)]
    );
}

#[tokio::test]
async fn absent_usage_stays_unknown_while_reported_zero_closes_a_zero_ceiling() {
    let (reported_provider, reported_requests) = ScriptedProvider::new(vec![turn_with_usage(
        "",
        vec![query_call("zero")],
        TokenUsage::default(),
    )]);
    let cancellation = CancellationToken::new();
    let reported_sink = RecordingSink::new(cancellation.clone());
    let reported_executor = RecordingExecutor::default();
    let reported = controlled_run(
        &reported_provider,
        &reported_executor,
        RunSettings {
            definitions: definitions(),
            limits: AgentLimits::default(),
            answering: answering_limits(Some(2), Some(1), Some(0)),
        },
        &AllowReadOnlyApproval,
        &reported_sink,
        cancellation,
    )
    .await
    .expect("reported zero is a settled report");
    assert_eq!(reported_requests.lock().unwrap().len(), 1);
    assert!(reported_executor.calls.lock().unwrap().is_empty());
    assert!(stopped(&reported, BudgetDimension::ReportedTokens));
    assert!(
        reported
            .events
            .iter()
            .any(|event| matches!(event, AgentEvent::Usage { .. }))
    );

    let (unknown_provider, unknown_requests) = ScriptedProvider::new(vec![
        turn("", vec![query_call("unknown")]),
        turn("completed with unknown usage", Vec::new()),
    ]);
    let cancellation = CancellationToken::new();
    let unknown_sink = RecordingSink::new(cancellation.clone());
    let unknown_executor = RecordingExecutor::default();
    let unknown = controlled_run(
        &unknown_provider,
        &unknown_executor,
        RunSettings {
            definitions: definitions(),
            limits: AgentLimits::default(),
            answering: answering_limits(Some(2), Some(1), Some(0)),
        },
        &AllowReadOnlyApproval,
        &unknown_sink,
        cancellation,
    )
    .await
    .expect("unknown usage does not become a reported zero");
    assert_eq!(unknown.answer, "completed with unknown usage");
    assert_eq!(unknown_requests.lock().unwrap().len(), 2);
    assert_eq!(unknown_executor.calls.lock().unwrap().as_slice(), ["query"]);
    assert!(!unknown.events.iter().any(|event| matches!(
        event,
        AgentEvent::AnsweringBudgetStopped {
            dimension: BudgetDimension::ReportedTokens
        }
    )));
    assert!(
        !unknown
            .events
            .iter()
            .any(|event| matches!(event, AgentEvent::Usage { .. }))
    );
}

struct PendingProvider {
    started: Mutex<Option<oneshot::Sender<()>>>,
    calls: AtomicUsize,
}

#[async_trait]
impl ChatProvider for PendingProvider {
    fn name(&self) -> &str {
        "pending-provider"
    }

    async fn complete(&self, _: ChatRequest) -> Result<saya_agent::ChatResponse, ProviderError> {
        unreachable!("the agent loop receives through stream")
    }

    async fn stream(
        &self,
        _: ChatRequest,
        _: CancellationToken,
    ) -> Result<ProviderStream, ProviderError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if let Some(started) = self.started.lock().unwrap().take() {
            let _ = started.send(());
        }
        Ok(Box::pin(futures_util::stream::pending()))
    }
}

#[tokio::test]
async fn typed_elapsed_cancellation_interrupts_a_pending_provider_receive() {
    let (started, started_rx) = oneshot::channel();
    let provider = PendingProvider {
        started: Mutex::new(Some(started)),
        calls: AtomicUsize::new(0),
    };
    let cancellation = CancellationToken::new();
    let sink = RecordingSink::new(cancellation.clone());
    let executor = RecordingExecutor::default();
    let approval = AllowReadOnlyApproval;
    let run = controlled_run(
        &provider,
        &executor,
        RunSettings {
            definitions: definitions(),
            limits: AgentLimits::default(),
            answering: answering_limits(Some(2), None, None),
        },
        &approval,
        &sink,
        cancellation.clone(),
    );
    tokio::pin!(run);
    tokio::select! { _ = &mut run => panic!("pending provider returned"), _ = started_rx => {} }
    assert!(
        cancellation.cancel_with_reason(CancellationReason::Budget(BudgetDimension::ElapsedTime))
    );
    let output = tokio::time::timeout(std::time::Duration::from_secs(1), &mut run)
        .await
        .expect("receive observes cancellation")
        .expect("elapsed budget is a bounded output");

    assert!(output.truncated);
    assert!(stopped(&output, BudgetDimension::ElapsedTime));
    assert_eq!(
        cancellation.reason(),
        Some(CancellationReason::Budget(BudgetDimension::ElapsedTime))
    );
}

struct PendingExecutor {
    started: Mutex<Option<oneshot::Sender<()>>>,
    settled: Arc<AtomicBool>,
    seen_reason: Arc<Mutex<Option<CancellationReason>>>,
}

#[async_trait]
impl ToolExecutor for PendingExecutor {
    async fn execute(&self, _: &str, _: serde_json::Value) -> Result<serde_json::Value, ToolError> {
        Err(ToolError::QueryFailed)
    }

    async fn execute_with_context(
        &self,
        _: &str,
        _: serde_json::Value,
        context: ToolExecutionContext,
    ) -> Result<serde_json::Value, ToolError> {
        if let Some(started) = self.started.lock().unwrap().take() {
            let _ = started.send(());
        }
        context.cancellation.cancelled().await;
        *self.seen_reason.lock().unwrap() = context.cancellation.reason();
        self.settled.store(true, Ordering::SeqCst);
        Ok(serde_json::json!({"row_count": 0, "columns": []}))
    }
}

#[tokio::test]
async fn elapsed_stop_waits_for_the_context_aware_tool_to_settle() {
    let (started, started_rx) = oneshot::channel();
    let settled = Arc::new(AtomicBool::new(false));
    let seen_reason = Arc::new(Mutex::new(None));
    let executor = PendingExecutor {
        started: Mutex::new(Some(started)),
        settled: settled.clone(),
        seen_reason: seen_reason.clone(),
    };
    let (provider, requests) =
        ScriptedProvider::new(vec![turn("", vec![query_call("pending-tool")])]);
    let cancellation = CancellationToken::new();
    let sink = RecordingSink::new(cancellation.clone());
    let approval = AllowReadOnlyApproval;
    let run = controlled_run(
        &provider,
        &executor,
        RunSettings {
            definitions: definitions(),
            limits: AgentLimits::default(),
            answering: answering_limits(Some(2), Some(2), None),
        },
        &approval,
        &sink,
        cancellation.clone(),
    );
    tokio::pin!(run);
    tokio::select! { _ = &mut run => panic!("tool did not wait"), _ = started_rx => {} }
    cancellation.cancel_with_reason(CancellationReason::Budget(BudgetDimension::ElapsedTime));
    let output = tokio::time::timeout(std::time::Duration::from_secs(1), &mut run)
        .await
        .expect("tool settlement is awaited")
        .expect("elapsed budget returns a partial output");

    assert!(settled.load(Ordering::SeqCst));
    assert_eq!(
        *seen_reason.lock().unwrap(),
        Some(CancellationReason::Budget(BudgetDimension::ElapsedTime))
    );
    assert!(output.truncated);
    assert!(stopped(&output, BudgetDimension::ElapsedTime));
    assert_eq!(output.tool_metadata.len(), 1, "settled effect is retained");
    assert_eq!(requests.lock().unwrap().len(), 1, "no later receive starts");
}

#[tokio::test]
async fn failed_request_observation_prevents_provider_admission() {
    let (provider, requests) = ScriptedProvider::new(vec![turn("must not run", Vec::new())]);
    let cancellation = CancellationToken::new();
    let sink = RecordingSink::cancelling_on(cancellation.clone(), CancelOnObservation::Request);
    let error = controlled_run(
        &provider,
        &RecordingExecutor::default(),
        RunSettings {
            definitions: definitions(),
            limits: AgentLimits::default(),
            answering: answering_limits(Some(1), None, None),
        },
        &AllowReadOnlyApproval,
        &sink,
        cancellation.clone(),
    )
    .await
    .expect_err("owner observation failure fails closed");

    assert!(matches!(error, AgentError::Cancelled));
    assert_eq!(requests.lock().unwrap().len(), 0);
    assert_eq!(
        cancellation.reason(),
        Some(CancellationReason::OwnerFailure)
    );
}

#[tokio::test]
async fn failed_tool_batch_observation_prevents_shortcuts_and_effects() {
    let (provider, requests) = ScriptedProvider::new(vec![turn(
        "must not be trusted",
        vec![designation_call("designation"), query_call("query")],
    )]);
    let cancellation = CancellationToken::new();
    let sink = RecordingSink::cancelling_on(cancellation.clone(), CancelOnObservation::ToolBatch);
    let executor = RecordingExecutor::default();
    let error = controlled_run(
        &provider,
        &executor,
        RunSettings {
            definitions: definitions(),
            limits: AgentLimits::default(),
            answering: answering_limits(Some(2), None, None),
        },
        &AllowReadOnlyApproval,
        &sink,
        cancellation.clone(),
    )
    .await
    .expect_err("failed batch observation prevents all later handling");

    assert!(matches!(error, AgentError::Cancelled));
    assert_eq!(requests.lock().unwrap().len(), 1);
    assert_eq!(
        cancellation.reason(),
        Some(CancellationReason::OwnerFailure)
    );
    assert!(executor.calls.lock().unwrap().is_empty());
    assert!(
        !sink
            .events
            .lock()
            .unwrap()
            .iter()
            .any(|event| matches!(event, AgentEvent::AnswerDesignated { .. }))
    );
}

#[test]
fn cancellation_reason_is_first_writer_wins() {
    let token = CancellationToken::new();
    assert!(token.cancel_with_reason(CancellationReason::Budget(BudgetDimension::ReportedTokens,)));
    assert!(!token.cancel_with_reason(CancellationReason::User));
    assert_eq!(
        token.reason(),
        Some(CancellationReason::Budget(BudgetDimension::ReportedTokens))
    );
    assert!(token.is_cancelled());
}
