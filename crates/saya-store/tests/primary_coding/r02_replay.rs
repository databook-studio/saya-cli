use super::{
    r01::{child, create_job, temp_root},
    r02::event,
};
use saya_store::{SqliteStateStore, StoreError};
use saya_types::{
    CodingEventKind, CodingEventV1, CodingResultOutcome, InvocationId, JobState, Sha256Digest,
};
use sqlx::{
    SqlitePool,
    sqlite::{SqliteConnectOptions, SqlitePoolOptions},
};
use std::{fs, num::NonZeroU64, path::Path};

#[path = "r02_replay/rollback.rs"]
mod rollback;

async fn pool(db: &Path) -> SqlitePool {
    SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(SqliteConnectOptions::new().filename(db))
        .await
        .unwrap()
}

#[tokio::test]
async fn ordinary_events_refuse_stale_terminal_and_approval_resume_paths() {
    let root = temp_root("r02-policy");
    let store = SqliteStateStore::new(root.join("state.sqlite3"));
    let job = create_job(&store, "r02-policy").await;
    let child = store
        .create_coding_child(child(job.job_id.clone(), "r02-policy"))
        .await
        .unwrap();
    store
        .append_coding_event(event(
            1,
            &job.job_id,
            None,
            CodingEventKind::StateChanged {
                state: JobState::Running,
            },
        ))
        .await
        .unwrap();
    let stale = CodingEventV1::new(
        NonZeroU64::new(2).unwrap(),
        job.job_id.clone(),
        Some(child.agent_id.clone()),
        Some(InvocationId::parse("invocation-stale").unwrap()),
        CodingEventKind::StateChanged {
            state: JobState::Running,
        },
    )
    .unwrap();
    assert_eq!(
        store.append_coding_event(stale).await.unwrap_err(),
        StoreError::Conflict
    );
    store
        .append_coding_event(event(
            2,
            &job.job_id,
            Some(&child),
            CodingEventKind::StateChanged {
                state: JobState::Running,
            },
        ))
        .await
        .unwrap();
    store
        .append_coding_event(event(
            3,
            &job.job_id,
            Some(&child),
            CodingEventKind::ResultReported {
                source_sha256: Sha256Digest::parse(&"b".repeat(64)).unwrap(),
                outcome: CodingResultOutcome::Failed,
            },
        ))
        .await
        .unwrap();
    assert_eq!(
        store
            .append_coding_event(event(
                4,
                &job.job_id,
                Some(&child),
                CodingEventKind::ResultReported {
                    source_sha256: Sha256Digest::parse(&"c".repeat(64)).unwrap(),
                    outcome: CodingResultOutcome::Failed,
                },
            ))
            .await
            .unwrap_err(),
        StoreError::Conflict
    );
    store
        .append_coding_event(event(
            4,
            &job.job_id,
            None,
            CodingEventKind::StateChanged {
                state: JobState::WaitingForApproval,
            },
        ))
        .await
        .unwrap();
    assert_eq!(
        store
            .append_coding_event(event(
                5,
                &job.job_id,
                None,
                CodingEventKind::StateChanged {
                    state: JobState::Queued,
                },
            ))
            .await
            .unwrap_err(),
        StoreError::Conflict
    );
    store
        .append_coding_event(event(
            5,
            &job.job_id,
            None,
            CodingEventKind::StateChanged {
                state: JobState::Cancelled,
            },
        ))
        .await
        .unwrap();
    assert_eq!(
        store
            .append_coding_event(event(
                6,
                &job.job_id,
                Some(&child),
                CodingEventKind::StateChanged {
                    state: JobState::Completed,
                },
            ))
            .await
            .unwrap_err(),
        StoreError::Conflict
    );
    store.close().await;
    let _ = fs::remove_dir_all(root);
}
