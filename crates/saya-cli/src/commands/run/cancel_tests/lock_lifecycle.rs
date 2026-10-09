#[tokio::test]
async fn panel_start_keeps_run_lock_until_cancelled_plan_source_returns() {
    let _env_lock = RUNS_DIR_TEST_LOCK.lock().await;
    let root = temp_root("start-settlement");
    let _runs_dir = RunsDirEnv::set(root.join("runs"));

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (request_tx, request_rx) = oneshot::channel();
    let (release_tx, release_rx) = oneshot::channel();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let request = read_http_request(&mut socket).await;
        let _ = request_tx.send(request);
        release_rx.await.unwrap();
        let body = "source failed after the panel stop";
        let response = format!(
            "HTTP/1.1 500 Internal Server Error\r\nContent-Type: application/x-ndjson\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        socket.write_all(response.as_bytes()).await.unwrap();
    });

    let mut runtime = runtime_at(&root);
    let endpoint = runtime
        .resolved
        .endpoints
        .get_mut(saya_config::ORCHESTRATOR_ROLE)
        .unwrap();
    endpoint.provider = saya_config::AiProvider::Ollama;
    endpoint.base_url = Some(format!("http://{address}"));
    let store = SqliteStateStore::new(root.join("state.sqlite3"));
    let run_id = RunId::parse("r-start-settlement").unwrap();
    let cancellation = saya_agent::CancellationToken::default();
    let approval = super::super::approval::PlanApproval::PreAuthorized;
    let active = start_for_panel(
        RunRequest {
            run_id: run_id.clone(),
            goal: Some("observe safely".into()),
            allow: vec!["none".into()],
            budget: Vec::new(),
        },
        &runtime,
        RenderFormat::Text,
        saya_agent::ApprovalPolicy::ReadOnly,
        &store,
        HostRun {
            journal_wire: None,
            agent_stream: None,
            profile: None,
            plan_approval: &approval,
            cancellation: cancellation.clone(),
        },
    );
    tokio::pin!(active);
    let request = tokio::select! {
        biased;
        result = &mut active => panic!("panel start returned before the provider request: {result:?}"),
        request = request_rx => request.unwrap(),
    };
    assert!(String::from_utf8_lossy(&request).contains("/api/chat"));

    cancellation.cancel();
    tokio::select! {
        biased;
        result = &mut active => panic!("panel start returned before source settlement: {result:?}"),
        _ = tokio::task::yield_now() => {}
    }
    let lock = root.join("runs").join(run_id.as_str()).join("lock");
    assert!(
        RunLock::acquire(&lock).is_err(),
        "the started run keeps its single-writer lock until its plan request returns"
    );

    release_tx.send(()).unwrap();
    let code = tokio::time::timeout(Duration::from_secs(2), &mut active)
        .await
        .expect("panel drive settles after local provider response")
        .unwrap();
    assert_eq!(code, 130);
    server.await.unwrap();
    let events = Journal::open(root.join("runs").join(run_id.as_str()))
        .read()
        .unwrap();
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, RunEvent::Cancelled))
            .count(),
        1
    );
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, RunEvent::StepStarted { .. }))
    );

    let _ = fs::remove_dir_all(root);
}
