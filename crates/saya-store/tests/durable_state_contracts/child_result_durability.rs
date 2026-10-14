use super::{
    event_projection_storage::event,
    job_metadata_storage::{child, create_job, temp_root},
};
use saya_store::{CodingEventAppend, SqliteStateStore, StoreError};
use saya_types::{CodingEventKind, CodingResultOutcome, JobState, JoinedResultV1};
use std::{fs, num::NonZeroU64};

#[tokio::test]
async fn completion_replay_ack_is_idempotent() {
    let root = temp_root("joined-replay");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    let job = create_job(&store, "joined").await;
    let child = store
        .create_coding_child(child(job.job_id.clone(), "joined"))
        .await
        .unwrap();
    for (sequence, target) in [(1, None), (2, Some(&child))] {
        store
            .append_coding_event(event(
                sequence,
                &job.job_id,
                target,
                CodingEventKind::StateChanged {
                    state: JobState::Running,
                },
            ))
            .await
            .unwrap();
    }
    let result = JoinedResultV1::new(
        job.job_id.clone(),
        child.agent_id.clone(),
        child.current_invocation_id.clone(),
        CodingResultOutcome::Completed,
        "observed child answer".into(),
        false,
    )
    .unwrap();
    let completion = event(
        3,
        &job.job_id,
        Some(&child),
        CodingEventKind::StateChanged {
            state: JobState::Completed,
        },
    );
    assert_eq!(
        store
            .record_coding_completion(completion.clone(), &result)
            .await
            .unwrap(),
        CodingEventAppend::Appended(NonZeroU64::new(3).unwrap())
    );
    store.close().await;
    let store = SqliteStateStore::new(&db);
    let page = store
        .list_coding_completions(&job.job_id, None, 1)
        .await
        .unwrap();
    assert_eq!(page.items.len(), 1);
    assert_eq!(page.items[0].result, result);
    assert_eq!(page.items[0].sequence.get(), 3);
    assert_eq!(
        store
            .record_coding_completion(completion.clone(), &result)
            .await
            .unwrap(),
        CodingEventAppend::AlreadyPresent(NonZeroU64::new(3).unwrap())
    );
    let different = JoinedResultV1::new(
        job.job_id.clone(),
        child.agent_id.clone(),
        child.current_invocation_id.clone(),
        CodingResultOutcome::Completed,
        "different answer".into(),
        false,
    )
    .unwrap();
    assert_eq!(
        store
            .record_coding_completion(completion, &different)
            .await
            .unwrap_err(),
        StoreError::Conflict
    );
    assert!(
        store
            .ack_coding_completion(&job.job_id, page.items[0].sequence)
            .await
            .unwrap()
    );
    assert!(
        !store
            .ack_coding_completion(&job.job_id, page.items[0].sequence)
            .await
            .unwrap()
    );
    assert!(
        store
            .list_coding_completions(&job.job_id, None, 32)
            .await
            .unwrap()
            .items
            .is_empty()
    );
    assert_eq!(
        store
            .list_coding_children(&job.job_id, None, 1)
            .await
            .unwrap()
            .items[0]
            .state,
        JobState::Completed
    );
    let accepted = store
        .get_coding_completion(&job.job_id, page.items[0].sequence)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        store
            .list_coding_completion_history(&job.job_id, None, 32)
            .await
            .unwrap()
            .items,
        vec![accepted.clone()]
    );
    assert!(accepted.acknowledged);
    assert_eq!(accepted.result.answer(), "observed child answer");
    store.close().await;
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn mismatched_completion_rolls_back_state_and_event() {
    let root = temp_root("joined-conflict");
    let store = SqliteStateStore::new(root.join("state.sqlite3"));
    let job = create_job(&store, "conflict").await;
    let child = store
        .create_coding_child(child(job.job_id.clone(), "conflict"))
        .await
        .unwrap();
    let result = JoinedResultV1::new(
        job.job_id.clone(),
        child.agent_id.clone(),
        child.current_invocation_id.clone(),
        CodingResultOutcome::Cancelled,
        String::new(),
        false,
    )
    .unwrap();
    let wrong = event(
        1,
        &job.job_id,
        Some(&child),
        CodingEventKind::StateChanged {
            state: JobState::Failed,
        },
    );
    assert_eq!(
        store
            .record_coding_completion(wrong, &result)
            .await
            .unwrap_err(),
        StoreError::Invalid
    );
    assert!(
        store
            .list_coding_events(&job.job_id, None, 100)
            .await
            .unwrap()
            .events
            .is_empty()
    );
    assert_eq!(
        store
            .list_coding_children(&job.job_id, None, 1)
            .await
            .unwrap()
            .items[0]
            .state,
        JobState::Queued
    );
    store.close().await;
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn durable_job_list_is_bounded_and_continues_without_duplicates() {
    let root = temp_root("job-list");
    let store = SqliteStateStore::new(root.join("state.sqlite3"));
    let a = create_job(&store, "list-a").await;
    let b = create_job(&store, "list-b").await;
    let c = create_job(&store, "list-c").await;
    let first = store.list_coding_jobs(None, 2).await.unwrap();
    assert_eq!(first.items, vec![a, b]);
    let second = store
        .list_coding_jobs(first.next_cursor.as_ref(), 2)
        .await
        .unwrap();
    assert_eq!(second.items, vec![c]);
    assert!(second.next_cursor.is_none());
    assert_eq!(
        store.list_coding_jobs(None, 101).await.unwrap_err(),
        StoreError::LimitExceeded
    );
    store.close().await;
    fs::remove_dir_all(root).unwrap();
}
