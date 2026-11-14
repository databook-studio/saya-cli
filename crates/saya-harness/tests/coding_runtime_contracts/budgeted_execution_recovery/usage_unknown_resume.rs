//! A transport attempt whose usage is unknown resumes as unknown usage, never
//! as an unreconciled effect. Records written before usage and effect
//! uncertainty were separated (settled `Uncertain` with only an unknown
//! transport observation) are read the same way, without rewriting history.

use std::{num::NonZeroU64, time::Duration};

use saya_agent::{
    EffectAdmission, EffectCallKind, EffectOutcome, EffectSettlement, PhysicalAttempt,
};
use saya_harness::coding::budget::{BudgetSnapshot, UsageCoverage};
use saya_harness::coding::coordinator::resume_job;
use saya_store::CodingRuntimeAuthority;
use saya_types::{
    BudgetCharge, BudgetDimension, BudgetEffectState, BudgetObservation, BudgetObservationValue,
    BudgetReservationV1, BudgetSettlementV1, CodingEventKind, JobId, Sha256Digest,
};

use super::support::{Fixture, job_event, limits};

fn charges(dimensions: &[BudgetDimension]) -> Vec<BudgetCharge> {
    dimensions
        .iter()
        .map(|dimension| {
            BudgetCharge::new(*dimension, None, None, NonZeroU64::new(1).unwrap()).unwrap()
        })
        .collect()
}

pub(super) async fn reserve(fixture: &Fixture, sequence: u64, dimensions: &[BudgetDimension]) {
    let reservation = BudgetReservationV1::new(
        fixture.job.clone(),
        NonZeroU64::new(sequence).unwrap(),
        None,
        None,
        charges(dimensions),
    )
    .unwrap();
    let event = job_event(
        &fixture.job,
        sequence,
        CodingEventKind::BudgetReserved {
            reservation: Box::new(reservation),
        },
    );
    fixture.store.reserve_coding_budget(event).await.unwrap();
}

pub(super) async fn settle(
    fixture: &Fixture,
    reservation: u64,
    sequence: u64,
    state: BudgetEffectState,
    observations: &[(BudgetDimension, BudgetObservationValue)],
) {
    let observations = observations
        .iter()
        .map(|(dimension, value)| BudgetObservation::new(*dimension, None, None, *value).unwrap())
        .collect();
    let settlement = BudgetSettlementV1::new(
        fixture.job.clone(),
        NonZeroU64::new(reservation).unwrap(),
        NonZeroU64::new(sequence).unwrap(),
        state,
        observations,
    )
    .unwrap();
    let event = job_event(
        &fixture.job,
        sequence,
        CodingEventKind::BudgetSettled {
            settlement: Box::new(settlement),
        },
    );
    fixture.store.settle_coding_budget(event).await.unwrap();
}

pub(super) fn authority() -> CodingRuntimeAuthority {
    CodingRuntimeAuthority {
        epoch: 2,
        policy_digest: Sha256Digest::parse(&"a".repeat(64)).unwrap(),
    }
}

async fn resumed(fixture: &Fixture, job: JobId) -> BudgetSnapshot {
    let store = fixture.reopen();
    let owner_limits = limits(8, 8, 8, 2, 1, Duration::from_secs(30));
    let ledger = resume_job(store, job, owner_limits, authority())
        .await
        .expect("unknown usage needs no reconciliation before resume");
    ledger.snapshot().await
}

fn assert_usage_unknown_only(snapshot: &BudgetSnapshot) {
    assert_eq!(snapshot.transport_attempts, 2, "both attempts stay charged");
    assert_eq!(snapshot.unknown_transport_attempts, 1);
    assert_eq!(snapshot.unknown_logical_requests, 0);
    assert_eq!(snapshot.unknown_tool_calls, 0);
    assert_eq!(snapshot.usage_coverage, UsageCoverage::Unknown);
    assert_eq!(snapshot.model_slots_active, 0);
    assert!(!snapshot.poisoned);
}

#[tokio::test]
async fn an_old_uncertain_transport_settlement_loads_as_unknown_usage() {
    use BudgetDimension::{LogicalRequests, ModelRequestSlots, TransportAttempts};
    use BudgetObservationValue::{Measured, Unknown};
    let fixture = Fixture::new("usage-unknown-legacy").await;
    let owner_limits = limits(8, 8, 8, 2, 1, Duration::from_secs(30));
    drop(fixture.ledger(owner_limits).await.unwrap());
    reserve(&fixture, 2, &[LogicalRequests, ModelRequestSlots]).await;
    reserve(&fixture, 3, &[TransportAttempts]).await;
    reserve(&fixture, 4, &[TransportAttempts]).await;
    // The shape the previous release persisted for a cut attempt.
    settle(
        &fixture,
        3,
        5,
        BudgetEffectState::Uncertain,
        &[(TransportAttempts, Unknown)],
    )
    .await;
    settle(
        &fixture,
        4,
        6,
        BudgetEffectState::Settled,
        &[(TransportAttempts, Measured(1))],
    )
    .await;
    settle(
        &fixture,
        2,
        7,
        BudgetEffectState::Settled,
        &[
            (LogicalRequests, Measured(1)),
            (ModelRequestSlots, Measured(1)),
        ],
    )
    .await;
    fixture.close_store().await;
    let status = fixture
        .reopen()
        .coding_recovery_status(&fixture.job)
        .await
        .unwrap();
    assert_eq!(status.unresolved_reservations, 0);
    let snapshot = resumed(&fixture, fixture.job.clone()).await;
    assert_usage_unknown_only(&snapshot);
    fixture.close().await;
}

#[tokio::test]
async fn a_new_unknown_usage_settlement_resumes_the_same_way() {
    let fixture = Fixture::new("usage-unknown-new").await;
    let owner_limits = limits(8, 8, 8, 2, 1, Duration::from_secs(30));
    let ledger = fixture.ledger(owner_limits).await.unwrap();
    let root = ledger.root_view();
    let model = root.admit_model_request().await.unwrap();
    for (ordinal, outcome) in [(0, EffectOutcome::Unknown), (1, EffectOutcome::Succeeded)] {
        let attempt = PhysicalAttempt::new(model, EffectCallKind::Answer, ordinal);
        let send = root.admit_physical_attempt(attempt).await.unwrap();
        root.settle(
            send,
            EffectSettlement::PhysicalAttempt {
                attempt,
                outcome,
                reported_usage: None,
                usage_complete: false,
            },
        )
        .await
        .unwrap();
    }
    root.settle(
        model,
        EffectSettlement::ModelRequest {
            outcome: EffectOutcome::Succeeded,
            reported_usage: None,
            usage_complete: false,
        },
    )
    .await
    .unwrap();
    drop(root);
    fixture.close_store().await;
    drop(ledger);
    let snapshot = resumed(&fixture, fixture.job.clone()).await;
    assert_usage_unknown_only(&snapshot);
    fixture.close().await;
}
