#[tokio::test]
async fn pre_cancelled_typed_replay_does_not_execute_or_refresh_its_binding() {
    let root = temp_root("outcome-cancelled");
    let runtime = runtime_at(&root, &single_profile_toml(&root));
    let repo = repo_at(&root);
    seed_events(&root.join("data.sqlite3")).await;
    let id = saved_id(&repo, &runtime).await;
    let state_db = SqliteStateStore::new(root.join("state.sqlite3"));

    // Positive control through the new typed entry: this exact saved replay
    // executes and records its ordinary successful result/audit.
    let control = CancellationToken::new();
    let (completed, out, err) =
        run_outcome_with_cancellation(&repo, run_command(&id), &runtime, &state_db, &control).await;
    assert_eq!(completed.code, 0, "out: {out} err: {err}");
    assert!(completed.replay.is_some(), "the positive control executes");
    let audit_count = state_db.recent_audit(20).await.unwrap().len();
    assert_eq!(audit_count, 1, "the successful control is audited once");
    let binding_before = fs::read(binding_path(&root, &id)).unwrap();

    let cancellation = CancellationToken::new();
    cancellation.cancel();
    let (stopped, out, err) =
        run_outcome_with_cancellation(&repo, run_command(&id), &runtime, &state_db, &cancellation)
            .await;

    assert_eq!(stopped.code, 2, "out: {out} err: {err}");
    assert!(
        stopped.replay.is_none(),
        "a stopped replay carries no result"
    );
    assert!(
        err.contains("cancelled"),
        "the fixed refusal is captured: {err}"
    );
    assert_eq!(
        state_db.recent_audit(20).await.unwrap().len(),
        audit_count,
        "a pre-cancelled replay does not write a query audit"
    );
    assert_eq!(
        fs::read(binding_path(&root, &id)).unwrap(),
        binding_before,
        "a stopped replay does not refresh review authority"
    );
    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn machine_driven_run_of_an_unbound_investigation_is_refused_without_executing() {
    let root = temp_root("machine-unbound");
    let runtime = runtime_at(&root, &single_profile_toml(&root));
    let repo = repo_at(&root);
    seed_events(&root.join("data.sqlite3")).await;
    let id = saved_id(&repo, &runtime).await;
    let state_db = SqliteStateStore::new(root.join("state.sqlite3"));
    // Unbound: drop the save-time binding — the imported-document shape the
    // step import deliberately leaves to a human.
    fs::remove_file(binding_path(&root, &id)).unwrap();

    let cancellation = CancellationToken::new();
    let (outcome, out, err) =
        run_outcome_machine_driven(&repo, run_command(&id), &runtime, &state_db, &cancellation)
            .await;
    assert_eq!(outcome.code, 2, "out: {out} err: {err}");
    assert!(outcome.replay.is_none(), "a refusal carries no replay");
    assert!(
        err.contains("no local review binding"),
        "the refusal names the missing human review: {err}"
    );
    assert!(
        state_db.recent_audit(20).await.unwrap().is_empty(),
        "nothing executed: no audit row"
    );
    assert!(
        repo.get_binding(&InvestigationId::parse(&id).unwrap())
            .unwrap()
            .is_none(),
        "a refused machine run writes no binding"
    );
    let _ = fs::remove_dir_all(root);
}
