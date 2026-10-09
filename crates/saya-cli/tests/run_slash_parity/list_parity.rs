#[tokio::test]
async fn runs_slash_and_headless_agree_byte_for_byte() {
    let _env = lock_env().await;
    let root = temp_root("show_parity");
    unsafe { set_runs_dir(&root.join("runs")) };
    let runtime = runtime_at(&root);
    let store = store_at(&root).await;
    seed_run(&root, &store, "r-parity-1", "survey the data quality", &[]).await;

    let headless_show = run_headless(
        RunCommand::Show {
            run_id: "r-parity-1".into(),
        },
        &runtime,
        &store,
        RenderFormat::Text,
    )
    .await;
    let run_id = parse_runs("/runs r-parity-1");
    let (code, out, err) = run_headless(
        RunCommand::Show {
            run_id: run_id.clone().expect("the slash form carries the id"),
        },
        &runtime,
        &store,
        RenderFormat::Text,
    )
    .await;

    assert_eq!(code, 0, "/runs <id> stderr: {err}");
    assert_eq!(run_id.as_deref(), Some("r-parity-1"));
    // The whole rendered stanza — status, goal, scopes, timestamps — byte for
    // byte. A second renderer would diverge here.
    assert_eq!(
        out, headless_show.1,
        "/runs <id> diverged from `saya run show`"
    );
    assert_eq!(err, headless_show.2);
    // And the show actually says the run.
    assert!(out.contains("survey the data quality"), "{out}");
    assert!(out.contains("completed"), "{out}");

    // The list form is the same dispatcher too: `/runs` (no id) → `List`.
    let headless_list = run_headless(RunCommand::List, &runtime, &store, RenderFormat::Text).await;
    let slash_list = match parse_slash_command("/runs") {
        Ok(Some(SlashCommand::Runs(None))) => {
            run_headless(RunCommand::List, &runtime, &store, RenderFormat::Text).await
        }
        other => panic!("expected Runs(None) for /runs, got {other:?}"),
    };
    assert_eq!(slash_list.1, headless_list.1, "/runs diverged from list");
    assert_eq!(slash_list.2, headless_list.2);
    assert!(
        slash_list.1.contains("r-parity-1"),
        "the seeded run is listed: {}",
        slash_list.1
    );

    unsafe { restore_runs_dir(None) };
    let _ = fs::remove_dir_all(root);
}

// ---------------------------------------------------------------------------
// 2. An unknown id fails identically on both paths: same exit class, same
//    bytes, and the message names the miss instead of panicking.
// ---------------------------------------------------------------------------
#[tokio::test]
async fn unknown_run_id_fails_identically_on_both_paths() {
    let _env = lock_env().await;
    let root = temp_root("unknown_parity");
    unsafe { set_runs_dir(&root.join("runs")) };
    let runtime = runtime_at(&root);
    let store = store_at(&root).await;

    let headless = run_headless(
        RunCommand::Show {
            run_id: "r-nope".into(),
        },
        &runtime,
        &store,
        RenderFormat::Text,
    )
    .await;
    let slash = match parse_slash_command("/runs r-nope") {
        Ok(Some(SlashCommand::Runs(Some(run_id)))) => {
            run_headless(
                RunCommand::Show { run_id },
                &runtime,
                &store,
                RenderFormat::Text,
            )
            .await
        }
        other => panic!("expected Runs(Some(r-nope)), got {other:?}"),
    };

    assert_ne!(headless.0, 0, "an unknown id must fail");
    assert_eq!(slash.0, headless.0, "exit class diverged");
    assert_eq!(slash.1, headless.1, "stdout diverged");
    assert_eq!(slash.2, headless.2, "stderr diverged");
    assert!(
        format!("{}{}", slash.1, slash.2).contains("no run with id"),
        "the failure must say why: {}{}",
        slash.1,
        slash.2
    );

    unsafe { restore_runs_dir(None) };
    let _ = fs::remove_dir_all(root);
}
