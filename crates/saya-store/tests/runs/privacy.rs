use super::*;

#[tokio::test]
async fn credential_shaped_endpoint_keys_are_refused_and_never_stored() {
    let root = temp_root("admission");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    // The clean run first, so the database and its sidecars exist and the
    // byte scan below has real bytes to scan.
    store
        .create_run(NewRun {
            id: run_id("clean"),
            capabilities: flags(),
            budgets: budgets(),
        })
        .await
        .unwrap();
    let mut secret_budgets = budgets();
    secret_budgets
        .tokens_per_endpoint
        .insert("api_key=sk-live-SENTINELKEY".to_owned(), 1_000);
    let error = store
        .create_run(NewRun {
            id: run_id("admission"),
            capabilities: flags(),
            budgets: secret_budgets,
        })
        .await
        .unwrap_err();
    assert_eq!(error, StoreError::Invalid);
    // The refusal happened before any INSERT: the sentinel is in no byte.
    assert_absent("run store", &db_bytes(&db), "SENTINELKEY");
    store.close().await;
    let _ = fs::remove_dir_all(root);
}
#[tokio::test]
async fn unshaped_endpoint_keys_and_overwide_maps_are_refused() {
    let root = temp_root("keys");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    let mut control_budgets = budgets();
    control_budgets
        .tokens_per_endpoint
        .insert("bad\u{7}key".to_owned(), 1);
    let error = store
        .create_run(NewRun {
            id: run_id("control"),
            capabilities: flags(),
            budgets: control_budgets,
        })
        .await
        .unwrap_err();
    assert_eq!(error, StoreError::Invalid);
    let mut wide_budgets = RunBudgets::default();
    for index in 0..9 {
        wide_budgets
            .tokens_per_endpoint
            .insert(format!("endpoint-{index}"), 1);
    }
    let error = store
        .create_run(NewRun {
            id: run_id("wide"),
            capabilities: flags(),
            budgets: wide_budgets,
        })
        .await
        .unwrap_err();
    // A map over the endpoint bound exceeds a store limit, not a shape fault.
    assert_eq!(error, StoreError::LimitExceeded);
    store.close().await;
    let _ = fs::remove_dir_all(root);
}
#[tokio::test]
async fn goal_sentinel_never_reaches_the_bytes() {
    let root = temp_root("sentinel");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    // Drive the full flow the engine will drive. The goal below carries the
    // sentinels; the API exposes no parameter it could travel through.
    let goal = GOAL_SENTINELS.join(" and ");
    assert!(!goal.is_empty());
    let id = run_id("sentinel");
    store
        .create_run(NewRun {
            id: id.clone(),
            capabilities: flags(),
            budgets: budgets(),
        })
        .await
        .unwrap();
    store
        .set_run_status(&id, RunStatus::Approved, None)
        .await
        .unwrap();
    store
        .set_run_status(&id, RunStatus::Executing, None)
        .await
        .unwrap();
    store
        .upsert_step(&id, 0, RunStepStatus::Running)
        .await
        .unwrap();
    store.set_step_usage(&id, 0, usage_known()).await.unwrap();
    store
        .upsert_step(&id, 0, RunStepStatus::Done)
        .await
        .unwrap();
    store.set_run_usage(&id, usage_known()).await.unwrap();
    store
        .set_run_status(&id, RunStatus::Completed, None)
        .await
        .unwrap();
    // The flow really wrote rows — the scan below is not vacuous.
    let record = store.get_run(&id).await.unwrap().unwrap();
    assert_eq!(record.status, RunStatus::Completed);
    let bytes = db_bytes(&db);
    for sentinel in GOAL_SENTINELS {
        assert_absent("run store", &bytes, sentinel);
    }
    store.close().await;
    let _ = fs::remove_dir_all(root);
}
#[tokio::test]
async fn run_schema_stays_metadata_only() {
    let root = temp_root("schema");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    store.list_schema_metadata().await.unwrap();
    store.close().await;
    for (table, allowlist) in [("runs", RUNS_COLUMNS), ("run_steps", RUN_STEPS_COLUMNS)] {
        let pool = read_pool(&db).await;
        let mut columns: Vec<String> =
            sqlx::query_scalar("SELECT name FROM pragma_table_info(?) ORDER BY name")
                .bind(table)
                .fetch_all(&pool)
                .await
                .unwrap();
        pool.close().await;
        let mut expected: Vec<&str> = allowlist.to_vec();
        expected.sort_unstable();
        columns.sort();
        assert_eq!(
            columns, expected,
            "{table} gained a column outside the metadata allowlist"
        );
    }
    let _ = fs::remove_dir_all(root);
}
