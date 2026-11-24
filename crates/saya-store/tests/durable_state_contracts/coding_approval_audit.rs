//! Two writers of one job's event stream: the number an append takes is
//! allocated inside that append's own transaction, so a number read before it
//! can never be taken from another writer.
#![cfg(unix)]

use super::job_metadata_storage::{create_job, temp_root};
use saya_store::{CodingEventAppend, SqliteStateStore, StoreError};
use saya_types::{
    ApprovalNetworkDestination, ApprovalNetworkScheme, ApprovalRequestContext, ApprovalRequestV1,
    ApprovalScope, CodingEventKind, CodingEventV1, JobId, JobState, Sha256Digest,
};
use std::{
    fs,
    num::NonZeroU64,
    time::{SystemTime, UNIX_EPOCH},
};

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis()
        .try_into()
        .unwrap()
}

fn next(sequence: u64) -> NonZeroU64 {
    NonZeroU64::new(sequence).unwrap()
}

fn event(sequence: u64, job: &JobId, kind: CodingEventKind) -> CodingEventV1 {
    CodingEventV1::new(next(sequence), job.clone(), None, None, kind).unwrap()
}

/// One coordinator approval request, at `sequence`, over `host`'s scope: two
/// requests for one job differ, so neither can be mistaken for the other.
fn request(job: &JobId, sequence: NonZeroU64, host: &str) -> Result<CodingEventV1, StoreError> {
    let requested = now();
    let context = ApprovalRequestContext::new(
        Sha256Digest::parse(&"a".repeat(64)).unwrap(),
        1,
        requested,
        requested.saturating_add(600_000),
    )
    .unwrap();
    let scope = ApprovalScope::network_access(
        ApprovalNetworkDestination::new(ApprovalNetworkScheme::Https, host).unwrap(),
    );
    let request =
        ApprovalRequestV1::new_for_coordinator(job.clone(), sequence, context, scope).unwrap();
    CodingEventV1::new(
        sequence,
        job.clone(),
        None,
        None,
        CodingEventKind::ApprovalRequested {
            request: Box::new(request),
        },
    )
    .map_err(|_| StoreError::Invalid)
}

/// The sequences the two racing writers landed at, in order, once neither is
/// still running.
fn landed(results: &[Result<CodingEventAppend, StoreError>]) -> Vec<u64> {
    let mut sequences: Vec<u64> = results
        .iter()
        .filter_map(|result| match result {
            Ok(CodingEventAppend::Appended(sequence)) => Some(sequence.get()),
            _ => None,
        })
        .collect();
    sequences.sort_unstable();
    sequences
}

#[tokio::test]
async fn two_coding_appends_from_one_observed_sequence_never_take_each_others_number() {
    let root = temp_root("coding-approval-audit");
    let store = SqliteStateStore::new(root.join("state.sqlite3"));
    let job = create_job(&store, "audit-race").await;
    let id = job.job_id.clone();
    store
        .append_coding_event(event(
            1,
            &id,
            CodingEventKind::StateChanged {
                state: JobState::Running,
            },
        ))
        .await
        .unwrap();

    // The premise: both writers see the same tail before either appends.
    let observed = store
        .coding_recovery_status(&id)
        .await
        .unwrap()
        .last_sequence;
    assert_eq!(observed, 1, "one event is durable before the race");

    let (one, two) = tokio::join!(
        store.record_coding_approval_request_at_next_sequence(&id, |sequence| request(
            &id,
            sequence,
            "one.example"
        )),
        store.record_coding_approval_request_at_next_sequence(&id, |sequence| request(
            &id,
            sequence,
            "two.example"
        ))
    );
    let results = [one, two];
    for result in &results {
        if let Err(error) = result {
            assert_eq!(
                *error,
                StoreError::Conflict,
                "a writer that loses the race is refused with the typed conflict: {results:?}"
            );
        }
    }
    let sequences = landed(&results);
    assert!(
        sequences.iter().all(|sequence| *sequence > observed),
        "every landed append is past the observed tail: {sequences:?}"
    );
    assert_eq!(
        sequences.len(),
        2,
        "an append that allocates its own number leaves both writers one: {results:?}"
    );
    assert_eq!(
        sequences,
        vec![observed + 1, observed + 2],
        "the two land at distinct, contiguous sequences: {results:?}"
    );

    // Neither record was overwritten by the other, and a later append still
    // finds a free number: nothing was silently lost.
    let page = store
        .list_coding_approval_requests(&id, None, 10)
        .await
        .unwrap();
    let stored: Vec<u64> = page
        .items
        .iter()
        .map(|record| record.request.request_sequence().get())
        .collect();
    assert_eq!(
        stored,
        vec![observed + 1, observed + 2],
        "both audit records are durable, one per sequence"
    );
    assert!(
        page.items.iter().all(|record| record.closure.is_none()),
        "each record stands on its own request"
    );
    let later = event(
        observed + 3,
        &id,
        CodingEventKind::StateChanged {
            state: JobState::Paused,
        },
    );
    assert_eq!(
        store.append_coding_event(later.clone()).await.unwrap(),
        CodingEventAppend::Appended(later.sequence()),
        "the next writer still finds the next number free"
    );
    store.close().await;
    let _ = fs::remove_dir_all(root);
}
