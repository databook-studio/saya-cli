#[tokio::test]
async fn answering_deadline_starts_before_postgres_setup_and_awaits_the_source() {
    let root = temp_root();
    let (setup_started_tx, setup_started) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let (port, source) = postgres_setup_source(setup_started_tx, release_rx);
    let runtime = runtime_for_postgres_setup(&root, port).await;
    let cancellation = CancellationToken::new();
    let sink = RecordingSink {
        events: Arc::new(Mutex::new(Vec::new())),
        knowledge_log: Arc::new(Mutex::new(Vec::new())),
    };
    let work = run_prompt_with_sink_and_outcomes(SinkPrompt {
        runtime: &runtime,
        prompt: "count rows",
        approval: ApprovalPolicy::ReadOnly,
        can_prompt: false,
        can_obtain_approval: false,
        overrides: PromptOverrides::default(),
        history: Vec::new(),
        sink: &sink,
        cancellation: cancellation.clone(),
        question_scope: None,
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
    let setup_wait = tokio::time::timeout(Duration::from_secs(3), setup_started);
    tokio::pin!(setup_wait);
    tokio::select! {
        setup = &mut setup_wait => match setup {
            Ok(Ok(())) => {}
            Ok(Err(_)) => panic!("runtime stopped before PostgreSQL setup observation"),
            Err(_) => {
                let _ = release_tx.send(());
                panic!(
                    "the production PostgreSQL connector did not send startup bytes: {:?}",
                    source.join().expect("PostgreSQL setup source thread joins")
                );
            }
        },
        result = &mut work => panic!("runtime settled before PostgreSQL setup observation: {result:?}"),
    }

    tokio::select! {
        _ = tokio::time::sleep(Duration::from_millis(1200)) => {}
        result = &mut work => panic!("runtime settled before setup source release: {result:?}"),
    }
    assert_eq!(
        cancellation.reason(),
        Some(saya_agent::CancellationReason::Budget(
            BudgetDimension::ElapsedTime
        )),
        "the supplied token latches while prepare_turn is still in PostgreSQL setup"
    );
    assert!(
        sink.events.lock().expect("event log lock").is_empty(),
        "setup cancellation reaches no provider or tool event"
    );

    release_tx
        .send(())
        .expect("release PostgreSQL setup source");
    let result = tokio::time::timeout(Duration::from_secs(3), &mut work)
        .await
        .expect("runtime awaits source settlement after cancellation");
    assert!(
        result.is_err(),
        "the released synthetic setup must fail the exact runtime"
    );
    source
        .join()
        .expect("PostgreSQL setup source thread joins")
        .expect("PostgreSQL setup source observed and released one startup");
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn answering_deadline_disarms_before_source_observed_extraction() {
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
        question_scope: None,
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
            "runtime settled before source-observed extraction: {result:?}; events: {:?}",
            events.lock().expect("event log lock")
        ),
    }

    // The source, not a sleep, establishes extraction is now pending. Holding
    // it longer than the one-second answering budget catches a timer that
    // remains armed for ancillary work.
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
        "the answering clock must be disarmed while extraction is pending"
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
    let events = events.lock().expect("event log lock");
    assert!(events.iter().any(|event| matches!(
        event,
        AgentEvent::ToolCompleted { name, .. } if name == "bounded_sql_query"
    )));
    assert!(
        events
            .iter()
            .any(|event| matches!(event, AgentEvent::KnowledgeLearningStarted))
    );
    assert!(
        !events.iter().any(|event| matches!(
            event,
            AgentEvent::AnsweringBudgetStopped {
                dimension: BudgetDimension::ElapsedTime
            }
        )),
        "ancillary extraction does not add an answering budget stop"
    );
    let _ = std::fs::remove_dir_all(root);
}
