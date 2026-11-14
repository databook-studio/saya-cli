use super::*;

#[tokio::test]
async fn run_lifecycle_walks_the_state_machine() {
    let root = temp_root("lifecycle");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    let id = run_id("lifecycle");
    store
        .create_run(NewRun {
            id: id.clone(),
            capabilities: flags(),
            budgets: budgets(),
        })
        .await
        .unwrap();
    for status in [
        RunStatus::Approved,
        RunStatus::Executing,
        RunStatus::Paused,
        RunStatus::Executing,
        RunStatus::Completed,
    ] {
        store.set_run_status(&id, status, None).await.unwrap();
        let record = store.get_run(&id).await.unwrap().unwrap();
        assert_eq!(
            record.status, status,
            "transition to {status:?} did not hold"
        );
    }
    store.close().await;
    let _ = fs::remove_dir_all(root);
}
#[tokio::test]
async fn illegal_transitions_refuse_with_conflict() {
    let root = temp_root("illegal");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    let id = run_id("illegal");
    store
        .create_run(NewRun {
            id: id.clone(),
            capabilities: flags(),
            budgets: budgets(),
        })
        .await
        .unwrap();
    // No implicit approval: planned → executing skips the gate.
    let error = store
        .set_run_status(&id, RunStatus::Executing, None)
        .await
        .unwrap_err();
    assert_eq!(error, StoreError::Conflict);
    store
        .set_run_status(&id, RunStatus::Cancelled, None)
        .await
        .unwrap();
    // A terminal status never leaves.
    for status in [RunStatus::Executing, RunStatus::Paused, RunStatus::Failed] {
        let error = store.set_run_status(&id, status, None).await.unwrap_err();
        assert_eq!(
            error,
            StoreError::Conflict,
            "{status:?} left a cancelled run"
        );
    }
    store.close().await;
    let _ = fs::remove_dir_all(root);
}
#[tokio::test]
async fn terminal_failure_carries_a_typed_code() {
    let root = temp_root("failure");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    let id = run_id("failure");
    store
        .create_run(NewRun {
            id: id.clone(),
            capabilities: flags(),
            budgets: budgets(),
        })
        .await
        .unwrap();
    // Walk to executing: only a run that is executing can fail.
    store
        .set_run_status(&id, RunStatus::Approved, None)
        .await
        .unwrap();
    store
        .set_run_status(&id, RunStatus::Executing, None)
        .await
        .unwrap();
    // A failure without its typed cause is not a terminal state.
    let error = store
        .set_run_status(&id, RunStatus::Failed, None)
        .await
        .unwrap_err();
    assert_eq!(error, StoreError::Invalid);
    // A code on a non-failure state is meaningless, even on a legal transition.
    let error = store
        .set_run_status(&id, RunStatus::Paused, Some(RunFailureCode::Provider))
        .await
        .unwrap_err();
    assert_eq!(error, StoreError::Invalid);
    store
        .set_run_status(&id, RunStatus::Paused, None)
        .await
        .unwrap();
    store
        .set_run_status(&id, RunStatus::Executing, None)
        .await
        .unwrap();
    store
        .set_run_status(&id, RunStatus::Failed, Some(RunFailureCode::SafetyQuery))
        .await
        .unwrap();
    let record = store.get_run(&id).await.unwrap().unwrap();
    assert_eq!(record.status, RunStatus::Failed);
    assert_eq!(record.failure_code, Some(RunFailureCode::SafetyQuery));
    store.close().await;
    let _ = fs::remove_dir_all(root);
}
