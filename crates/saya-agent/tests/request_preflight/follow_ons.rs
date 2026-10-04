use super::*;

#[derive(Debug, Clone, Copy)]
enum Route {
    Continuation,
    Designation,
    Salvage,
}

#[tokio::test]
async fn continuation_designation_and_salvage_share_the_pre_dispatch_guard() {
    for route in [Route::Continuation, Route::Designation, Route::Salvage] {
        let usage = TokenUsage::new(17, 9);
        let (turns, definitions, mut limits, expected_window) = match route {
            Route::Continuation => (
                vec![vec![Err(ProviderError::output_truncated(
                    "partial answer".into(),
                    Vec::new(),
                ))]],
                Vec::new(),
                AgentLimits::default(),
                400,
            ),
            Route::Designation => (
                vec![
                    turn_with_usage(
                        "",
                        vec![call(
                            "designation-id",
                            "designate_answer",
                            serde_json::json!({"sql": "SELECT ".to_owned() + &"1".repeat(2_500)}),
                        )],
                        usage,
                    ),
                    turn("must not dispatch", Vec::new()),
                ],
                vec![definition("designate_answer")],
                AgentLimits::default(),
                4_000,
            ),
            Route::Salvage => {
                let limits = AgentLimits {
                    max_turns: Some(1),
                    ..AgentLimits::default()
                };
                let mut query = definition("query");
                query.description = "q".into();
                (
                    vec![
                        turn_with_usage(
                            "prior answer",
                            vec![call(
                                "salvage-id",
                                "query",
                                serde_json::json!({"sql": "SELECT ".to_owned() + &"1".repeat(180)}),
                            )],
                            usage,
                        ),
                        turn("must not dispatch", Vec::new()),
                    ],
                    vec![query],
                    limits,
                    855,
                )
            }
        };
        limits.context_byte_budget = 16 * 1024;
        let (provider, requests) = ScriptedProvider::new(turns);
        let mut agent_request = request(
            "same user question",
            matches!(route, Route::Salvage).then(|| "required system ".repeat(3)),
        );
        if matches!(route, Route::Designation) {
            agent_request.history = vec![
                ChatMessage::text(
                    "user",
                    format!("OLD_ORDINARY_PAIR {}", "older ".repeat(250)),
                ),
                ChatMessage::text("assistant", "old answer ".repeat(250)),
            ];
        }
        let executor = RecordingExecutor::with_result(serde_json::json!({
            "columns":["value"],"rows":[[42]],"row_count":1,
            "truncated":false,"executed_sql":"SELECT 1"
        }));
        let sink = RecordingSink::default();
        let output = run(
            &provider,
            &executor,
            agent_request,
            definitions,
            control(
                limits,
                &RecordingApproval::default(),
                &sink,
                CancellationToken::new(),
                preflight(Some(1024 + 64 + expected_window), 64),
            ),
        )
        .await
        .unwrap();
        match route {
            Route::Continuation => {
                assert!(stopped(&output, BudgetDimension::RequestContextWindow));
                assert_eq!(requests.lock().unwrap().len(), 1);
                assert_eq!(executor.calls.load(Ordering::SeqCst), 0);
                assert_eq!(
                    sink.metrics
                        .lock()
                        .unwrap()
                        .iter()
                        .filter(|metric| **metric
                            == AgentMetric::Count(AgentMetricCount::OutputTruncated))
                        .count(),
                    1,
                    "the provider truncation is counted even when its continuation is refused"
                );
                assert_eq!(
                    sink.metrics
                        .lock()
                        .unwrap()
                        .iter()
                        .filter(|metric| {
                            **metric == AgentMetric::Count(AgentMetricCount::AdmittedContinuation)
                        })
                        .count(),
                    0,
                    "a truncated response refused by preflight is not an admitted continuation"
                );
            }
            Route::Designation => {
                assert_eq!(output.answer, "must not dispatch");
                let requests = requests.lock().unwrap();
                assert_eq!(requests.len(), 2);
                assert!(
                    requests
                        .iter()
                        .all(|request| provider_projection_bytes(request) <= 4_000)
                );
                let recovery = requests.last().unwrap();
                assert!(recovery.messages.iter().any(|message| {
                    message.role == "user" && message.content.contains("same user question")
                }));
                assert!(recovery.messages.iter().all(|message| {
                    !message.content.contains("OLD_ORDINARY_PAIR")
                        && !message.content.contains("old answer")
                }));
                assert_eq!(
                    sink.observations
                        .lock()
                        .unwrap()
                        .iter()
                        .filter(|observation| {
                            **observation == AnsweringObservation::RequestAdmitted
                        })
                        .count(),
                    2,
                    "the designation recovery receives the same preflight before admission"
                );
                assert_eq!(executor.calls.load(Ordering::SeqCst), 0);
                assert_eq!(
                    output.answer_sql.as_deref(),
                    Some(("SELECT ".to_owned() + &"1".repeat(2_500)).as_str())
                );
                assert_eq!(output.usage, usage);
            }
            Route::Salvage => {
                assert!(
                    stopped(&output, BudgetDimension::RequestContextWindow),
                    "events {:?}; requests {}",
                    output.events,
                    requests.lock().unwrap().len()
                );
                assert_eq!(requests.lock().unwrap().len(), 1);
                assert_eq!(executor.calls.load(Ordering::SeqCst), 1);
                assert_eq!(output.tool_metadata.len(), 1);
                assert_eq!(output.tool_metadata[0].status, "completed");
                let expected_sql = "SELECT ".to_owned() + &"1".repeat(180);
                assert_eq!(output.answer_sql.as_deref(), Some(expected_sql.as_str()));
                assert_eq!(output.usage, usage);
            }
        }
    }
}

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
