/// A hand-written v1 definition with an empty stored objects list, exactly
/// what a lying exporter could hand over (the audit repro's on-disk shape).
const EMPTY_OBJECTS_DEFINITION: &str = r#"{
  "format": "saya.investigation",
  "version": 1,
  "id": "order-events-01234567",
  "revision": 1,
  "name": "Order events",
  "sql": "SELECT id, label FROM events ORDER BY id",
  "dialect": "sqlite",
  "connection": "local",
  "objects": [],
  "schema_fingerprint": null,
  "created_unix_ms": 1700000000000,
  "updated_unix_ms": 1700000000000
}"#;

#[tokio::test]
async fn run_ignores_stored_objects_in_review() {
    let h = harness_with_database("ignored-objects");
    seed_events(&h.database).await;
    // The document lands on disk with `objects: []` and no binding. The
    // review must take its dependencies from the SQL, so the first replay
    // still binds the referenced table's fingerprint.
    let id = "order-events-01234567";
    fs::write(h.document_path(id), EMPTY_OBJECTS_DEFINITION).unwrap();

    let first = h.run(&["investigation", "run", id, "--connection", "local"]);
    assert_eq!(
        first.status.code(),
        Some(0),
        "{}{}",
        h.stdout(&first),
        h.stderr(&first)
    );

    alter_table(&h.database, "ALTER TABLE events ADD COLUMN extra TEXT").await;

    let stale = h.run(&["investigation", "run", id]);
    assert_eq!(
        stale.status.code(),
        Some(2),
        "the empty stored list must not skip the schema review: {}{}",
        h.stdout(&stale),
        h.stderr(&stale)
    );
    assert!(
        h.stderr(&stale).contains("schema changed"),
        "the refusal names what changed: {}",
        h.stderr(&stale)
    );
    assert!(
        !h.stdout(&stale).contains("\"event\":\"query_result\""),
        "nothing executed: {}",
        h.stdout(&stale)
    );
    let _ = fs::remove_dir_all(h.root);
}

#[tokio::test]
async fn quoted_dotted_table_gets_a_real_fingerprint() {
    let h = harness_with_database("dotted-table");
    {
        let options = sqlx::sqlite::SqliteConnectOptions::new()
            .filename(&h.database)
            .create_if_missing(true);
        let pool = SqlitePool::connect_with(options).await.unwrap();
        sqlx::query(r#"CREATE TABLE "orders.v1" (id INTEGER PRIMARY KEY, label TEXT NOT NULL)"#)
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(r#"INSERT INTO "orders.v1" (id, label) VALUES (1, 'first')"#)
            .execute(&pool)
            .await
            .unwrap();
        pool.close().await;
    }

    let id = saved_sql_id(&h, r#"SELECT id, label FROM "orders.v1""#);

    // The document names the dotted table as one canonically quoted object,
    // never a schema/table split on the dot inside the name.
    let document: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(h.document_path(&id)).unwrap()).unwrap();
    assert_eq!(
        document["objects"],
        serde_json::json!([r#""orders.v1""#]),
        "objects: {document}"
    );

    // The replay resolves that one object and binds its real fingerprint.
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
        "the dotted table contributed a real fingerprint: {binding}"
    );

    alter_table(
        &h.database,
        r#"ALTER TABLE "orders.v1" ADD COLUMN extra TEXT"#,
    )
    .await;

    let stale = h.run(&["investigation", "run", &id]);
    assert_eq!(
        stale.status.code(),
        Some(2),
        "a change to the dotted table invalidates the review: {}{}",
        h.stdout(&stale),
        h.stderr(&stale)
    );
    assert!(
        h.stderr(&stale).contains("schema changed"),
        "the refusal names what changed: {}",
        h.stderr(&stale)
    );
    let _ = fs::remove_dir_all(h.root);
}

#[tokio::test]
async fn missing_table_is_unverifiable_not_reviewed() {
    let h = harness_with_database("missing-table");
    // AUTOINCREMENT makes SQLite create sqlite_sequence: a live, queryable
    // table the schema discovery never lists, so the SQL names a table the
    // tree cannot resolve.
    {
        let options = sqlx::sqlite::SqliteConnectOptions::new()
            .filename(&h.database)
            .create_if_missing(true);
        let pool = SqlitePool::connect_with(options).await.unwrap();
        sqlx::query(
            "CREATE TABLE events (id INTEGER PRIMARY KEY AUTOINCREMENT, label TEXT NOT NULL)",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query("INSERT INTO events (label) VALUES ('first')")
            .execute(&pool)
            .await
            .unwrap();
        pool.close().await;
    }

    let id = saved_sql_id(&h, "SELECT * FROM sqlite_sequence");

    // A missing object is unverifiable: refused, and never recorded as a
    // reviewed fingerprint via a missing-name constant.
    let refused = h.run(&["investigation", "run", &id]);
    assert_eq!(
        refused.status.code(),
        Some(2),
        "{}{}",
        h.stdout(&refused),
        h.stderr(&refused)
    );
    let stderr = h.stderr(&refused);
    // The refusal is JSON-encoded, so the name's display quotes arrive
    // escaped.
    assert!(
        stderr.contains(r#"schema review unavailable: table \"sqlite_sequence\" not found"#),
        "{stderr}"
    );
    assert!(stderr.contains("--revalidate"), "{stderr}");
    assert!(
        !h.stdout(&refused).contains("\"event\":\"query_result\""),
        "nothing executed: {}",
        h.stdout(&refused)
    );

    // With --revalidate the run proceeds — the query itself succeeds — and
    // the rewritten binding records no fingerprint: it claims no review.
    let revalidated = h.run(&["investigation", "run", &id, "--revalidate"]);
    assert_eq!(
        revalidated.status.code(),
        Some(0),
        "{}{}",
        h.stdout(&revalidated),
        h.stderr(&revalidated)
    );
    let binding = fs::read_to_string(h.binding_path(&id)).unwrap();
    assert!(
        binding.contains("\"reviewed_schema_fingerprint\": null"),
        "the revalidated run must not claim a schema review: {binding}"
    );

    // The next run is unverifiable again.
    let again = h.run(&["investigation", "run", &id]);
    assert_eq!(
        again.status.code(),
        Some(2),
        "{}{}",
        h.stdout(&again),
        h.stderr(&again)
    );
    assert!(
        h.stderr(&again).contains("schema review unavailable"),
        "{}",
        h.stderr(&again)
    );
    let _ = fs::remove_dir_all(h.root);
}
