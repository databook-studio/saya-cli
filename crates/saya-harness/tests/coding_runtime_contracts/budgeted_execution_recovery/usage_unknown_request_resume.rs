//! A model request Saya gave up on resumes as unknown usage, never as an
//! unreconciled effect, and its slot is free. Records written before usage and
//! effect uncertainty were separated (settled `Uncertain` with only unknown
//! request observations) are read the same way; a record that also leaves an
//! effect unknown still blocks. Nothing is rewritten.

use std::time::Duration;

use saya_harness::coding::budget::UsageCoverage;
use saya_harness::coding::coordinator::{ResumeError, resume_job};
use saya_types::{BudgetDimension, BudgetEffectState, BudgetObservationValue};

use super::{
    support::{Fixture, limits},
    usage_unknown_resume::{authority, reserve, settle},
};

use BudgetDimension::{LogicalRequests, ModelRequestSlots, ToolCalls, TransportAttempts};
use BudgetObservationValue::Unknown;

#[tokio::test]
async fn an_old_uncertain_model_request_loads_as_unknown_usage() {
    let fixture = Fixture::new("usage-unknown-old-request").await;
    let owner_limits = limits(8, 8, 8, 2, 1, Duration::from_secs(30));
    drop(fixture.ledger(owner_limits).await.unwrap());
    reserve(&fixture, 2, &[LogicalRequests, ModelRequestSlots]).await;
    reserve(&fixture, 3, &[TransportAttempts]).await;
    // The shapes the previous release persisted when it gave up on a request.
    settle(
        &fixture,
        3,
        4,
        BudgetEffectState::Uncertain,
        &[(TransportAttempts, Unknown)],
    )
    .await;
    settle(
        &fixture,
        2,
        5,
        BudgetEffectState::Uncertain,
        &[(LogicalRequests, Unknown), (ModelRequestSlots, Unknown)],
    )
    .await;
    fixture.close_store().await;

    let store = fixture.reopen();
    let status = store.coding_recovery_status(&fixture.job).await.unwrap();
    assert_eq!(status.unresolved_reservations, 0);
    let resumed = resume_job(store, fixture.job.clone(), owner_limits, authority())
        .await
        .expect("unknown usage needs no reconciliation before resume");
    let snapshot = resumed.snapshot().await;
    assert_eq!(snapshot.logical_requests, 1, "the request stays charged");
    assert_eq!(snapshot.unknown_logical_requests, 1);
    assert_eq!(snapshot.unknown_transport_attempts, 1);
    assert_eq!(snapshot.unknown_tool_calls, 0);
    assert_eq!(snapshot.usage_coverage, UsageCoverage::Unknown);
    assert_eq!(snapshot.model_slots_active, 0, "the slot is free");
    assert!(!snapshot.poisoned);
    fixture.close().await;
}

#[tokio::test]
async fn an_old_record_that_also_leaves_an_effect_unknown_still_needs_reconciliation() {
    let fixture = Fixture::new("usage-unknown-old-mixed").await;
    let owner_limits = limits(8, 8, 8, 2, 1, Duration::from_secs(30));
    drop(fixture.ledger(owner_limits).await.unwrap());
    reserve(
        &fixture,
        2,
        &[LogicalRequests, ModelRequestSlots, ToolCalls],
    )
    .await;
    settle(
        &fixture,
        2,
        3,
        BudgetEffectState::Uncertain,
        &[
            (LogicalRequests, Unknown),
            (ModelRequestSlots, Unknown),
            (ToolCalls, Unknown),
        ],
    )
    .await;
    fixture.close_store().await;

    let store = fixture.reopen();
    let status = store.coding_recovery_status(&fixture.job).await.unwrap();
    assert_eq!(status.unresolved_reservations, 1);
    let resumed = resume_job(store, fixture.job.clone(), owner_limits, authority()).await;
    assert!(
        matches!(resumed, Err(ResumeError::ReconciliationRequired(_))),
        "an unknown effect beside unknown usage still blocks"
    );
    fixture.close().await;
}
