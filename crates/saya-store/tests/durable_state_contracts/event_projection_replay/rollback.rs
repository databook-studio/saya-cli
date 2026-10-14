use std::fs;

use saya_store::{SqliteStateStore, StoreError};
use saya_types::{CodingEventKind, JobState};

use crate::event_projection_storage::event;
use crate::job_metadata_storage::{create_job, temp_root};

use super::pool;

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
