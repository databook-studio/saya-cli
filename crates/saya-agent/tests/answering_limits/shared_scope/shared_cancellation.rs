use super::*;

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
