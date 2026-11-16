#[tokio::test]
async fn machine_driven_run_of_a_legacy_binding_succeeds_and_leaves_it_byte_identical() {
    let root = temp_root("machine-legacy");
    let runtime = runtime_at(&root, &single_profile_toml(&root));
    let repo = repo_at(&root);
    seed_events(&root.join("data.sqlite3")).await;
    let id = saved_id(&repo, &runtime).await;
    let state_db = SqliteStateStore::new(root.join("state.sqlite3"));
    // The save-time binding predates fingerprinting: a reviewed revision,
    // no schema fingerprint — a human review; a machine replay may run it
    // but never completes that review.
    let binding_before = fs::read(binding_path(&root, &id)).unwrap();
    assert!(
        repo.get_binding(&InvestigationId::parse(&id).unwrap())
            .unwrap()
            .expect("the save wrote a binding")
            .reviewed_schema_fingerprint
            .is_none(),
        "setup: the legacy binding carries no fingerprint"
    );

    let cancellation = CancellationToken::new();
    let (outcome, out, err) =
        run_outcome_machine_driven(&repo, run_command(&id), &runtime, &state_db, &cancellation)
            .await;
    assert_eq!(outcome.code, 0, "out: {out} err: {err}");
    assert!(
        outcome.replay.is_some(),
        "a legacy binding replays under a machine-driven run"
    );
    assert_eq!(
        state_db.recent_audit(20).await.unwrap().len(),
        1,
        "the legacy replay executed once"
    );
    assert!(
        repo.get_binding(&InvestigationId::parse(&id).unwrap())
            .unwrap()
            .expect("the save wrote a binding")
            .reviewed_schema_fingerprint
            .is_none(),
        "a machine replay never completes the legacy review"
    );
    assert_eq!(
        fs::read(binding_path(&root, &id)).unwrap(),
        binding_before,
        "a machine replay leaves the legacy binding byte-identical"
    );
    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn machine_driven_run_of_a_fully_reviewed_binding_succeeds_and_leaves_it_byte_identical() {
    let root = temp_root("machine-reviewed");
    let runtime = runtime_at(&root, &single_profile_toml(&root));
    let repo = repo_at(&root);
    seed_events(&root.join("data.sqlite3")).await;
    let id = saved_id(&repo, &runtime).await;
    let state_db = SqliteStateStore::new(root.join("state.sqlite3"));
    // Complete the review the way the ordinary CLI does: one plain run
    // writes the schema fingerprint into the binding.
    let (done, out, err) = run_outcome(&repo, run_command(&id), &runtime, &state_db).await;
    assert_eq!(done.code, 0, "out: {out} err: {err}");
    assert!(
        repo.get_binding(&InvestigationId::parse(&id).unwrap())
            .unwrap()
            .expect("the plain run wrote the binding")
            .reviewed_schema_fingerprint
            .is_some(),
        "setup: the review is complete (a fingerprint is recorded)"
    );
    let binding_before = fs::read(binding_path(&root, &id)).unwrap();

    let cancellation = CancellationToken::new();
    let (outcome, out, err) =
        run_outcome_machine_driven(&repo, run_command(&id), &runtime, &state_db, &cancellation)
            .await;
    assert_eq!(outcome.code, 0, "out: {out} err: {err}");
    assert!(
        outcome.replay.is_some(),
        "a fully reviewed binding replays under a machine-driven run"
    );
    assert_eq!(
        fs::read(binding_path(&root, &id)).unwrap(),
        binding_before,
        "a machine-driven replay never refreshes review authority"
    );
    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn an_ordinary_run_still_establishes_the_binding_of_an_unbound_investigation() {
    let root = temp_root("ordinary-binds");
    let runtime = runtime_at(&root, &single_profile_toml(&root));
    let repo = repo_at(&root);
    seed_events(&root.join("data.sqlite3")).await;
    let id = saved_id(&repo, &runtime).await;
    let state_db = SqliteStateStore::new(root.join("state.sqlite3"));
    // Unbound: drop the save-time binding — the imported-document shape.
    fs::remove_file(binding_path(&root, &id)).unwrap();

    // The ordinary CLI step (never machine-driven): an explicit
    // --connection run establishes the binding on success (existing
    // behavior, pinned).
    let command = InvestigationCommand::Run {
        id: id.clone(),
        connection: Some("local".into()),
        revalidate: false,
        report: None,
        rows: None,
        overwrite: false,
        params: Vec::new(),
    };
    let (outcome, out, err) = run_outcome(&repo, command, &runtime, &state_db).await;
    assert_eq!(outcome.code, 0, "out: {out} err: {err}");
    let binding = repo
        .get_binding(&InvestigationId::parse(&id).unwrap())
        .unwrap()
        .expect("the ordinary run establishes the binding");
    assert_eq!(binding.profile, "local");
    let _ = fs::remove_dir_all(root);
}
