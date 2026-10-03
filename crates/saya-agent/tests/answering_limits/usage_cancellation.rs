use super::*;

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
