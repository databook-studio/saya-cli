//! Resume tests for the policy proof: a job's recorded policy admits an
//! identical construction, refuses changed limits, and a job with budget
//! history but no policy row is never adopted. Also covers the typed
//! refusals at the construction boundary: the bounded history scan and
//! the checked deadline conversion.

use std::{num::NonZeroU64, time::Duration};

use saya_agent::{EffectAdmission, EffectOutcome, EffectSettlement};
use saya_harness::coding::budget::{BudgetLedgerError, JobBudgetLedger};
use saya_types::{
    AgentId, BudgetCharge, BudgetDimension, BudgetReservationV1, CodingEventKind, CodingEventV1,
    InvocationId,
};

use super::support::{Fixture, limits};

#[tokio::test]
async fn fresh_owner_records_policy_and_second_construction_with_changed_limits_refuses() {
    let fixture = Fixture::new("resume-policy-recorded").await;
    let owner_limits = limits(5, 5, 5, 2, 2, Duration::from_secs(30));
    let ledger = fixture.ledger(owner_limits).await.unwrap();
    drop(ledger);
    fixture.close_store().await;
    let store = fixture.reopen();
    let recorded = store
        .get_coding_budget_policy(&fixture.job)
        .await
        .unwrap()
        .unwrap();
    // An identical construction is the resume proof and constructs again.
    assert!(
        JobBudgetLedger::new(store.clone(), fixture.job.clone(), owner_limits)
            .await
            .is_ok()
    );
    // Changed limits are refused before any fresh-job guard.
    let changed = limits(5, 7, 5, 2, 2, Duration::from_secs(30));
    let refused = JobBudgetLedger::new(store.clone(), fixture.job.clone(), changed).await;
    assert!(matches!(refused, Err(BudgetLedgerError::PolicyMismatch)));
    let after = store
        .get_coding_budget_policy(&fixture.job)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(after.policy_sha256, recorded.policy_sha256);
    let reservations = store
        .list_coding_budget_reservations(&fixture.job, None, 100)
        .await
        .unwrap();
    assert!(reservations.items.is_empty());
    fixture.close().await;
}

#[tokio::test]
async fn resume_policy_mismatch_refuses() {
    let fixture = Fixture::new("resume-policy-mismatch").await;
    let owner_limits = limits(5, 5, 5, 2, 2, Duration::from_secs(30));
    let ledger = fixture.ledger(owner_limits).await.unwrap();
    let _admitted = ledger.root_view().admit_model_request().await.unwrap();
    drop(ledger);
    fixture.close_store().await;
    let store = fixture.reopen();
    let recorded = store
        .get_coding_budget_policy(&fixture.job)
        .await
        .unwrap()
        .unwrap();
    let changed = limits(5, 5, 5, 3, 2, Duration::from_secs(30));
    let refused = JobBudgetLedger::resume(store.clone(), fixture.job.clone(), changed).await;
    assert!(matches!(refused, Err(BudgetLedgerError::PolicyMismatch)));
    // The refusal leaves the durable truth untouched.
    let after = store
        .get_coding_budget_policy(&fixture.job)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(after.policy_sha256, recorded.policy_sha256);
    let reservations = store
        .list_coding_budget_reservations(&fixture.job, None, 100)
        .await
        .unwrap();
    assert_eq!(reservations.items.len(), 1);
    assert!(reservations.items[0].settlement.is_none());
    fixture.close().await;
}

#[tokio::test]
async fn resume_without_policy_row_refuses() {
    let fixture = Fixture::new("resume-policy-missing").await;
    let authority = fixture.add_child("agent-hist", "invocation-hist").await;
    let actor: (&AgentId, &InvocationId) = (authority.agent_id(), authority.invocation_id());
    let store = fixture.store.clone();
    // Budget history written directly through the store, with no policy
    // row: the pre-R03b1 shape that must never be adopted.
    let reservation = BudgetReservationV1::new(
        fixture.job.clone(),
        NonZeroU64::new(2).unwrap(),
        Some(actor.0.clone()),
        Some(actor.1.clone()),
        vec![
            BudgetCharge::new(
                BudgetDimension::LogicalRequests,
                None,
                None,
                NonZeroU64::new(1).unwrap(),
            )
            .unwrap(),
        ],
    )
    .unwrap();
    let event = CodingEventV1::new(
        NonZeroU64::new(2).unwrap(),
        fixture.job.clone(),
        Some(actor.0.clone()),
        Some(actor.1.clone()),
        CodingEventKind::BudgetReserved {
            reservation: Box::new(reservation),
        },
    )
    .unwrap();
    store.reserve_coding_budget(event).await.unwrap();
    // Reopen as a restarted process, matching the other restart tests.
    fixture.close_store().await;
    let store = fixture.reopen();
    let owner_limits = limits(4, 4, 4, 2, 2, Duration::from_secs(30));
    // A fresh construction on history without a policy is refused before
    // the fresh-job guard: the store's first-policy insert may not
    // authenticate budget history that no recorded policy covers, and no
    // policy row may appear as a side effect.
    let fresh_refused =
        JobBudgetLedger::new(store.clone(), fixture.job.clone(), owner_limits).await;
    assert!(matches!(fresh_refused, Err(BudgetLedgerError::Store)));
    let policy_after_new = store.get_coding_budget_policy(&fixture.job).await.unwrap();
    assert!(policy_after_new.is_none());
    let refused = JobBudgetLedger::resume(store.clone(), fixture.job.clone(), owner_limits).await;
    assert!(matches!(refused, Err(BudgetLedgerError::PolicyMissing)));
    let policy = store.get_coding_budget_policy(&fixture.job).await.unwrap();
    assert!(policy.is_none());
    let reservations = store
        .list_coding_budget_reservations(&fixture.job, None, 100)
        .await
        .unwrap();
    assert_eq!(reservations.items.len(), 1);
    fixture.close().await;
}

#[tokio::test]
async fn resume_history_over_bound_refuses() {
    let fixture = Fixture::new("resume-history-bound").await;
    let owner_limits = limits(400, 8, 8, 2, 2, Duration::from_secs(30));
    let ledger = fixture.ledger(owner_limits).await.unwrap();
    let root = ledger.root_view();
    // 128 reserve+settle rounds produce 256 budget events; with the
    // fixture's running event the durable history is 257 events, one over
    // the bounded scan.
    for _ in 0..128 {
        let permit = root.admit_model_request().await.unwrap();
        root.settle(
            permit,
            EffectSettlement::ModelRequest {
                outcome: EffectOutcome::NotStarted,
                reported_usage: None,
                usage_complete: false,
            },
        )
        .await
        .unwrap();
    }
    drop(root);
    fixture.close_store().await;
    drop(ledger);
    let store = fixture.reopen();
    let refused = JobBudgetLedger::resume(store, fixture.job.clone(), owner_limits).await;
    assert!(matches!(
        refused,
        Err(BudgetLedgerError::ResumeHistoryTooLong)
    ));
    fixture.close().await;
}

#[tokio::test]
async fn deadline_conversion_rejects_sub_millisecond_and_overflowing_values() {
    let fixture = Fixture::new("resume-deadline-conversion").await;
    let sub_millisecond = limits(4, 4, 4, 2, 2, Duration::from_nanos(500));
    let refused = fixture.ledger(sub_millisecond).await;
    assert!(matches!(refused, Err(BudgetLedgerError::InvalidLimits)));
    let overflowing = limits(4, 4, 4, 2, 2, Duration::from_secs(u64::MAX));
    let refused = fixture.ledger(overflowing).await;
    assert!(matches!(refused, Err(BudgetLedgerError::InvalidLimits)));
    // Neither refusal recorded a policy row.
    let policy = fixture
        .store
        .get_coding_budget_policy(&fixture.job)
        .await
        .unwrap();
    assert!(policy.is_none());
    // An exact u64-millisecond deadline is representable and constructs.
    let exact = limits(4, 4, 4, 2, 2, Duration::from_millis(u64::MAX));
    assert!(fixture.ledger(exact).await.is_ok());
    fixture.close().await;
}
