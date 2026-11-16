use super::*;

#[tokio::test]
async fn steps_walk_pending_running_done_and_bounded_retry() {
    let root = temp_root("steps");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    let id = run_id("steps");
    store
        .create_run(NewRun {
            id: id.clone(),
            capabilities: flags(),
            budgets: budgets(),
        })
        .await
        .unwrap();
    // A step is born pending or running, never already done.
    let error = store
        .upsert_step(&id, 0, RunStepStatus::Done)
        .await
        .unwrap_err();
    assert_eq!(error, StoreError::Invalid);
    store
        .upsert_step(&id, 0, RunStepStatus::Pending)
        .await
        .unwrap();
    store
        .upsert_step(&id, 0, RunStepStatus::Running)
        .await
        .unwrap();
    store
        .upsert_step(&id, 0, RunStepStatus::Done)
        .await
        .unwrap();
    // A failed step may restart (the engine's bounded retry).
    store
        .upsert_step(&id, 1, RunStepStatus::Running)
        .await
        .unwrap();
    store
        .upsert_step(&id, 1, RunStepStatus::Failed)
        .await
        .unwrap();
    store
        .upsert_step(&id, 1, RunStepStatus::Running)
        .await
        .unwrap();
    let steps = store.list_steps(&id).await.unwrap();
    assert_eq!(steps.len(), 2);
    assert_eq!(steps[0].status, RunStepStatus::Done);
    assert_eq!(steps[1].status, RunStepStatus::Running);
    store.close().await;
    let _ = fs::remove_dir_all(root);
}
#[tokio::test]
async fn steps_of_an_unknown_run_are_not_found() {
    let root = temp_root("orphan-step");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    let error = store
        .upsert_step(&run_id("ghost"), 0, RunStepStatus::Pending)
        .await
        .unwrap_err();
    assert_eq!(error, StoreError::NotFound);
    let steps = store.list_steps(&run_id("ghost")).await.unwrap();
    assert!(steps.is_empty());
    store.close().await;
    let _ = fs::remove_dir_all(root);
}
