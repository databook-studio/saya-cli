use std::{num::NonZeroU64, time::Duration};

use saya_agent::{EffectAdmission, EffectAdmissionError, EffectOutcome, EffectSettlement};
use saya_harness::coding::budget::BudgetLedgerError;
use saya_store::CodingEventAppend;
use saya_types::{
    BudgetCharge, BudgetDimension, BudgetReservationV1, CodingEventKind, CodingEventV1,
};

use super::fixture::{Fixture, limits};

#[tokio::test]
async fn independent_owner_cannot_resequence_after_a_competing_writer() {
    let fixture = Fixture::new("sequence-race").await;
    let limits = limits(1, 1, 1, 1, 1, Duration::from_secs(5));
    let first = fixture.ledger(limits).await.unwrap();
    let second = fixture.ledger(limits).await.unwrap();
    let left = first.root_view();
    let right = second.root_view();
    let (one, two) = tokio::join!(left.admit_model_request(), right.admit_model_request());
    assert_eq!(usize::from(one.is_ok()) + usize::from(two.is_ok()), 1);
    assert!(matches!(&one, Ok(_)) || matches!(&two, Ok(_)));
    assert!(
        matches!(&one, Err(EffectAdmissionError::OwnerFailure(_)))
            || matches!(&two, Err(EffectAdmissionError::OwnerFailure(_)))
    );
    assert_eq!(
        fixture
            .store
            .list_coding_budget_reservations(&fixture.job, None, 10)
            .await
            .unwrap()
            .items
            .len(),
        1
    );
    assert!(first.snapshot().await.poisoned || second.snapshot().await.poisoned);
    fixture.close().await;
}

#[tokio::test]
async fn constructor_refuses_prior_budget_history_instead_of_resetting_counts() {
    let fixture = Fixture::new("prior-history").await;
    let reservation = BudgetReservationV1::new(
        fixture.job.clone(),
        NonZeroU64::new(2).unwrap(),
        None,
        None,
        vec![
            BudgetCharge::new(
                BudgetDimension::ToolCalls,
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
        None,
        None,
        CodingEventKind::BudgetReserved {
            reservation: Box::new(reservation),
        },
    )
    .unwrap();
    assert_eq!(
        fixture.store.reserve_coding_budget(event).await.unwrap(),
        CodingEventAppend::Appended(NonZeroU64::new(2).unwrap())
    );
    assert!(matches!(
        fixture
            .ledger(limits(2, 2, 2, 1, 1, Duration::from_secs(5)))
            .await,
        Err(BudgetLedgerError::NotFreshRunning)
    ));
    fixture.close().await;
}

#[tokio::test]
async fn admission_denial_does_not_append_or_consume_a_count() {
    let fixture = Fixture::new("count-denial").await;
    let limits = limits(1, 1, 1, 1, 1, Duration::from_secs(5));
    let ledger = fixture.ledger(limits).await.unwrap();
    let root = ledger.root_view();
    let first = root.admit_model_request().await.unwrap();
    let denied = root.admit_model_request().await.unwrap_err();
    assert!(matches!(denied, EffectAdmissionError::Denied));
    root.settle(
        first,
        EffectSettlement::ModelRequest {
            outcome: EffectOutcome::NotStarted,
            reported_usage: None,
            usage_complete: false,
        },
    )
    .await
    .unwrap();
    assert_eq!(ledger.snapshot().await.logical_requests, 1);
    assert_eq!(
        fixture
            .store
            .list_coding_budget_reservations(&fixture.job, None, 10)
            .await
            .unwrap()
            .items
            .len(),
        1
    );
    fixture.close().await;
}
