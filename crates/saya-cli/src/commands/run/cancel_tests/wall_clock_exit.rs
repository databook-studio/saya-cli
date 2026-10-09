#[tokio::test]
async fn panel_start_maps_settled_wall_clock_stop_to_exit_six() {
    let _env_lock = RUNS_DIR_TEST_LOCK.lock().await;
    let root = temp_root("start-deadline");
    let _runs_dir = RunsDirEnv::set(root.join("runs"));

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (episode_tx, episode_rx) = oneshot::channel();
    let (release_tx, release_rx) = oneshot::channel();
    let plan_content = serde_json::to_string(
        &RunPlan::new(vec![
            StepSpec::new(
                "wait for answer",
                Capabilities::default(),
                None,
                Vec::new(),
                None,
            )
            .unwrap(),
        ])
        .unwrap(),
    )
    .unwrap();
    let server = tokio::spawn(async move {
        let (mut planning, _) = listener.accept().await.unwrap();
        let _ = read_http_request(&mut planning).await;
        write_ollama_response(&mut planning, &plan_content).await;

        let (mut episode, _) = listener.accept().await.unwrap();
        let _ = read_http_request(&mut episode).await;
        let _ = episode_tx.send(());
        release_rx.await.unwrap();
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
    let run_id = RunId::parse("r-start-deadline").unwrap();
    let cancellation = saya_agent::CancellationToken::default();
    let approval = super::super::approval::PlanApproval::PreAuthorized;
    let active = start_for_panel(
        RunRequest {
            run_id: run_id.clone(),
            goal: Some("wait safely".into()),
            allow: vec!["none".into()],
            budget: vec!["wall-clock=1".into()],
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
    tokio::select! {
        biased;
        result = &mut active => panic!("panel start ended before the episode source began: {result:?}"),
        started = episode_rx => started.unwrap(),
    }
    tokio::time::timeout(Duration::from_secs(3), async {
        tokio::select! {
            biased;
            result = &mut active => panic!("panel start ended before its stop token: {result:?}"),
            _ = cancellation.cancelled() => {},
        }
    })
    .await
    .expect("independent run deadline signals the panel's shared token");

    let code = tokio::time::timeout(Duration::from_secs(2), &mut active)
        .await
        .expect("the local provider attempt settles")
        .unwrap();
    assert_eq!(code, 6);
    let events = Journal::open(root.join("runs").join(run_id.as_str()))
        .read()
        .unwrap();
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, RunEvent::Paused { .. }))
            .count(),
        1
    );
    assert!(events.contains(&RunEvent::Paused {
        reason: saya_types::PauseReason::WallClockExceeded,
    }));

    release_tx.send(()).unwrap();
    server.await.unwrap();
    let _ = fs::remove_dir_all(root);
}
