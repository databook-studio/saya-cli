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

#[tokio::test]
async fn failure_rolls_back_and_replay_refuses_invalid_stored_events() {
    let root = temp_root("r02-replay");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    let job = create_job(&store, "r02-replay").await;
    store.close().await;
    let raw = pool(&db).await;
    sqlx::query("CREATE TRIGGER reject_job_update BEFORE UPDATE ON coding_jobs BEGIN SELECT RAISE(ABORT, 'reject'); END")
        .execute(&raw)
        .await
        .unwrap();
    raw.close().await;
    let rejected = SqliteStateStore::new(&db);
    assert_eq!(
        rejected
            .append_coding_event(event(
                1,
                &job.job_id,
                None,
                CodingEventKind::StateChanged {
                    state: JobState::Running,
                },
            ))
            .await
            .unwrap_err(),
        StoreError::Unavailable
    );
    rejected.close().await;
    let raw = pool(&db).await;
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM coding_events")
        .fetch_one(&raw)
        .await
        .unwrap();
    let state: String = sqlx::query_scalar("SELECT state FROM coding_jobs WHERE job_id=?")
        .bind(job.job_id.as_str())
        .fetch_one(&raw)
        .await
        .unwrap();
    assert_eq!(count, 0);
    assert_eq!(state, "queued");
    sqlx::query("DROP TRIGGER reject_job_update")
        .execute(&raw)
        .await
        .unwrap();
    raw.close().await;
    let accepted = SqliteStateStore::new(&db);
    accepted
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
    accepted.close().await;

    let raw = pool(&db).await;
    sqlx::query(
        "INSERT INTO coding_events(job_id, sequence, event_json) VALUES (?, 2, 'not-json')",
    )
    .bind(job.job_id.as_str())
    .execute(&raw)
    .await
    .unwrap();
    raw.close().await;
    let replay = SqliteStateStore::new(&db);
    assert_eq!(
        replay
            .list_coding_events(&job.job_id, None, 100)
            .await
            .unwrap_err(),
        StoreError::Invalid
    );
    replay.close().await;

    let raw = pool(&db).await;
    sqlx::query("DELETE FROM coding_events WHERE sequence=2")
        .execute(&raw)
        .await
        .unwrap();
    let mismatched = event(
        3,
        &job.job_id,
        None,
        CodingEventKind::StateChanged {
            state: JobState::Failed,
        },
    );
    sqlx::query("INSERT INTO coding_events(job_id, sequence, event_json) VALUES (?, 2, ?)")
        .bind(job.job_id.as_str())
        .bind(String::from_utf8(mismatched.to_json_bytes().unwrap()).unwrap())
        .execute(&raw)
        .await
        .unwrap();
    raw.close().await;
    let replay = SqliteStateStore::new(&db);
    assert_eq!(
        replay
            .list_coding_events(&job.job_id, None, 100)
            .await
            .unwrap_err(),
        StoreError::Invalid
    );
    replay.close().await;

    let raw = pool(&db).await;
    sqlx::query("DELETE FROM coding_events WHERE sequence=2")
        .execute(&raw)
        .await
        .unwrap();
    let future = String::from_utf8(
        event(
            2,
            &job.job_id,
            None,
            CodingEventKind::StateChanged {
                state: JobState::Failed,
            },
        )
        .to_json_bytes()
        .unwrap(),
    )
    .unwrap()
    .replace("\"version\":1", "\"version\":2");
    sqlx::query("INSERT INTO coding_events(job_id, sequence, event_json) VALUES (?, 2, ?)")
        .bind(job.job_id.as_str())
        .bind(future)
        .execute(&raw)
        .await
        .unwrap();
    assert!(
        sqlx::query("INSERT INTO coding_events(job_id, sequence, event_json) VALUES (?, 3, ?)")
            .bind(job.job_id.as_str())
            .bind("x".repeat(65_537))
            .execute(&raw)
            .await
            .is_err()
    );
    raw.close().await;
    let replay = SqliteStateStore::new(&db);
    assert_eq!(
        replay
            .list_coding_events(&job.job_id, None, 100)
            .await
            .unwrap_err(),
        StoreError::VersionUnsupported
    );
    replay.close().await;
    let _ = fs::remove_dir_all(root);
}
