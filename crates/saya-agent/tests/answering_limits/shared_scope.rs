use super::*;
use saya_agent::{AnsweringScope, run_agent_with_control};

async fn run_in_scope(
    provider: &dyn ChatProvider,
    executor: &dyn ToolExecutor,
    sink: &RecordingSink,
    scope: &AnsweringScope,
    answering: AnsweringLimits,
) -> Result<saya_agent::AgentOutput, AgentError> {
    run_in_scope_with_token(
        provider,
        executor,
        sink,
        scope,
        answering,
        scope.cancellation_token(),
    )
    .await
}

async fn run_in_scope_with_token(
    provider: &dyn ChatProvider,
    executor: &dyn ToolExecutor,
    sink: &RecordingSink,
    scope: &AnsweringScope,
    answering: AnsweringLimits,
    control_token: CancellationToken,
) -> Result<saya_agent::AgentOutput, AgentError> {
    run_agent_with_control(
        provider,
        executor,
        request(),
        definitions(),
        AgentRunControl::new(
            AgentLimits::default(),
            answering,
            &AllowReadOnlyApproval,
            sink,
            control_token,
        )
        .with_answering_scope(scope.clone()),
    )
    .await
}

fn request_count(observations: &[AnsweringObservation]) -> usize {
    observations
        .iter()
        .filter(|observation| matches!(observation, AnsweringObservation::RequestAdmitted))
        .count()
}

fn batch_counts(observations: &[AnsweringObservation]) -> Vec<u64> {
    observations
        .iter()
        .filter_map(|observation| match observation {
            AnsweringObservation::ToolBatchObserved { requested } => Some(*requested),
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn shared_scope_request_limit_stops_second_invocation_before_provider() {
    let cancellation = CancellationToken::new();
    let scope = AnsweringScope::new(answering_limits(Some(1), None, None), cancellation.clone());
    let sink = RecordingSink::new(cancellation);
    let first = ScriptedProvider::new(vec![turn("first answer", Vec::new())]).0;
    let first_output = run_in_scope(
        &first,
        &RecordingExecutor::default(),
        &sink,
        &scope,
        answering_limits(Some(50), Some(50), Some(50)),
    )
    .await
    .expect("the first admitted invocation answers");

    let (second, second_requests) = ScriptedProvider::new(vec![turn("must not run", Vec::new())]);
    let stopped_output = run_in_scope(
        &second,
        &RecordingExecutor::default(),
        &sink,
        &scope,
        answering_limits(Some(50), Some(50), Some(50)),
    )
    .await
    .expect("the scope returns a typed pre-work budget refusal");

    assert_eq!(first_output.answer, "first answer");
    assert!(stopped(&stopped_output, BudgetDimension::LogicalRequests));
    assert!(stopped_output.answer.is_empty());
    assert!(stopped_output.truncated);
    assert!(second_requests.lock().unwrap().is_empty());
    let observations = sink.observations.lock().unwrap();
    assert_eq!(request_count(&observations), 1);
    assert_eq!(
        observations
            .iter()
            .filter(|observation| matches!(observation, AnsweringObservation::Initialized))
            .count(),
        1,
        "reusing a scope must not reinitialize its spent counts"
    );
    assert_eq!(
        scope.cancellation_token().reason(),
        Some(CancellationReason::Budget(BudgetDimension::LogicalRequests))
    );
}

#[tokio::test]
async fn shared_scope_tool_limit_refuses_the_crossing_batch_before_effects() {
    let cancellation = CancellationToken::new();
    let scope = AnsweringScope::new(
        answering_limits(Some(8), Some(1), None),
        cancellation.clone(),
    );
    let sink = RecordingSink::new(cancellation);
    let executor = RecordingExecutor::default();
    let (first, _) = ScriptedProvider::new(vec![
        turn("", vec![query_call("first-query")]),
        turn("first attempt finished", Vec::new()),
    ]);
    run_in_scope(
        &first,
        &executor,
        &sink,
        &scope,
        answering_limits(Some(8), Some(8), None),
    )
    .await
    .expect("the first tool call fits the question scope");

    let (second, second_requests) =
        ScriptedProvider::new(vec![turn("second prose", vec![query_call("second-query")])]);
    let output = run_in_scope(
        &second,
        &executor,
        &sink,
        &scope,
        answering_limits(Some(8), Some(8), None),
    )
    .await
    .expect("the crossing batch becomes a typed partial result");

    assert_eq!(second_requests.lock().unwrap().len(), 1);
    assert!(stopped(&output, BudgetDimension::RequestedToolCalls));
    assert_eq!(
        executor.calls.lock().unwrap().as_slice(),
        ["query"],
        "the complete crossing batch has no prefix effect"
    );
    assert_eq!(
        batch_counts(&sink.observations.lock().unwrap()),
        [1, 0, 1],
        "complete source batches include the natural zero-call answer"
    );
}

#[tokio::test]
async fn shared_scope_failed_and_successful_receipts_reach_one_token_ceiling() {
    let cancellation = CancellationToken::new();
    let scope = AnsweringScope::new(
        answering_limits(Some(4), None, Some(8)),
        cancellation.clone(),
    );
    let sink = RecordingSink::new(cancellation.clone());
    let first = ScriptedProvider::new(vec![vec![
        Ok(ProviderEvent::Usage(TokenUsage::new(9, 0))),
        Ok(ProviderEvent::Usage(TokenUsage::new(2, 1))),
        Ok(ProviderEvent::Done),
    ]])
    .0;
    let error = run_in_scope(
        &first,
        &RecordingExecutor::default(),
        &sink,
        &scope,
        answering_limits(Some(4), None, Some(80)),
    )
    .await
    .expect_err("the first provider attempt ended without an answer");
    assert!(matches!(
        error,
        AgentError::Provider(ProviderError::InvalidResponse)
    ));

    let (second, second_requests) = ScriptedProvider::new(vec![turn_with_usage(
        "settled answer",
        Vec::new(),
        TokenUsage::new(3, 3),
    )]);
    let output = run_in_scope(
        &second,
        &RecordingExecutor::default(),
        &sink,
        &scope,
        answering_limits(Some(4), None, Some(80)),
    )
    .await
    .expect("the completed answer that reaches the cap is retained");

    assert_eq!(second_requests.lock().unwrap().len(), 1);
    assert_eq!(output.answer, "settled answer");
    assert_eq!(output.usage, TokenUsage::new(3, 3));
    assert!(
        !output.truncated,
        "a settled natural answer remains complete"
    );
    assert!(
        !output
            .events
            .iter()
            .any(|event| matches!(event, AgentEvent::AnsweringBudgetStopped { .. }))
    );
    assert_eq!(
        cancellation.reason(),
        Some(CancellationReason::Budget(BudgetDimension::ReportedTokens))
    );
    let failed_receipts: Vec<_> = sink
        .events
        .lock()
        .unwrap()
        .iter()
        .filter_map(|event| match event {
            AgentEvent::FailedAttemptUsage { usage } => Some(*usage),
            _ => None,
        })
        .collect();
    assert_eq!(failed_receipts, [TokenUsage::new(2, 1)]);

    let (third, third_requests) = ScriptedProvider::new(vec![turn("must not run", Vec::new())]);
    let error = run_in_scope(
        &third,
        &RecordingExecutor::default(),
        &sink,
        &scope,
        answering_limits(Some(99), None, Some(99)),
    )
    .await
    .expect_err("a stopped token scope cannot be widened by a later call");
    assert!(matches!(
        error,
        AgentError::Limit("answering budget stopped")
    ));
    assert!(third_requests.lock().unwrap().is_empty());
    assert_eq!(request_count(&sink.observations.lock().unwrap()), 2);
}

#[tokio::test]
async fn shared_scope_keeps_unknown_usage_distinct_from_reported_zero() {
    let cancellation = CancellationToken::new();
    let scope = AnsweringScope::new(
        answering_limits(Some(3), None, Some(0)),
        cancellation.clone(),
    );
    let sink = RecordingSink::new(cancellation);
    let unknown = ScriptedProvider::new(vec![turn("unknown receipt", Vec::new())]).0;
    run_in_scope(
        &unknown,
        &RecordingExecutor::default(),
        &sink,
        &scope,
        answering_limits(Some(3), None, None),
    )
    .await
    .expect("unknown usage does not consume a reported-token budget");
    assert!(!scope.cancellation_token().is_cancelled());

    let reported = ScriptedProvider::new(vec![turn_with_usage(
        "reported zero",
        Vec::new(),
        TokenUsage::default(),
    )])
    .0;
    let output = run_in_scope(
        &reported,
        &RecordingExecutor::default(),
        &sink,
        &scope,
        answering_limits(Some(3), None, None),
    )
    .await
    .expect("reported zero is a settled receipt under the explicit zero ceiling");
    assert_eq!(output.answer, "reported zero");
    assert_eq!(
        scope.cancellation_token().reason(),
        Some(CancellationReason::Budget(BudgetDimension::ReportedTokens))
    );

    let (later, later_requests) = ScriptedProvider::new(vec![turn("must not run", Vec::new())]);
    let error = run_in_scope(
        &later,
        &RecordingExecutor::default(),
        &sink,
        &scope,
        answering_limits(Some(99), None, None),
    )
    .await
    .expect_err("reported zero closes this question's token scope");
    assert!(matches!(
        error,
        AgentError::Limit("answering budget stopped")
    ));
    assert!(later_requests.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_new_scope_starts_a_distinct_question_budget() {
    let first_cancellation = CancellationToken::new();
    let first_scope = AnsweringScope::new(
        answering_limits(Some(1), None, None),
        first_cancellation.clone(),
    );
    let first_sink = RecordingSink::new(first_cancellation);
    let first = ScriptedProvider::new(vec![turn("first question", Vec::new())]).0;
    run_in_scope(
        &first,
        &RecordingExecutor::default(),
        &first_sink,
        &first_scope,
        answering_limits(Some(1), None, None),
    )
    .await
    .expect("first question completes");

    let second_cancellation = CancellationToken::new();
    let second_scope = AnsweringScope::new(
        answering_limits(Some(1), None, None),
        second_cancellation.clone(),
    );
    let second_sink = RecordingSink::new(second_cancellation);
    let second = ScriptedProvider::new(vec![turn("new question", Vec::new())]).0;
    let output = run_in_scope(
        &second,
        &RecordingExecutor::default(),
        &second_sink,
        &second_scope,
        answering_limits(Some(1), None, None),
    )
    .await
    .expect("a separately constructed scope is a fresh question");
    assert_eq!(output.answer, "new question");
}

#[tokio::test]
async fn pre_cancelled_fresh_scope_keeps_its_initialization_observation() {
    let cancellation = CancellationToken::new();
    cancellation.cancel();
    let scope = AnsweringScope::new(answering_limits(Some(1), None, None), cancellation.clone());
    let sink = RecordingSink::new(cancellation);
    let (provider, requests) = ScriptedProvider::new(vec![turn("must not run", Vec::new())]);

    let error = run_in_scope(
        &provider,
        &RecordingExecutor::default(),
        &sink,
        &scope,
        answering_limits(Some(1), None, None),
    )
    .await
    .expect_err("pre-cancelled controlled work remains refused");

    assert!(matches!(error, AgentError::Cancelled));
    assert!(requests.lock().unwrap().is_empty());
    assert_eq!(
        sink.observations.lock().unwrap().as_slice(),
        [AnsweringObservation::Initialized]
    );
}

struct WaitingExecutor {
    started: Mutex<Option<oneshot::Sender<()>>>,
    settled: AtomicBool,
    reason: Mutex<Option<CancellationReason>>,
}

#[async_trait]
impl ToolExecutor for WaitingExecutor {
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
        *self.reason.lock().unwrap() = context.cancellation.reason();
        self.settled.store(true, Ordering::SeqCst);
        Ok(serde_json::json!({"row_count": 1, "columns": ["n"]}))
    }
}

#[tokio::test]
async fn cancelled_scope_waiter_does_not_start_provider_work() {
    let (started, started_rx) = oneshot::channel();
    let executor = WaitingExecutor {
        started: Mutex::new(Some(started)),
        settled: AtomicBool::new(false),
        reason: Mutex::new(None),
    };
    let (provider, provider_requests) =
        ScriptedProvider::new(vec![turn("", vec![query_call("held-tool")])]);
    let cancellation = CancellationToken::new();
    let scope = AnsweringScope::new(answering_limits(Some(4), None, None), cancellation.clone());
    let sink = RecordingSink::new(cancellation.clone());
    let stale_token = CancellationToken::new();
    let first = run_in_scope_with_token(
        &provider,
        &executor,
        &sink,
        &scope,
        answering_limits(Some(4), None, None),
        stale_token.clone(),
    );
    tokio::pin!(first);
    tokio::select! {
        result = &mut first => panic!("the first tool is pending: {result:?}"),
        _ = started_rx => {}
    }

    let (waiting_provider, waiting_requests) =
        ScriptedProvider::new(vec![turn("must not run", Vec::new())]);
    let waiting = run_in_scope(
        &waiting_provider,
        &executor,
        &sink,
        &scope,
        answering_limits(Some(4), None, None),
    );
    tokio::pin!(waiting);
    tokio::select! {
        result = &mut waiting => panic!("the first invocation still owns the scope: {result:?}"),
        _ = tokio::task::yield_now() => {}
    }

    cancellation.cancel();
    let (active, waiter) = tokio::time::timeout(std::time::Duration::from_secs(1), async {
        tokio::join!(first, waiting)
    })
    .await
    .expect("the scope token settles both the active tool and scope waiter");
    assert!(matches!(active, Err(AgentError::Cancelled)));
    assert!(matches!(waiter, Err(AgentError::Cancelled)));
    assert!(executor.settled.load(Ordering::SeqCst));
    assert_eq!(
        *executor.reason.lock().unwrap(),
        Some(CancellationReason::User)
    );
    assert_eq!(
        stale_token.reason(),
        None,
        "the stale token is not the owner"
    );
    assert_eq!(provider_requests.lock().unwrap().len(), 1);
    assert!(waiting_requests.lock().unwrap().is_empty());
}
