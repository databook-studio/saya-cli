#[tokio::test]
async fn select_without_tables_is_complete() {
    let h = harness_with_database("select-one");
    seed_events(&h.database).await;
    let id = saved_sql_id(&h, "SELECT 1");

    let output = h.run(&["investigation", "run", &id]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "a query with no objects completes its review without --revalidate: {}{}",
        h.stdout(&output),
        h.stderr(&output)
    );
    assert!(
        h.stdout(&output).contains("\"event\":\"query_result\""),
        "{}",
        h.stdout(&output)
    );
    let _ = fs::remove_dir_all(h.root);
}

#[tokio::test]
async fn different_target_requires_revalidate() {
    let other_database = label_database("different-target-other");
    fs::write(&other_database, b"").unwrap();
    let h = harness(
        "different-target",
        format!(
            "[profiles.local]\ntype = 'sqlite'\npath = '{}'\n\n\
             [profiles.other]\ntype = 'sqlite'\npath = '{}'\n",
            label_database("different-target-local").display(),
            other_database.display()
        ),
    );
    let local_database = label_database("different-target-local");
    fs::write(&local_database, b"").unwrap();
    seed_events(&local_database).await;
    seed_events(&other_database).await;
    let id = saved_id(&h, Some("local"));

    let refused = h.run(&["investigation", "run", &id, "--connection", "other"]);
    assert_eq!(refused.status.code(), Some(2), "{}", h.stderr(&refused));
    let stderr = h.stderr(&refused);
    assert!(
        stderr.contains("target changed"),
        "a different profile is a stale review: {stderr}"
    );

    let revalidated = h.run(&[
        "investigation",
        "run",
        &id,
        "--connection",
        "other",
        "--revalidate",
    ]);
    assert_eq!(
        revalidated.status.code(),
        Some(0),
        "{}{}",
        h.stdout(&revalidated),
        h.stderr(&revalidated)
    );

    let plain = h.run(&["investigation", "run", &id, "--connection", "other"]);
    assert_eq!(
        plain.status.code(),
        Some(0),
        "{}{}",
        h.stdout(&plain),
        h.stderr(&plain)
    );
    let _ = fs::remove_dir_all(h.root);
    let _ = fs::remove_dir_all(local_database.parent().unwrap());
}

#[tokio::test]
async fn dialect_mismatch_refused() {
    let duck_database = label_database("dialect-duck");
    let h = harness(
        "dialect",
        format!(
            "[profiles.local]\ntype = 'sqlite'\npath = '{}'\n\n\
             [profiles.duck]\ntype = 'duckdb'\npath = '{}'\n",
            label_database("dialect-local").display(),
            duck_database.display()
        ),
    );
    let local_database = label_database("dialect-local");
    fs::write(&local_database, b"").unwrap();
    seed_events(&local_database).await;
    let id = saved_id(&h, Some("local"));

    let output = h.run(&["investigation", "run", &id, "--connection", "duck"]);
    assert_eq!(output.status.code(), Some(2), "{}", h.stderr(&output));
    let stderr = h.stderr(&output);
    assert!(
        stderr.contains("dialect mismatch"),
        "the refusal names the dialect: {stderr}"
    );
    assert!(
        !h.stdout(&output).contains("\"event\":\"query_result\""),
        "nothing executed: {}",
        h.stdout(&output)
    );
    let _ = fs::remove_dir_all(h.root);
    let _ = fs::remove_dir_all(local_database.parent().unwrap());
}
