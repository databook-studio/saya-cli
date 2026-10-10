use super::*;
use saya_store::{NewCodingChild, NewCodingJob};
use saya_types::{
    AgentId, CodingEventKind, CodingEventV1, InvocationId, JobId, JobState, RepositoryId,
    SnapshotId,
};
use std::num::NonZeroU64;

#[tokio::test]
async fn upgrade_from_eleven_preserves_coding_events_and_child_metadata() {
    let root = temp_root("upgrade-v11-budget");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    let job_id = JobId::parse("job-v11-budget").unwrap();
    store
        .create_coding_job(NewCodingJob {
            job_id: job_id.clone(),
            repository_id: RepositoryId::parse("repository-v11-budget").unwrap(),
            snapshot_id: SnapshotId::parse("snapshot-v11-budget").unwrap(),
        })
        .await
        .unwrap();
    let child = store
        .create_coding_child(NewCodingChild {
            job_id: job_id.clone(),
            agent_id: AgentId::parse("agent-v11-budget").unwrap(),
            current_invocation_id: InvocationId::parse("invocation-v11-budget").unwrap(),
        })
        .await
        .unwrap();
    let running = CodingEventV1::new(
        NonZeroU64::new(1).unwrap(),
        job_id.clone(),
        None,
        None,
        CodingEventKind::StateChanged {
            state: JobState::Running,
        },
    )
    .unwrap();
    store.append_coding_event(running.clone()).await.unwrap();
    store.close().await;
    let raw = create_pool(&db).await;
    checkout_upgrade::drop_step13_tables(&raw).await;
    sqlx::query("DROP TABLE coding_budget_reservations")
        .execute(&raw)
        .await
        .unwrap();
    sqlx::query("PRAGMA user_version = 11")
        .execute(&raw)
        .await
        .unwrap();
    raw.close().await;

    let upgraded = SqliteStateStore::new(&db);
    assert_eq!(
        upgraded
            .get_coding_job(&job_id)
            .await
            .unwrap()
            .unwrap()
            .state,
        JobState::Running
    );
    assert_eq!(
        upgraded
            .list_coding_children(&job_id, None, 1)
            .await
            .unwrap()
            .items,
        vec![child]
    );
    assert_eq!(
        upgraded
            .list_coding_events(&job_id, None, 1)
            .await
            .unwrap()
            .events,
        vec![running]
    );
    upgraded.close().await;
    assert_eq!(user_version(&db).await, 14);
    assert!(table_exists(&db, "coding_budget_reservations").await);
    let _ = fs::remove_dir_all(root);
}
