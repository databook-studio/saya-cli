use super::*;

#[tokio::test]
async fn usage_is_null_when_unknown_and_never_zero() {
    let root = temp_root("usage");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    let id = run_id("usage");
    store
        .create_run(NewRun {
            id: id.clone(),
            capabilities: flags(),
            budgets: budgets(),
        })
        .await
        .unwrap();
    store
        .upsert_step(&id, 0, RunStepStatus::Running)
        .await
        .unwrap();
    // Unknown usage at the API, and literally NULL in the row: absence must
    // never be read back as zero.
    let record = store.get_run(&id).await.unwrap().unwrap();
    assert_eq!(record.usage, RunUsage::unknown());
    {
        let pool = read_pool(&db).await;
        let turns: Option<i64> = sqlx::query_scalar("SELECT usage_turns FROM runs WHERE id = ?")
            .bind(id.as_str())
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(turns, None, "unknown usage was stored as a number");
        let step_turns: Option<i64> =
            sqlx::query_scalar("SELECT usage_turns FROM run_steps WHERE run_id=? AND step=0")
                .bind(id.as_str())
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(
            step_turns, None,
            "unknown step usage was stored as a number"
        );
        pool.close().await;
    }
    // Reporting unknown again — an explicitly unreported figure for the
    // primary endpoint — keeps the figures unknown, not zero.
    store.set_run_usage(&id, usage_unknown()).await.unwrap();
    let record = store.get_run(&id).await.unwrap().unwrap();
    assert_eq!(record.usage, usage_unknown());
    // A known figure round-trips through the API and the bytes.
    store.set_run_usage(&id, usage_known()).await.unwrap();
    let record = store.get_run(&id).await.unwrap().unwrap();
    assert_eq!(record.usage, usage_known());
    store.set_step_usage(&id, 0, usage_known()).await.unwrap();
    let steps = store.list_steps(&id).await.unwrap();
    assert_eq!(steps[0].usage, usage_known());
    store.close().await;
    let _ = fs::remove_dir_all(root);
}
#[tokio::test]
async fn usage_for_missing_records_is_not_found() {
    let root = temp_root("usage-missing");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    let error = store
        .set_run_usage(&run_id("ghost"), usage_known())
        .await
        .unwrap_err();
    assert_eq!(error, StoreError::NotFound);
    let error = store
        .set_run_status(&run_id("ghost"), RunStatus::Approved, None)
        .await
        .unwrap_err();
    assert_eq!(error, StoreError::NotFound);
    store.close().await;
    let _ = fs::remove_dir_all(root);
}
