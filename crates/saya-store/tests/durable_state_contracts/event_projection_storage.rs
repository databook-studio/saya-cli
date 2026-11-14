use super::job_metadata_storage::{child, create_job, temp_root};
use saya_store::{
    CodingEventAppend, CodingEventPage, MAX_CODING_EVENT_PAGE_SIZE, SqliteStateStore, StoreError,
};
use saya_types::{CodingEventKind, CodingEventV1, CodingResultOutcome, JobState, Sha256Digest};
use std::{fs, num::NonZeroU64, sync::Arc};

pub(super) fn event(
    sequence: u64,
    job: &saya_types::JobId,
    child: Option<&saya_store::CodingChildRecord>,
    kind: CodingEventKind,
) -> CodingEventV1 {
    CodingEventV1::new(
        NonZeroU64::new(sequence).unwrap(),
        job.clone(),
        child.map(|child| child.agent_id.clone()),
        child.map(|child| child.current_invocation_id.clone()),
        kind,
    )
    .unwrap()
}

#[tokio::test]
async fn appending_events_projects_state_and_result() {
    let root = temp_root("r02-append");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    let job = create_job(&store, "r02-append").await;
    let child = store
        .create_coding_child(child(job.job_id.clone(), "r02-append"))
        .await
        .unwrap();
    let job_running = event(
        1,
        &job.job_id,
        None,
        CodingEventKind::StateChanged {
            state: JobState::Running,
        },
    );
    let child_running = event(
        2,
        &job.job_id,
        Some(&child),
        CodingEventKind::StateChanged {
            state: JobState::Running,
        },
    );
    let result = event(
        3,
        &job.job_id,
        Some(&child),
        CodingEventKind::ResultReported {
            source_sha256: Sha256Digest::parse(&"a".repeat(64)).unwrap(),
            outcome: CodingResultOutcome::Completed,
        },
    );

    assert_eq!(
        store
            .append_coding_event(job_running.clone())
            .await
            .unwrap(),
        CodingEventAppend::Appended(job_running.sequence())
    );
    assert_eq!(
        store
            .append_coding_event(child_running.clone())
            .await
            .unwrap(),
        CodingEventAppend::Appended(child_running.sequence())
    );
    assert_eq!(
        store.append_coding_event(result.clone()).await.unwrap(),
        CodingEventAppend::Appended(result.sequence())
    );
    assert_eq!(
        store
            .get_coding_job(&job.job_id)
            .await
            .unwrap()
            .unwrap()
            .state,
        JobState::Running
    );
    let projected = store
        .list_coding_children(&job.job_id, None, 1)
        .await
        .unwrap()
        .items
        .pop()
        .unwrap();
    assert_eq!(projected.state, JobState::Running);
    assert_eq!(
        projected.result_source_sha256,
        Some(Sha256Digest::parse(&"a".repeat(64)).unwrap())
    );
    assert_eq!(
        projected.result_outcome,
        Some(CodingResultOutcome::Completed)
    );
    store.close().await;
    let reopened = SqliteStateStore::new(&db);
    let first = reopened
        .list_coding_events(&job.job_id, None, 1)
        .await
        .unwrap();
    let second = reopened
        .list_coding_events(&job.job_id, first.next_cursor, 100)
        .await
        .unwrap();
    assert_eq!(first.events, vec![job_running]);
    assert_eq!(second.events, vec![child_running, result]);
    assert_eq!(second.next_cursor, None);
    assert_eq!(
        reopened
            .list_coding_events(&job.job_id, None, 0)
            .await
            .unwrap_err(),
        StoreError::Invalid
    );
    assert_eq!(
        reopened
            .list_coding_events(&job.job_id, None, MAX_CODING_EVENT_PAGE_SIZE + 1)
            .await
            .unwrap_err(),
        StoreError::LimitExceeded
    );
    let empty: CodingEventPage = reopened
        .list_coding_events(
            &saya_types::JobId::parse("job-r02-unknown").unwrap(),
            None,
            1,
        )
        .await
        .unwrap();
    assert!(empty.events.is_empty());
    reopened.close().await;
    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn append_refuses_conflicts_and_control_events() {
    let root = temp_root("r02-sequence");
    let store = SqliteStateStore::new(root.join("state.sqlite3"));
    let job = create_job(&store, "r02-sequence").await;
    let running = event(
        1,
        &job.job_id,
        None,
        CodingEventKind::StateChanged {
            state: JobState::Running,
        },
    );
    assert_eq!(
        store.append_coding_event(running.clone()).await.unwrap(),
        CodingEventAppend::Appended(running.sequence())
    );
    assert_eq!(
        store.append_coding_event(running.clone()).await.unwrap(),
        CodingEventAppend::AlreadyPresent(running.sequence())
    );
    for event in [
        event(
            1,
            &job.job_id,
            None,
            CodingEventKind::StateChanged {
                state: JobState::Failed,
            },
        ),
        event(
            3,
            &job.job_id,
            None,
            CodingEventKind::StateChanged {
                state: JobState::Failed,
            },
        ),
    ] {
        assert_eq!(
            store.append_coding_event(event).await.unwrap_err(),
            StoreError::Conflict
        );
    }
    assert_eq!(
        store
            .append_coding_event(event(
                i64::MAX as u64 + 1,
                &job.job_id,
                None,
                CodingEventKind::StateChanged {
                    state: JobState::Failed,
                },
            ))
            .await
            .unwrap_err(),
        StoreError::LimitExceeded
    );
    assert_eq!(
        store
            .append_coding_event(event(
                2,
                &job.job_id,
                None,
                CodingEventKind::ApprovalClosed {
                    request_sequence: NonZeroU64::new(1).unwrap(),
                    disposition: saya_types::ApprovalDisposition::Revoked,
                },
            ))
            .await
            .unwrap_err(),
        StoreError::Invalid
    );
    store.close().await;
    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn racing_next_sequence_has_exactly_one_winner() {
    let root = temp_root("r02-race");
    let db = root.join("state.sqlite3");
    let setup = SqliteStateStore::new(&db);
    let job = create_job(&setup, "r02-race").await;
    setup.close().await;
    let first = Arc::new(SqliteStateStore::new(&db));
    let second = Arc::new(SqliteStateStore::new(&db));
    let running = event(
        1,
        &job.job_id,
        None,
        CodingEventKind::StateChanged {
            state: JobState::Running,
        },
    );
    let failed = event(
        1,
        &job.job_id,
        None,
        CodingEventKind::StateChanged {
            state: JobState::Failed,
        },
    );
    let (one, two) = tokio::join!(
        first.append_coding_event(running),
        second.append_coding_event(failed)
    );
    let results = [one, two];
    assert_eq!(
        results
            .iter()
            .filter(|result| matches!(result, Ok(CodingEventAppend::Appended(_))))
            .count(),
        1
    );
    assert_eq!(
        results
            .iter()
            .filter(|result| **result == Err(StoreError::Conflict))
            .count(),
        1
    );
    first.close().await;
    second.close().await;
    let _ = fs::remove_dir_all(root);
}
