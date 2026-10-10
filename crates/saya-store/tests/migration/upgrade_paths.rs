use super::*;

#[tokio::test]
async fn upgrade_from_version_one_preserves_data() {
    let root = temp_root("upgrade");
    let db = root.join("state.sqlite3");
    build_version_one_database(&db).await;
    let store = SqliteStateStore::new(&db);
    store.list_schema_metadata().await.unwrap();
    store.close().await;
    assert_eq!(user_version(&db).await, 14);
    // Step 6 drops the legacy tables even on a v1 upgrade path.
    assert_eq!(contract_tables(&db).await.len(), 0);
    let pool = read_pool(&db).await;
    let (json, updated, version): (String, i64, i64) = sqlx::query_as(
        "SELECT schema_json, updated_unix_ms, version FROM schema_cache WHERE profile_id='p-test'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(json, "{}");
    assert_eq!(updated, 11111);
    assert_eq!(version, 1);
    let row: AuditRow =
        sqlx::query_as("SELECT created_unix_ms, session_id, profile_id, operation, status, duration_ms, row_count, truncated FROM audit_log ORDER BY id LIMIT 1")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(row.0, 22222);
    assert!(row.1.is_none());
    assert_eq!(row.2, "p-test");
    assert_eq!(row.3, "query");
    assert_eq!(row.4, "success");
    assert_eq!(row.5, 10);
    assert_eq!(row.6, Some(5));
    assert_eq!(row.7, Some(0));
    pool.close().await;
    let _ = fs::remove_dir_all(root);
}

/// A `user_version = 2` database upgrades to 13. Step 6 drops the legacy
/// `contract_*` tables, so the claim/object/evidence/event rows a v2 database
/// held do not survive — they were never going to: nothing has shipped, and
/// `knowledge_items` is the sole store. What the ladder preserves across the
/// upgrade is that the `schema_cache`, `audit_log`, `user_preferences`, and
/// `knowledge_items` tables all exist on the migrated database, and every
/// `contract_*` table is gone.
#[tokio::test]
async fn upgrade_from_version_two_drops_contract_tables_keeps_survivors() {
    let root = temp_root("upgrade-v2");
    let db = root.join("state.sqlite3");
    build_version_two_database(&db).await;
    let store = SqliteStateStore::new(&db);
    store.list_schema_metadata().await.unwrap();
    store.close().await;
    assert_eq!(user_version(&db).await, 14);
    assert!(
        table_exists(&db, "schema_cache").await,
        "schema_cache dropped"
    );
    assert!(table_exists(&db, "audit_log").await, "audit_log dropped");
    assert!(
        table_exists(&db, "user_preferences").await,
        "upgrade did not add user_preferences"
    );
    assert!(
        table_exists(&db, "knowledge_items").await,
        "upgrade did not add knowledge_items"
    );
    // Step 6 dropped the four legacy tables the v2 database built.
    assert_eq!(contract_tables(&db).await.len(), 0);
    let _ = fs::remove_dir_all(root);
}

/// A `user_version = 4` database — the latest before step 5 — upgrades to 13 and
/// gains `knowledge_items`. Step 6 then drops the legacy `contract_*` tables,
/// so the claim a v4 database held is gone too; the surviving guarantee is the
/// knowledge table arriving and the legacy ones leaving.
#[tokio::test]
async fn upgrade_from_version_four_adds_knowledge_items_and_drops_contract_tables() {
    let root = temp_root("upgrade-v4");
    let db = root.join("state.sqlite3");
    build_version_four_database(&db).await;
    let store = SqliteStateStore::new(&db);
    store.list_schema_metadata().await.unwrap();
    store.close().await;
    assert_eq!(user_version(&db).await, 14);
    assert!(
        table_exists(&db, "knowledge_items").await,
        "upgrade did not add knowledge_items"
    );
    // Step 6 dropped the legacy tables the v4 database held.
    assert_eq!(contract_tables(&db).await.len(), 0);
    let _ = fs::remove_dir_all(root);
}

/// A `user_version = 6` database — the latest before step 7 — upgrades to 13
/// and gains the run-spine tables (`runs`, `run_steps`). A v6 database is one
/// that migrated through step 6 before step 7 existed: fully migrated for its
/// ladder, carrying no run tables. The test builds that state by migrating a
/// fresh database and dropping the step-7 tables, which is the same table set
/// a genuine v6 database held, then reopens and asserts the upgrade.
#[tokio::test]
async fn upgrade_from_version_six_adds_run_tables() {
    let root = temp_root("upgrade-v6");
    let db = root.join("state.sqlite3");
    // Migrate to the current head, then rewind to v6 by removing the tables
    // step 7 owns — the exact state a pre-step-7 database is in.
    let store = SqliteStateStore::new(&db);
    store.list_schema_metadata().await.unwrap();
    store.close().await;
    let pool = create_pool(&db).await;
    super::checkout_upgrade::drop_step13_tables(&pool).await;
    sqlx::query("DROP TABLE coding_budget_reservations")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("DROP TABLE coding_events")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("DROP TABLE coding_children")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("DROP TABLE coding_jobs")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("DROP TABLE IF EXISTS run_steps")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("DROP TABLE IF EXISTS runs")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("PRAGMA user_version = 6")
        .execute(&pool)
        .await
        .unwrap();
    pool.close().await;
    assert_eq!(user_version(&db).await, 6);
    assert!(!table_exists(&db, "runs").await, "runs should be absent");
    // The upgrade re-adds the run tables and reaches version 13.
    let reopened = SqliteStateStore::new(&db);
    reopened.list_schema_metadata().await.unwrap();
    reopened.close().await;
    assert_eq!(user_version(&db).await, 14);
    assert!(table_exists(&db, "runs").await, "upgrade did not add runs");
    assert!(
        table_exists(&db, "run_steps").await,
        "upgrade did not add run_steps"
    );
    // Everything the v6 database held survives the upgrade.
    assert!(
        table_exists(&db, "knowledge_items").await,
        "v6 upgrade dropped knowledge_items"
    );
    assert!(
        table_exists(&db, "user_preferences").await,
        "v6 upgrade dropped user_preferences"
    );
    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn upgrade_from_version_nine_preserves_metadata_and_adds_coding_tables() {
    let root = temp_root("upgrade-v9");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    store.list_schema_metadata().await.unwrap();
    store.close().await;
    let pool = create_pool(&db).await;
    sqlx::query("INSERT INTO user_preferences(scope_key, preference_kind, value_json, updated_unix_ms) VALUES ('scope', 'kind', '{}', 7)")
        .execute(&pool)
        .await
        .unwrap();
    super::checkout_upgrade::drop_step13_tables(&pool).await;
    sqlx::query("DROP TABLE coding_budget_reservations")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("DROP TABLE coding_events")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("DROP TABLE coding_children")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("DROP TABLE coding_jobs")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("PRAGMA user_version = 9")
        .execute(&pool)
        .await
        .unwrap();
    pool.close().await;

    let upgraded = SqliteStateStore::new(&db);
    upgraded.list_schema_metadata().await.unwrap();
    upgraded.close().await;
    assert_eq!(user_version(&db).await, 14);
    assert!(table_exists(&db, "coding_jobs").await);
    assert!(table_exists(&db, "coding_children").await);
    let pool = read_pool(&db).await;
    let value: String = sqlx::query_scalar(
        "SELECT value_json FROM user_preferences WHERE scope_key='scope' AND preference_kind='kind'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(value, "{}");
    pool.close().await;
    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn upgrade_from_version_ten_preserves_coding_metadata() {
    let root = temp_root("upgrade-v10");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    store.list_schema_metadata().await.unwrap();
    store.close().await;
    let pool = create_pool(&db).await;
    sqlx::query("INSERT INTO coding_jobs(job_id, repository_id, snapshot_id, state, created_unix_ms, updated_unix_ms) VALUES ('job-v10', 'repository-v10', 'snapshot-v10', 'running', 1, 2)")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO coding_children(job_id, agent_id, current_invocation_id, state, created_unix_ms, updated_unix_ms) VALUES ('job-v10', 'agent-v10', 'invocation-v10', 'running', 1, 2)")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("ALTER TABLE coding_children RENAME TO coding_children_v11")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("CREATE TABLE coding_children(job_id TEXT NOT NULL REFERENCES coding_jobs(job_id) ON DELETE RESTRICT, agent_id TEXT NOT NULL, current_invocation_id TEXT NOT NULL, state TEXT NOT NULL CHECK(state IN ('queued', 'running', 'waiting_for_approval', 'paused', 'completed', 'failed', 'cancelled', 'uncertain')), created_unix_ms INTEGER NOT NULL CHECK(created_unix_ms >= 0), updated_unix_ms INTEGER NOT NULL CHECK(updated_unix_ms >= created_unix_ms), PRIMARY KEY(job_id, agent_id), UNIQUE(job_id, current_invocation_id))")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO coding_children(job_id, agent_id, current_invocation_id, state, created_unix_ms, updated_unix_ms) SELECT job_id, agent_id, current_invocation_id, state, created_unix_ms, updated_unix_ms FROM coding_children_v11")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("DROP TABLE coding_children_v11")
        .execute(&pool)
        .await
        .unwrap();
    super::checkout_upgrade::drop_step13_tables(&pool).await;
    sqlx::query("DROP TABLE coding_budget_reservations")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("DROP TABLE coding_events")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("PRAGMA user_version = 10")
        .execute(&pool)
        .await
        .unwrap();
    pool.close().await;

    let upgraded = SqliteStateStore::new(&db);
    let job_id = saya_types::JobId::parse("job-v10").unwrap();
    assert_eq!(
        upgraded
            .get_coding_job(&job_id)
            .await
            .unwrap()
            .unwrap()
            .state,
        saya_types::JobState::Running
    );
    let child = upgraded
        .list_coding_children(&job_id, None, 1)
        .await
        .unwrap()
        .items
        .pop()
        .unwrap();
    assert_eq!(child.result_source_sha256, None);
    assert_eq!(child.result_outcome, None);
    upgraded.close().await;
    assert_eq!(user_version(&db).await, 14);
    assert!(table_exists(&db, "coding_events").await);
    let _ = fs::remove_dir_all(root);
}
