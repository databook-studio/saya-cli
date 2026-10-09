#[tokio::test]
async fn replay_does_not_call_provider() {
    let h = harness_with_database("provider");
    seed_events(&h.database).await;
    let id = saved_id(&h, None);

    let output = h.run_env(&["investigation", "run", &id], &provider_env());
    assert_eq!(
        output.status.code(),
        Some(0),
        "replay must not need a provider: {}{}",
        h.stdout(&output),
        h.stderr(&output)
    );
    let stdout = h.stdout(&output);
    assert!(
        stdout.contains("\"event\":\"query_result\""),
        "the normal query result event renders: {stdout}"
    );
    assert!(
        stdout.contains("saved investigation"),
        "an evidence line with the saved source follows: {stdout}"
    );
    let _ = fs::remove_dir_all(h.root);
}

#[tokio::test]
async fn replay_cannot_fall_back_to_active_profile() {
    // The definition is written directly into the dir with no binding, as an
    // import would leave it; the single profile is still the active one.
    let source = harness_with_database("fallthrough-source");
    seed_events(&source.database).await;
    let id = saved_id(&source, None);
    let document = fs::read_to_string(source.document_path(&id)).unwrap();

    let h = harness_with_database("fallthrough");
    seed_events(&h.database).await;
    fs::write(h.document_path(&id), document).unwrap();

    let output = h.run(&["investigation", "run", &id]);
    assert_eq!(output.status.code(), Some(2), "{}", h.stderr(&output));
    assert!(
        h.stderr(&output)
            .contains("no local connection mapped: pass --connection <profile>"),
        "the refusal names the explicit flag: {}",
        h.stderr(&output)
    );
    assert!(
        !h.stdout(&output).contains("\"event\":\"query_result\""),
        "nothing executed: {}",
        h.stdout(&output)
    );
    let _ = fs::remove_dir_all(source.root);
    let _ = fs::remove_dir_all(h.root);
}

#[tokio::test]
async fn changed_schema_invalidates_review() {
    let h = harness_with_database("schema-drift");
    seed_events(&h.database).await;
    let id = saved_id(&h, None);

    // First run: the save-time binding carries no fingerprint, so this run
    // completes the review with the live schema fingerprint.
    let first = h.run(&["investigation", "run", &id]);
    assert_eq!(
        first.status.code(),
        Some(0),
        "{}{}",
        h.stdout(&first),
        h.stderr(&first)
    );
    let binding = fs::read_to_string(h.binding_path(&id)).unwrap();
    assert!(
        binding.contains("reviewed_schema_fingerprint"),
        "the first successful run binds the fingerprint: {binding}"
    );

    // A fresh, unchanged replay still passes.
    let second = h.run(&["investigation", "run", &id]);
    assert_eq!(
        second.status.code(),
        Some(0),
        "{}{}",
        h.stdout(&second),
        h.stderr(&second)
    );

    // The referenced table changes underneath the review.
    let options = sqlx::sqlite::SqliteConnectOptions::new().filename(&h.database);
    let pool = SqlitePool::connect_with(options).await.unwrap();
    sqlx::query("ALTER TABLE events ADD COLUMN extra TEXT")
        .execute(&pool)
        .await
        .unwrap();
    pool.close().await;

    let stale = h.run(&["investigation", "run", &id]);
    assert_eq!(stale.status.code(), Some(2), "{}", h.stderr(&stale));
    let stderr = h.stderr(&stale);
    assert!(
        stderr.contains("schema changed"),
        "the refusal names what changed: {stderr}"
    );
    assert!(
        !h.stdout(&stale).contains("\"event\":\"query_result\""),
        "a stale review never executes: {}",
        h.stdout(&stale)
    );

    let revalidated = h.run(&["investigation", "run", &id, "--revalidate"]);
    assert_eq!(
        revalidated.status.code(),
        Some(0),
        "{}{}",
        h.stdout(&revalidated),
        h.stderr(&revalidated)
    );

    // The rewritten binding makes plain replays pass again.
    let after = h.run(&["investigation", "run", &id]);
    assert_eq!(
        after.status.code(),
        Some(0),
        "{}{}",
        h.stdout(&after),
        h.stderr(&after)
    );
    let _ = fs::remove_dir_all(h.root);
}
