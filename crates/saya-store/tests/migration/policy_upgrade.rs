//! The step-15 upgrade proof: a `user_version = 14` database — the latest
//! before step 15 — gains an empty `coding_budget_policies` table on its way
//! to the current version and keeps every pre-existing job row.

use super::*;

/// A `user_version = 14` database upgrades to the current version (17),
/// gaining the `coding_budget_policies` table with no rows in it, and
/// preserves the job rows the v14 database already held. Built by migrating a
/// fresh database, inserting one job, then dropping the step-15 table and
/// rewinding the version — the exact table set a pre-step-15 database holds.
#[tokio::test]
async fn upgrade_from_version_fourteen_reaches_fifteen_with_policy_table() {
    let root = temp_root("upgrade-v14-policy");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    store.list_schema_metadata().await.unwrap();
    store.close().await;

    let pool = create_pool(&db).await;
    sqlx::query("INSERT INTO coding_jobs(job_id, repository_id, snapshot_id, state, created_unix_ms, updated_unix_ms) VALUES ('job-v14', 'repository-v14', 'snapshot-v14', 'running', 1, 2)")
        .execute(&pool)
        .await
        .unwrap();
    super::checkout_upgrade::drop_step16_table(&pool).await;
    super::checkout_upgrade::drop_step15_table(&pool).await;
    sqlx::query("PRAGMA user_version = 14")
        .execute(&pool)
        .await
        .unwrap();
    pool.close().await;
    assert_eq!(user_version(&db).await, 14);
    assert!(
        !table_exists(&db, "coding_budget_policies").await,
        "fixture must start without the step-15 table"
    );

    let upgraded = SqliteStateStore::new(&db);
    let job_id = saya_types::JobId::parse("job-v14").unwrap();
    assert_eq!(
        upgraded
            .get_coding_job(&job_id)
            .await
            .unwrap()
            .unwrap()
            .state,
        saya_types::JobState::Running
    );
    upgraded.close().await;
    assert_eq!(user_version(&db).await, 19);
    assert!(table_exists(&db, "coding_budget_policies").await);
    let pool = read_pool(&db).await;
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM coding_budget_policies")
        .fetch_one(&pool)
        .await
        .unwrap();
    pool.close().await;
    assert_eq!(count, 0, "step 15 creates the table empty");
    let _ = fs::remove_dir_all(root);
}
