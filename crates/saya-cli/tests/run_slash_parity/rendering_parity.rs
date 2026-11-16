#[tokio::test]
async fn json_framing_of_runs_matches_headless_json() {
    let _env = lock_env().await;
    let root = temp_root("json_parity");
    unsafe { set_runs_dir(&root.join("runs")) };
    let runtime = runtime_at(&root);
    let store = store_at(&root).await;
    seed_run(&root, &store, "r-parity-json", "audit the ledger", &[]).await;

    let headless = run_headless(
        RunCommand::Show {
            run_id: "r-parity-json".into(),
        },
        &runtime,
        &store,
        RenderFormat::Json,
    )
    .await;
    let slash = match parse_slash_command("/runs r-parity-json") {
        Ok(Some(SlashCommand::Runs(Some(run_id)))) => {
            run_headless(
                RunCommand::Show { run_id },
                &runtime,
                &store,
                RenderFormat::Json,
            )
            .await
        }
        other => panic!("expected Runs, got {other:?}"),
    };
    assert_eq!(slash.1, headless.1, "JSON framing diverged");
    assert_eq!(slash.2, headless.2);

    // One serde path: a paused event renders the same bytes as the journal
    // writes, in both JSON framings.
    let event = RunEvent::Paused {
        reason: saya_types::PauseReason::StepFailedAfterRetry,
    };
    let line = saya_cli::render_run_event(&event, RenderFormat::Ndjson).stdout;
    let json = saya_cli::render_run_event(&event, RenderFormat::Json).stdout;
    assert_eq!(line, json, "one serde path, two framings");
    assert_eq!(line.trim_end(), serde_json::to_string(&event).unwrap());

    unsafe { restore_runs_dir(None) };
    let _ = fs::remove_dir_all(root);
}

// ---------------------------------------------------------------------------
// 4. The per-endpoint usage section rides the shared stanza: `/runs <id>` and
//    `saya run show <id>` render it byte-identically, and the fold keeps the
//    honesty rule — a reported cache zero is 0, an unreported one unknown.
// ---------------------------------------------------------------------------
#[tokio::test]
async fn usage_renders_identically_on_both_paths_with_unknown_never_zero() {
    let _env = lock_env().await;
    let root = temp_root("usage_parity");
    unsafe { set_runs_dir(&root.join("runs")) };
    let runtime = runtime_at(&root);
    let store = store_at(&root).await;
    seed_run(
        &root,
        &store,
        "r-parity-usage",
        "audit the spend",
        &[
            RunEvent::Usage {
                endpoint: "primary".into(),
                tokens: Some(100),
                turns: None,
                tool_calls: None,
                cached_input_tokens: None,
                cache_creation_input_tokens: None,
            },
            RunEvent::Usage {
                endpoint: "primary".into(),
                tokens: Some(60),
                turns: None,
                tool_calls: None,
                cached_input_tokens: Some(0),
                cache_creation_input_tokens: None,
            },
            RunEvent::Usage {
                endpoint: "deepseek".into(),
                tokens: None,
                turns: None,
                tool_calls: None,
                cached_input_tokens: None,
                cache_creation_input_tokens: None,
            },
        ],
    )
    .await;

    let headless_show = run_headless(
        RunCommand::Show {
            run_id: "r-parity-usage".into(),
        },
        &runtime,
        &store,
        RenderFormat::Text,
    )
    .await;
    let slash = match parse_slash_command("/runs r-parity-usage") {
        Ok(Some(SlashCommand::Runs(Some(run_id)))) => {
            run_headless(
                RunCommand::Show { run_id },
                &runtime,
                &store,
                RenderFormat::Text,
            )
            .await
        }
        other => panic!("expected Runs, got {other:?}"),
    };

    assert_eq!(slash.0, 0, "stderr: {}", slash.2);
    // The whole stanza byte for byte — a usage section rendered at one call
    // site only would break exactly here.
    assert_eq!(slash.1, headless_show.1, "/runs diverged from run show");
    assert_eq!(slash.2, headless_show.2);
    let shown = &headless_show.1;
    assert!(
        shown.contains("usage:"),
        "the usage section renders: {shown}"
    );
    assert!(
        shown.contains("primary · tokens 160 · cache reads 0 · cache writes unknown"),
        "reported figures fold and a reported zero stays 0: {shown}"
    );
    assert!(
        shown.contains("deepseek · tokens unknown · cache reads unknown"),
        "figures no call reported render unknown, never zero: {shown}"
    );

    unsafe { restore_runs_dir(None) };
    let _ = fs::remove_dir_all(root);
}
