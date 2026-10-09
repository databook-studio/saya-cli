#[tokio::test]
async fn supplied_unlimited_question_does_not_inherit_a_new_setup_deadline() {
    let root = temp_root();
    let (started_tx, started_rx) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let (port, source) = postgres_setup_source(started_tx, release_rx);
    let mut runtime = runtime_for_postgres_setup(&root, port).await;
    runtime.resolved.investigation_budgets.elapsed_seconds = BudgetLimit::Unlimited;
    let cancellation = CancellationToken::new();
    let scope = crate::agent::question_scope::QuestionScope::new(&runtime, cancellation.clone());
    runtime.resolved.investigation_budgets.elapsed_seconds = finite(1);
    let sink = RecordingSink {
        events: Arc::new(Mutex::new(Vec::new())),
        knowledge_log: Arc::new(Mutex::new(Vec::new())),
    };
    let work = run_prompt_with_sink_and_outcomes(SinkPrompt {
        runtime: &runtime,
        prompt: "a frozen unlimited question",
        approval: ApprovalPolicy::ReadOnly,
        can_prompt: false,
        can_obtain_approval: false,
        overrides: PromptOverrides::default(),
        history: Vec::new(),
        sink: &sink,
        cancellation: cancellation.clone(),
        question_scope: Some(scope),
        state_db: None,
        decider: None,
        last_sql: None,
        session: None,
        agent_mode: AgentMode::Build,
        capture: None,
        prior_tool_outcomes: None,
        continuation: None,
        compaction_summary: None,
    });
    tokio::pin!(work);
    let started = tokio::time::timeout(Duration::from_secs(3), started_rx);
    tokio::pin!(started);
    tokio::select! {
        observed = &mut started => observed.unwrap().unwrap(),
        result = &mut work => panic!("work settled before the setup source: {result:?}"),
    }
    tokio::select! {
        _ = tokio::time::sleep(Duration::from_millis(1200)) => {},
        result = &mut work => panic!("work settled before source release: {result:?}"),
    }
    let reason_before_release = cancellation.reason();
    release_tx.send(()).unwrap();
    assert!(
        tokio::time::timeout(Duration::from_secs(3), &mut work)
            .await
            .unwrap()
            .is_err()
    );
    source.join().unwrap().unwrap();
    assert_eq!(
        reason_before_release, None,
        "the supplied unlimited scope owns the clock despite a different fresh configuration"
    );
    assert_eq!(cancellation.reason(), None);
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn supplied_scope_charges_answering_once_and_pauses_for_learning() {
    let root = temp_root();
    let (extraction_started_tx, extraction_started) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let (base_url, source) = timer_source(extraction_started_tx, release_rx, Arc::clone(&seen));
    let runtime = runtime_for_timer_source(&root, base_url).await;
    let events = Arc::new(Mutex::new(Vec::new()));
    let sink = RecordingSink {
        events: Arc::clone(&events),
        knowledge_log: Arc::new(Mutex::new(Vec::new())),
    };
    let cancellation = CancellationToken::new();
    let scope = crate::agent::question_scope::QuestionScope::new(&runtime, cancellation.clone());
    let state = SqliteStateStore::new(root.join("state.sqlite3"));
    let work = run_prompt_with_sink_and_outcomes(SinkPrompt {
        runtime: &runtime,
        prompt: "Remember that analytics is the default source, then find a value.",
        approval: ApprovalPolicy::ReadOnly,
        can_prompt: false,
        can_obtain_approval: false,
        overrides: PromptOverrides::default(),
        history: Vec::new(),
        sink: &sink,
        cancellation: cancellation.clone(),
        question_scope: Some(scope.clone()),
        state_db: Some(state),
        decider: None,
        last_sql: None,
        session: None,
        agent_mode: AgentMode::Build,
        capture: None,
        prior_tool_outcomes: None,
        continuation: None,
        compaction_summary: None,
    });
    tokio::pin!(work);

    let extraction_wait = tokio::time::timeout(Duration::from_secs(3), extraction_started);
    tokio::pin!(extraction_wait);
    tokio::select! {
        source_wait = &mut extraction_wait => match source_wait {
            Ok(Ok(())) => {}
            Ok(Err(_)) => panic!("timer source stopped before extraction observation"),
            Err(_) => panic!("real extraction request did not reach the local source"),
        },
        result = &mut work => panic!(
            "runtime settled before source-observed extraction: {result:?}"
        ),
    }

    // The source, not a sleep, establishes extraction is pending. Holding it
    // past the one-second answering ceiling proves the scope's clock is
    // disarmed for post-turn learning: the run neither settles nor trips.
    let past_answering_deadline = tokio::time::sleep(Duration::from_millis(1200));
    tokio::pin!(past_answering_deadline);
    tokio::select! {
        _ = &mut past_answering_deadline => {}
        result = &mut work => panic!(
            "runtime settled while the extraction source was intentionally pending: {result:?}"
        ),
    }
    assert_eq!(
        cancellation.reason(),
        None,
        "the held extraction must not charge the question scope while it is pending"
    );

    release_tx
        .send(())
        .expect("release bounded extraction source");
    let output = tokio::time::timeout(Duration::from_secs(3), &mut work)
        .await
        .expect("runtime settles after extraction source release")
        .expect("answering and extraction complete");
    source
        .join()
        .expect("timer source thread joins")
        .expect("timer source handled the expected requests");

    // The run is over and learning really happened; the ceiling never fired.
    assert_eq!(
        cancellation.reason(),
        None,
        "learning time must not be charged to the question scope"
    );
    assert_eq!(output.answer, "The query returned one.");
    assert_eq!(output.usage, TokenUsage::new(7, 2));
    assert_eq!(output.learning_usage, Some(TokenUsage::new(3, 1)));
    assert_eq!(
        *seen.lock().expect("request log lock"),
        vec![
            RequestKind::Tool,
            RequestKind::Answer,
            RequestKind::Extraction
        ],
        "one tool-bearing answering round, one final answering round, then one extraction"
    );
    assert!(
        events
            .lock()
            .expect("event log lock")
            .iter()
            .any(|event| matches!(event, AgentEvent::KnowledgeLearningStarted)),
        "learning must actually run for the pause to be exercised"
    );
    assert!(
        !events
            .lock()
            .expect("event log lock")
            .iter()
            .any(|event| matches!(
                event,
                AgentEvent::AnsweringBudgetStopped {
                    dimension: BudgetDimension::ElapsedTime
                }
            )),
        "the elapsed-time ceiling must never fire across answering and learning"
    );

    // The same scope was debited once for answering and never reset: a fresh
    // segment opens with real, non-zero, sub-ceiling time left. The full
    // configured ceiling is the fixture's one-second elapsed budget.
    let remaining = scope
        .begin_segment()
        .expect("segment reopens after learning");
    assert!(remaining > Duration::ZERO, "answering time was not debited");
    assert!(
        remaining < Duration::from_secs(1),
        "the remaining budget was reset instead of debited once"
    );
    scope.finish_segment();
    let _ = std::fs::remove_dir_all(root);
}
