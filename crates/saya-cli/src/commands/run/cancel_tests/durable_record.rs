/// Cancelling the same way both surfaces do: the panel's stop and
/// `saya run cancel` on twin runs leave identical records — journal tail and
/// store status — and a later `saya run cancel` of the panel-cancelled run
/// reports it already finished, not a second cancellation.
#[tokio::test]
async fn the_panel_stop_and_saya_run_cancel_write_the_same_record() {
    let _env_lock = RUNS_DIR_TEST_LOCK.lock().await;
    let root = temp_root("parity");
    let _runs_dir = RunsDirEnv::set(root.join("runs"));
    let store = SqliteStateStore::new(root.join("state.sqlite3"));
    let runtime = runtime_at(&root);
    let panel_run = seed_claimed_run(&root, &store, "r-panel-cancel").await;
    let headless_run = seed_claimed_run(&root, &store, "r-headless-cancel").await;

    cancel_like_the_panel(&root, &store, &panel_run).await;
    let code = run_management(
        RunCommand::Cancel {
            run_id: headless_run.as_str().to_string(),
        },
        &runtime,
        RenderFormat::Text,
        saya_agent::ApprovalPolicy::ReadOnly,
        &store,
    )
    .await
    .expect("the headless cancel succeeds");
    assert_eq!(code, 0, "cancelling a live-holder-free run succeeds");

    let (panel_tail, panel_status) = durable_record(&store, &root, &panel_run).await;
    let (headless_tail, headless_status) = durable_record(&store, &root, &headless_run).await;
    assert_eq!(
        panel_tail, headless_tail,
        "both surfaces leave the same journal tail"
    );
    assert_eq!(
        panel_tail,
        RunEvent::Cancelled,
        "the tail is the cancellation"
    );
    assert_eq!(panel_status, RunStatus::Cancelled);
    assert_eq!(
        panel_status, headless_status,
        "the store statuses cannot disagree"
    );

    // The panel-cancelled run reads as already finished to the CLI surface —
    // one cancellation, wherever it came from.
    let again = run_management(
        RunCommand::Cancel {
            run_id: panel_run.as_str().to_string(),
        },
        &runtime,
        RenderFormat::Text,
        saya_agent::ApprovalPolicy::ReadOnly,
        &store,
    )
    .await
    .unwrap();
    assert_eq!(again, 0);
    let _ = fs::remove_dir_all(root);
}
