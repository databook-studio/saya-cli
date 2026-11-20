//! A stale approval a dead process left behind is closed only when no live run
//! holds the job: the cleanup refuses a live owner in its own path, before it
//! writes a closure into a stream that owner is still numbering.

#![cfg(unix)]

use super::super::super::{CodingApplication, CodingError};
use super::super::parent_turn_fixture::in_place;
use super::super::parent_turn_provider::Scripted;
use super::first_turn;
use saya_store::StoreError;
use saya_types::{
    ApprovalRequestContext, ApprovalRequestV1, ApprovalScope, CodingEventKind, CodingEventV1,
    JobApprovalAction, JobId, Sha256Digest,
};
use std::sync::Arc;

/// The job's last durable event sequence.
async fn tail(application: &Arc<CodingApplication>, id: &JobId) -> u64 {
    application
        .store
        .coding_recovery_status(id)
        .await
        .expect("the recovery status reads")
        .last_sequence
}

/// How many of the job's approval requests are still open.
async fn pending(application: &Arc<CodingApplication>, id: &JobId) -> usize {
    application
        .store
        .list_coding_approval_requests(id, None, 100)
        .await
        .expect("the job's approval audit reads")
        .items
        .iter()
        .filter(|record| record.closure.is_none())
        .count()
}

/// Records the expired request a run that asked and never heard back leaves:
/// its window has already passed, so a resume may clean it.
async fn record_expired(application: &Arc<CodingApplication>, id: &JobId) {
    let digest = Sha256Digest::parse(&"0".repeat(64)).expect("a hex digest");
    application
        .store
        .record_coding_approval_request_at_next_sequence(id, |sequence| {
            let request = ApprovalRequestV1::new_for_coordinator(
                id.clone(),
                sequence,
                ApprovalRequestContext::new(digest.clone(), 1, 1_000, 2_000)
                    .map_err(|_| StoreError::Invalid)?,
                ApprovalScope::job_operation(JobApprovalAction::Resume, digest.clone()),
            )
            .map_err(|_| StoreError::Invalid)?;
            CodingEventV1::new(
                sequence,
                id.clone(),
                None,
                None,
                CodingEventKind::ApprovalRequested {
                    request: Box::new(request),
                },
            )
            .map_err(|_| StoreError::Invalid)
        })
        .await
        .expect("the dead process's request is recorded");
}

/// The cleanup is refused by the live owner itself, not by the caller's
/// ordering: the request stays open and the owner's stream takes no closure.
#[tokio::test]
async fn a_stale_approval_cleanup_never_writes_into_a_live_owner() {
    let provider = Scripted::start();
    let fixture = in_place("resume-stale-approval", &provider.endpoint).await;
    let id = fixture.job.job_id().clone();
    let application = Arc::clone(&fixture.job.application);
    let first = first_turn(&fixture.job).await;
    assert!(first.is_ok(), "the first turn must run: {first:?}");
    record_expired(&application, &id).await;
    let before = tail(&application, &id).await;
    assert_eq!(
        pending(&application, &id).await,
        1,
        "the job holds the expired request"
    );

    let refused = application.close_stale_approvals(&id).await;
    let Err(error) = refused else {
        panic!("a live owner must refuse the cleanup");
    };
    assert!(
        matches!(error, CodingError::InPlaceRootBusy),
        "the live owner refuses the cleanup: {error:?}"
    );
    assert_eq!(
        tail(&application, &id).await,
        before,
        "the cleanup appends nothing to the owner's stream"
    );
    assert_eq!(
        pending(&application, &id).await,
        1,
        "and leaves the request open"
    );
}
