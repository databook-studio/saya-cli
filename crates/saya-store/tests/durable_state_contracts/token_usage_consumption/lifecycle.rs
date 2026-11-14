use super::fixture::*;
use saya_store::{CodingEventAppend, SqliteStateStore, StoreError};
use saya_types::{ApprovalDisposition, CodingEventKind, JobState};
use std::num::NonZeroU64;

#[tokio::test]
async fn pending_request_survives_reopen_as_audit_only_record() {
    let (path, store, job) = running("reopen").await;
    let request = append_request(&store, &job, 2, None, 1, u64::MAX).await;
    store.close().await;
    let reopened = SqliteStateStore::new(path.join("state.sqlite3"));
    let record = reopened
        .get_coding_approval_request(&job, request.request_sequence())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(record.request, request);
    assert!(record.closure.is_none());
    assert_eq!(
        reopened
            .list_coding_approval_requests(&job, None, 10)
            .await
            .unwrap()
            .items,
        vec![record]
    );
    reopened.close().await;
    let _ = std::fs::remove_dir_all(path);
}

#[tokio::test]
async fn exact_request_and_negative_closure_retries_never_reopen_terminal_or_expired_records() {
    let (path, store, job) = running("terminal-retry").await;
    let actor = child(&store, &job, "terminal").await;
    let request = append_request(
        &store,
        &job,
        2,
        Some((&actor.0, &actor.1)),
        1,
        now().saturating_sub(1),
    )
    .await;
    store
        .append_coding_event(event(
            3,
            &job,
            None,
            CodingEventKind::StateChanged {
                state: JobState::Completed,
            },
        ))
        .await
        .unwrap();
    assert_eq!(
        store
            .record_coding_approval_request(request_event(&request))
            .await
            .unwrap(),
        CodingEventAppend::AlreadyPresent(request.request_sequence())
    );
    let denied = event(
        4,
        &job,
        Some((&actor.0, &actor.1)),
        CodingEventKind::ApprovalClosed {
            request_sequence: request.request_sequence(),
            disposition: ApprovalDisposition::Denied,
        },
    );
    assert_eq!(
        store.close_coding_approval(denied.clone()).await.unwrap(),
        CodingEventAppend::Appended(NonZeroU64::new(4).unwrap())
    );
    assert_eq!(
        store.close_coding_approval(denied).await.unwrap(),
        CodingEventAppend::AlreadyPresent(NonZeroU64::new(4).unwrap())
    );
    assert_eq!(
        store
            .get_coding_approval_request(&job, request.request_sequence())
            .await
            .unwrap()
            .unwrap()
            .closure
            .unwrap()
            .disposition,
        ApprovalDisposition::Denied
    );
    assert_eq!(
        store.get_coding_job(&job).await.unwrap().unwrap().state,
        JobState::Completed
    );
    store.close().await;
    let _ = std::fs::remove_dir_all(path);
}

#[tokio::test]
async fn negative_closure_with_wrong_actor_rolls_back_and_reference_errors_are_distinct() {
    let (path, store, job) = running("actor-refusal").await;
    let actor = child(&store, &job, "owner").await;
    let other = child(&store, &job, "other").await;
    let request = append_request(
        &store,
        &job,
        2,
        Some((&actor.0, &actor.1)),
        now().saturating_sub(1),
        now() + 60_000,
    )
    .await;
    let wrong = event(
        3,
        &job,
        Some((&other.0, &other.1)),
        CodingEventKind::ApprovalClosed {
            request_sequence: request.request_sequence(),
            disposition: ApprovalDisposition::Cancelled,
        },
    );
    assert_eq!(
        store.close_coding_approval(wrong).await.unwrap_err(),
        StoreError::Conflict
    );
    assert!(
        store
            .get_coding_approval_request(&job, request.request_sequence())
            .await
            .unwrap()
            .unwrap()
            .closure
            .is_none()
    );
    let missing = event(
        3,
        &job,
        Some((&actor.0, &actor.1)),
        CodingEventKind::ApprovalClosed {
            request_sequence: NonZeroU64::new(1).unwrap(),
            disposition: ApprovalDisposition::Denied,
        },
    );
    assert_eq!(
        store.close_coding_approval(missing).await.unwrap_err(),
        StoreError::NotFound
    );
    assert_eq!(
        store
            .list_coding_events(&job, None, 10)
            .await
            .unwrap()
            .events
            .len(),
        2
    );
    store.close().await;
    let _ = std::fs::remove_dir_all(path);
}
