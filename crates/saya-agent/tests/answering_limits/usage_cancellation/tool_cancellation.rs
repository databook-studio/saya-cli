use super::*;

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
