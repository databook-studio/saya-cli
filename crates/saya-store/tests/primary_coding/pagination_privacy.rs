use super::r01::{child, create_job, temp_root};
use saya_store::{SqliteStateStore, StoreError};
use sqlx::{
    SqlitePool,
    sqlite::{SqliteConnectOptions, SqlitePoolOptions},
};
use std::{fs, path::Path};

const JOB_COLUMNS: &[&str] = &[
    "job_id",
    "repository_id",
    "snapshot_id",
    "state",
    "created_unix_ms",
    "updated_unix_ms",
];
const CHILD_COLUMNS: &[&str] = &[
    "job_id",
    "agent_id",
    "current_invocation_id",
    "state",
    "created_unix_ms",
    "updated_unix_ms",
];

async fn read_pool(db: &Path) -> SqlitePool {
    SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(SqliteConnectOptions::new().filename(db))
        .await
        .unwrap()
}

async fn columns(db: &Path, table: &str) -> Vec<String> {
    let pool = read_pool(db).await;
    let query = format!("SELECT name FROM pragma_table_info('{table}') ORDER BY cid");
    let columns = sqlx::query_scalar(&query).fetch_all(&pool).await.unwrap();
    pool.close().await;
    columns
}

#[tokio::test]
async fn coding_children_page_without_a_lifetime_cap() {
    let root = temp_root("page");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    let record = create_job(&store, "page").await;
    for tag in ["01", "02", "03", "04", "05"] {
        store
            .create_coding_child(child(record.job_id.clone(), tag))
            .await
            .unwrap();
    }
    let first = store
        .list_coding_children(&record.job_id, None, 2)
        .await
        .unwrap();
    let second = store
        .list_coding_children(&record.job_id, first.next_cursor.as_ref(), 2)
        .await
        .unwrap();
    let third = store
        .list_coding_children(&record.job_id, second.next_cursor.as_ref(), 2)
        .await
        .unwrap();
    let agents: Vec<_> = first
        .items
        .iter()
        .chain(&second.items)
        .chain(&third.items)
        .map(|item| item.agent_id.as_str())
        .collect();
    assert_eq!(
        agents,
        ["agent-01", "agent-02", "agent-03", "agent-04", "agent-05"]
    );
    assert_eq!(third.next_cursor, None);
    assert_eq!(
        store
            .list_coding_children(&record.job_id, None, 0)
            .await
            .unwrap_err(),
        StoreError::Invalid
    );
    assert_eq!(
        store
            .list_coding_children(&record.job_id, None, 101)
            .await
            .unwrap_err(),
        StoreError::LimitExceeded
    );
    store.close().await;
    assert_eq!(
        columns(&db, "coding_jobs").await,
        JOB_COLUMNS
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
    );
    assert_eq!(
        columns(&db, "coding_children").await,
        CHILD_COLUMNS
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
    );
    let _ = fs::remove_dir_all(root);
}
