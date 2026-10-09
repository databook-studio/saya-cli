use super::*;

#[tokio::test]
async fn admitted_continuation_counts_once_and_metric_cancellation_prevents_source_entry() {
    let limits = AgentLimits {
        max_continuations: Some(1),
        context_byte_budget: 16 * 1024,
        ..AgentLimits::default()
    };
    let turns = vec![
        vec![Err(ProviderError::output_truncated(
            "partial answer".into(),
            Vec::new(),
        ))],
        turn("complete answer", Vec::new()),
    ];
    let (provider, requests) = ScriptedProvider::new(turns);
    let sink = RecordingSink::default();
    run(
        &provider,
        &RecordingExecutor::default(),
        request("same user question", None),
        Vec::new(),
        control(
            limits,
            &RecordingApproval::default(),
            &sink,
            CancellationToken::new(),
            preflight(Some(100_000), 64),
        ),
    )
    .await
    .unwrap();
    assert_eq!(requests.lock().unwrap().len(), 2);
    assert_eq!(
        sink.metrics
            .lock()
            .unwrap()
            .iter()
            .filter(|metric| **metric == AgentMetric::Count(AgentMetricCount::AdmittedContinuation))
            .count(),
        1
    );
    assert_eq!(
        sink.metrics
            .lock()
            .unwrap()
            .iter()
            .filter(|metric| **metric == AgentMetric::Count(AgentMetricCount::OutputTruncated))
            .count(),
        1
    );

    struct HeldMetric {
        entered: Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
        release: Mutex<Option<tokio::sync::oneshot::Receiver<()>>>,
        cancellation: CancellationToken,
        count: AtomicUsize,
        turn_started: AtomicUsize,
    }

    #[async_trait]
    impl AgentEventSink for HeldMetric {
        async fn emit(&self, event: AgentEvent) {
            if matches!(event, AgentEvent::TurnStarted) {
                self.turn_started.fetch_add(1, Ordering::SeqCst);
            }
        }

        async fn metric(&self, metric: AgentMetric) {
            if metric == AgentMetric::Count(AgentMetricCount::AdmittedContinuation) {
                self.count.fetch_add(1, Ordering::SeqCst);
                if let Some(entered) = self.entered.lock().unwrap().take() {
                    let _ = entered.send(());
                }
                let release = { self.release.lock().unwrap().take() };
                if let Some(release) = release {
                    let _ = release.await;
                }
                self.cancellation.cancel();
            }
        }
    }

    let (entered_tx, entered_rx) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = tokio::sync::oneshot::channel();
    let cancellation = CancellationToken::new();
    let held_sink = HeldMetric {
        entered: Mutex::new(Some(entered_tx)),
        release: Mutex::new(Some(release_rx)),
        cancellation: cancellation.clone(),
        count: AtomicUsize::new(0),
        turn_started: AtomicUsize::new(0),
    };
    let (provider, requests) = ScriptedProvider::new(vec![
        vec![Err(ProviderError::output_truncated(
            "partial answer".into(),
            Vec::new(),
        ))],
        turn("must not start", Vec::new()),
    ]);
    let executor = RecordingExecutor::default();
    let approval = RecordingApproval::default();
    let held_run = run(
        &provider,
        &executor,
        request("same user question", None),
        Vec::new(),
        control(
            limits,
            &approval,
            &held_sink,
            cancellation,
            preflight(Some(100_000), 64),
        ),
    );
    tokio::pin!(held_run);
    tokio::select! {
        result = &mut held_run => panic!("provider run finished before metric release: {result:?}"),
        _ = entered_rx => {},
    }
    assert_eq!(requests.lock().unwrap().len(), 1);
    assert_eq!(held_sink.count.load(Ordering::SeqCst), 1);
    assert_eq!(held_sink.turn_started.load(Ordering::SeqCst), 1);
    release_tx.send(()).unwrap();
    assert!(matches!(
        held_run.await,
        Err(saya_agent::AgentError::Cancelled)
    ));
    assert_eq!(requests.lock().unwrap().len(), 1);
    assert_eq!(held_sink.turn_started.load(Ordering::SeqCst), 1);
}
