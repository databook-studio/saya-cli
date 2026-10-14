//! Bounds, stale-plan barrier and concurrency legs of reconciliation: the
//! 256-effect ceiling, the refusal a foreign append must trigger, and two
//! concurrent reconciles of one plan reconciling every effect exactly once.

use std::{collections::BTreeMap, num::NonZeroU64};

use saya_harness::coding::coordinator::{ReconcileError, plan_reconciliation, reconcile_effects};
use saya_store::{SqliteStateStore, StoreError};
use saya_types::{
    BudgetCharge, BudgetDimension, BudgetEffectState, BudgetObservation, BudgetObservationValue,
    BudgetReservationV1, BudgetSettlementV1, CodingEventKind, CodingEventV1, JobId, JobState,
    Sha256Digest,
};

use super::support::{Fixture, job_event};

/// The `BudgetReserved` event for one unresolved-effect reservation.
fn reserved(job: &JobId, sequence: u64) -> CodingEventV1 {
    let one = NonZeroU64::new(1).unwrap();
    let charge = BudgetCharge::new(BudgetDimension::ToolCalls, None, None, one).unwrap();
    let reservation = BudgetReservationV1::new(
        job.clone(),
        NonZeroU64::new(sequence).unwrap(),
        None,
        None,
        vec![charge],
    )
    .unwrap();
    let kind = CodingEventKind::BudgetReserved {
        reservation: Box::new(reservation),
    };
    job_event(job, sequence, kind)
}

/// The `BudgetSettled` event leaving the reservation settled `Uncertain` at the next sequence.
fn uncertain_settled(job: &JobId, sequence: u64) -> CodingEventV1 {
    let unknown = BudgetObservationValue::Unknown;
    let obs = BudgetObservation::new(BudgetDimension::ToolCalls, None, None, unknown).unwrap();
    let settlement = BudgetSettlementV1::new(
        job.clone(),
        NonZeroU64::new(sequence).unwrap(),
        NonZeroU64::new(sequence + 1).unwrap(),
        BudgetEffectState::Uncertain,
        vec![obs],
    )
    .unwrap();
    let kind = CodingEventKind::BudgetSettled {
        settlement: Box::new(settlement),
    };
    job_event(job, sequence + 1, kind)
}

/// Appends `count` unresolved effects (alternating settled-Uncertain and
/// open) in exact stream order from `start`, through the same store budget
/// APIs the ledger writes. Returns the last sequence and reservation list.
async fn build_unresolved(
    db: &SqliteStateStore,
    job: &JobId,
    start: u64,
    count: u64,
) -> (u64, Vec<u64>) {
    let mut sequence = start;
    let mut reservations = Vec::new();
    for index in 0..count {
        let reserved_event = reserved(job, sequence);
        db.reserve_coding_budget(reserved_event).await.unwrap();
        reservations.push(sequence);
        if index % 2 == 0 {
            let settled_event = uncertain_settled(job, sequence);
            db.settle_coding_budget(settled_event).await.unwrap();
            sequence += 2;
        } else {
            sequence += 1;
        }
    }
    (sequence - 1, reservations)
}

/// Every `BudgetReconciled` event of a job in stream order: its sequence,
/// the reservation it reconciles, and the plan digest it carries.
async fn reconciliation_events(
    db: &SqliteStateStore,
    job: &JobId,
) -> Vec<(u64, u64, Sha256Digest)> {
    let mut cursor = None;
    let mut events = Vec::new();
    loop {
        let page = db.list_coding_events(job, cursor, 100).await.unwrap();
        for event in &page.events {
            let CodingEventKind::BudgetReconciled {
                reservation_sequence,
                note_sha256,
            } = event.event()
            else {
                continue;
            };
            events.push((
                event.sequence().get(),
                reservation_sequence.get(),
                note_sha256.clone(),
            ));
        }
        match page.next_cursor {
            None => break,
            found => cursor = found,
        }
    }
    events
}

#[tokio::test]
async fn more_than_256_unresolved_effects_refuse_the_plan_and_exactly_256_plan() {
    let fixture = Fixture::new("reconcile-bounds-256").await;
    let store = &fixture.store;
    let job = &fixture.job;
    let (last, reservations) = build_unresolved(store, job, 2, 256).await;

    // Positive control: exactly 256 unresolved effects plan fine, with a
    // digest that is stable because planning writes nothing.
    let plan = plan_reconciliation(store, job).await.unwrap();
    assert_eq!(plan.effects.len(), 256);
    assert_eq!(plan.last_sequence, last);
    for (index, effect) in plan.effects.iter().enumerate() {
        assert_eq!(effect.reservation_sequence.get(), reservations[index]);
        assert_eq!(effect.settled_uncertain, index % 2 == 0);
    }
    let repeat = plan_reconciliation(store, job).await.unwrap();
    assert_eq!(repeat.digest, plan.digest);

    // One more unresolved effect pushes the plan past its bound: it refuses
    // and writes nothing (257 still unresolved proves nothing reconciled).
    let (after, _) = build_unresolved(store, job, last + 1, 1).await;
    let refused = plan_reconciliation(store, job).await;
    assert!(matches!(refused, Err(ReconcileError::TooManyEffects)));
    let status = store.coding_recovery_status(job).await.unwrap();
    assert_eq!(status.last_sequence, after);
    assert_eq!(status.unresolved_reservations, 257);
    fixture.close().await;
}

#[tokio::test]
async fn a_foreign_append_after_the_plan_writes_nothing_and_a_fresh_plan_lists_the_same_effects() {
    let fixture = Fixture::new("reconcile-bounds-stale").await;
    let store = &fixture.store;
    let job = &fixture.job;
    let (last, reservations) = build_unresolved(store, job, 2, 3).await;
    let plan = plan_reconciliation(store, job).await.unwrap();
    assert_eq!(plan.last_sequence, last);

    // The interleaving the digest seals the plan against: a live writer
    // appends one foreign job event after the plan was taken.
    let kind = CodingEventKind::StateChanged {
        state: JobState::Paused,
    };
    let paused = job_event(job, last + 1, kind);
    store.append_coding_event(paused).await.unwrap();

    let refused = reconcile_effects(store, job, &plan.digest).await;
    assert!(matches!(refused, Err(ReconcileError::StalePlan)));
    assert!(reconciliation_events(store, job).await.is_empty());
    let status = store.coding_recovery_status(job).await.unwrap();
    assert_eq!(
        status.last_sequence,
        last + 1,
        "the stale call wrote nothing"
    );

    // A fresh plan lists exactly the same effects; only the digest differs,
    // because the stream grew by the foreign event.
    let fresh = plan_reconciliation(store, job).await.unwrap();
    assert_ne!(fresh.digest, plan.digest);
    assert_eq!(fresh.last_sequence, last + 1);
    assert_eq!(fresh.effects, plan.effects);
    // Positive control: the fresh digest reconciles the whole list.
    let report = reconcile_effects(store, job, &fresh.digest).await.unwrap();
    assert_eq!(report.reconciled.len(), reservations.len());
    fixture.close().await;
}

/// One race iteration: two store handles on the same database path, one
/// plan, two concurrent `reconcile_effects` calls, and the exactly-once
/// and no-refund checks afterwards.
async fn one_race() {
    let fixture = Fixture::new("reconcile-bounds-race").await;
    let store = &fixture.store;
    let job = &fixture.job;
    let (last, reservations) = build_unresolved(store, job, 2, 6).await;
    let plan = plan_reconciliation(store, job).await.unwrap();
    assert_eq!(plan.last_sequence, last);

    // Two handles on the same database path, the way a second process
    // would open it.
    let second = SqliteStateStore::new(fixture.root.join("state.sqlite3"));
    let (left, right) = tokio::join!(
        reconcile_effects(store, job, &plan.digest),
        reconcile_effects(&second, job, &plan.digest),
    );
    let outcomes = [left, right];
    let winners = outcomes.iter().filter(|result| result.is_ok()).count();
    assert_eq!(winners, 1, "exactly one reconcile may win: {outcomes:?}");
    for result in &outcomes {
        let Err(error) = result else { continue };
        assert!(
            matches!(
                error,
                ReconcileError::StalePlan | ReconcileError::Store(StoreError::Conflict)
            ),
            "a loser must lose with a typed refusal: {error:?}"
        );
    }

    // Every planned reservation is reconciled exactly once, by one
    // contiguous block of events that each carry the plan's own digest.
    let events = reconciliation_events(store, job).await;
    assert_eq!(events.len(), reservations.len());
    let mut counted: BTreeMap<u64, u32> = BTreeMap::new();
    for (index, (sequence, reservation_sequence, note)) in events.iter().enumerate() {
        assert_eq!(*sequence, plan.last_sequence + 1 + index as u64);
        assert_eq!(note, &plan.digest);
        *counted.entry(*reservation_sequence).or_insert(0) += 1;
    }
    for reservation_sequence in &reservations {
        assert_eq!(counted.get(reservation_sequence), Some(&1));
    }

    // Nothing was refunded or resumed: the job is still Running, a fresh
    // plan is empty, and every reservation stays present and reconciled.
    let status = store.coding_recovery_status(job).await.unwrap();
    assert_eq!(status.state, JobState::Running);
    assert_eq!(status.unresolved_reservations, 0);
    let fresh = plan_reconciliation(store, job).await.unwrap();
    assert!(fresh.effects.is_empty());
    let records = store
        .list_coding_budget_reservations(job, None, 100)
        .await
        .unwrap();
    assert_eq!(records.items.len(), reservations.len());
    assert!(records.items.iter().all(|record| record.reconciled));

    second.close().await;
    fixture.close().await;
}

#[tokio::test]
async fn two_concurrent_reconciles_of_one_plan_reconcile_each_effect_exactly_once() {
    for _ in 0..20 {
        one_race().await;
    }
}
