use super::*;

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
