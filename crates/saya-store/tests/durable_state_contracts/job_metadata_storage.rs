use saya_store::{CodingJobRecord, NewCodingChild, NewCodingJob, SqliteStateStore, StoreError};
use saya_types::{AgentId, InvocationId, JobId, JobState, RepositoryId, SnapshotId};
use sqlx::{
    SqlitePool,
    sqlite::{SqliteConnectOptions, SqlitePoolOptions},
};
use std::{
    fs,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

pub(super) fn temp_root(label: &str) -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "saya-primary-coding-{label}-{}-{stamp}",
        std::process::id()
    ));
    fs::create_dir_all(&root).unwrap();
    root
}

fn job(tag: &str) -> NewCodingJob {
    NewCodingJob {
        job_id: JobId::parse(&format!("job-{tag}")).unwrap(),
        repository_id: RepositoryId::parse(&format!("repository-{tag}")).unwrap(),
        snapshot_id: SnapshotId::parse(&format!("snapshot-{tag}")).unwrap(),
    }
}

pub(super) fn child(job_id: JobId, tag: &str) -> NewCodingChild {
    NewCodingChild {
        job_id,
        agent_id: AgentId::parse(&format!("agent-{tag}")).unwrap(),
        current_invocation_id: InvocationId::parse(&format!("invocation-{tag}")).unwrap(),
    }
}

pub(super) async fn create_job(store: &SqliteStateStore, tag: &str) -> CodingJobRecord {
    store.create_coding_job(job(tag)).await.unwrap()
}

async fn read_pool(db: &std::path::Path) -> SqlitePool {
    SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(SqliteConnectOptions::new().filename(db))
        .await
        .unwrap()
}

#[tokio::test]
async fn coding_metadata_round_trips_and_reopens() {
    let root = temp_root("reopen");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    let created = create_job(&store, "reopen").await;
    let created_child = store
        .create_coding_child(child(created.job_id.clone(), "reopen"))
        .await
        .unwrap();
    assert_eq!(created.state, JobState::Queued);
    assert_eq!(created.created_unix_ms, created.updated_unix_ms);
    assert!(created.created_unix_ms >= 0);
    assert_eq!(created_child.state, JobState::Queued);
    assert_eq!(created_child.created_unix_ms, created_child.updated_unix_ms);
    store.close().await;

    let reopened = SqliteStateStore::new(&db);
    assert_eq!(
        reopened.get_coding_job(&created.job_id).await.unwrap(),
        Some(created)
    );
    let page = reopened
        .list_coding_children(&created_child.job_id, None, 1)
        .await
        .unwrap();
    assert_eq!(page.items, vec![created_child]);
    assert_eq!(page.next_cursor, None);
    reopened.close().await;
    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn coding_metadata_refuses_conflicts_and_orphans() {
    let root = temp_root("refusals");
    let store = SqliteStateStore::new(root.join("state.sqlite3"));
    let record = create_job(&store, "refusals").await;
    assert_eq!(
        store.create_coding_job(job("refusals")).await.unwrap_err(),
        StoreError::Conflict
    );
    store
        .create_coding_child(child(record.job_id.clone(), "first"))
        .await
        .unwrap();
    assert_eq!(
        store
            .create_coding_child(NewCodingChild {
                job_id: record.job_id.clone(),
                agent_id: AgentId::parse("agent-first").unwrap(),
                current_invocation_id: InvocationId::parse("invocation-second").unwrap(),
            })
            .await
            .unwrap_err(),
        StoreError::Conflict
    );
    assert_eq!(
        store
            .create_coding_child(NewCodingChild {
                job_id: record.job_id.clone(),
                agent_id: AgentId::parse("agent-second").unwrap(),
                current_invocation_id: InvocationId::parse("invocation-first").unwrap(),
            })
            .await
            .unwrap_err(),
        StoreError::Conflict
    );
    assert_eq!(
        store
            .create_coding_child(child(JobId::parse("job-missing").unwrap(), "orphan"))
            .await
            .unwrap_err(),
        StoreError::NotFound
    );
    store.close().await;
    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn malformed_stored_metadata_is_refused_on_read() {
    let root = temp_root("malformed");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    let record = create_job(&store, "malformed").await;
    store.close().await;
    let pool = read_pool(&db).await;
    sqlx::query("UPDATE coding_jobs SET repository_id='not a valid id' WHERE job_id=?")
        .bind(record.job_id.as_str())
        .execute(&pool)
        .await
        .unwrap();
    pool.close().await;

    let reopened = SqliteStateStore::new(&db);
    assert_eq!(
        reopened.get_coding_job(&record.job_id).await.unwrap_err(),
        StoreError::Invalid
    );
    reopened.close().await;
    let _ = fs::remove_dir_all(root);
}
