use super::*;

#[tokio::test]
async fn create_run_records_metadata_and_reads_back() {
    let root = temp_root("create");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    let record = store
        .create_run(NewRun {
            id: run_id("create"),
            capabilities: flags(),
            budgets: budgets(),
        })
        .await
        .unwrap();
    assert_eq!(record.status, RunStatus::Planned);
    assert_eq!(record.failure_code, None);
    assert_eq!(record.capabilities, flags());
    assert_eq!(record.budgets, budgets());
    // A fresh run has no usage at all: unknown, never zero.
    assert_eq!(record.usage, RunUsage::unknown());
    let read = store.get_run(&run_id("create")).await.unwrap().unwrap();
    assert_eq!(read.status, RunStatus::Planned);
    assert_eq!(read.budgets.tokens_per_endpoint["primary"], 100_000);
    store.close().await;
    let _ = fs::remove_dir_all(root);
}
#[tokio::test]
async fn creating_the_same_run_twice_conflicts() {
    let root = temp_root("dup");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    let run = NewRun {
        id: run_id("dup"),
        capabilities: flags(),
        budgets: budgets(),
    };
    store.create_run(run.clone()).await.unwrap();
    let error = store.create_run(run).await.unwrap_err();
    assert_eq!(error, StoreError::Conflict);
    store.close().await;
    let _ = fs::remove_dir_all(root);
}
#[tokio::test]
async fn list_runs_returns_recent_first_summaries() {
    let root = temp_root("list");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    for tag in ["a", "b"] {
        store
            .create_run(NewRun {
                id: run_id(tag),
                capabilities: flags(),
                budgets: budgets(),
            })
            .await
            .unwrap();
        store
            .set_run_status(&run_id(tag), RunStatus::Approved, None)
            .await
            .unwrap();
    }
    let runs = store.list_runs().await.unwrap();
    assert_eq!(runs.len(), 2);
    assert_eq!(runs[0].id, run_id("b"));
    assert_eq!(runs[0].status, RunStatus::Approved);
    assert_eq!(runs[1].id, run_id("a"));
    store.close().await;
    let _ = fs::remove_dir_all(root);
}
