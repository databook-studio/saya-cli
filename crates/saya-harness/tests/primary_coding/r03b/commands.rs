//! Ledger-level command-slot tests: admission counts and caps, settlement
//! release semantics for known and not-started outcomes, unknown-outcome
//! slot retention with double-settle poisoning, and the resumed fold over
//! released command charges.

use std::{num::NonZeroU64, time::Duration};

use saya_agent::{EffectAdmissionError, EffectOutcome};
use saya_harness::coding::budget::JobBudgetLedger;
use saya_types::{BudgetCharge, BudgetDimension, BudgetReservationV1, CodingEventKind};

use super::support::{Fixture, limits};

#[tokio::test]
async fn command_slot_admission_counts_and_caps() {
    let fixture = Fixture::new("command-admission-caps").await;
    let ledger = fixture
        .ledger(limits(8, 8, 8, 4, 4, Duration::from_secs(30)))
        .await
        .unwrap();
    let _first = ledger.begin_command().await.unwrap();
    let _second = ledger.begin_command().await.unwrap();
    let third = ledger.begin_command().await;
    assert!(matches!(third, Err(EffectAdmissionError::Denied)));
    let snapshot = ledger.snapshot().await;
    assert_eq!(snapshot.command_slots_active, 2);
    let page = fixture
        .store
        .list_coding_events(&fixture.job, None, 100)
        .await
        .unwrap();
    let reserved: Vec<&BudgetReservationV1> = page
        .events
        .iter()
        .filter_map(|event| match event.event() {
            CodingEventKind::BudgetReserved { reservation } => Some(reservation.as_ref()),
            _ => None,
        })
        .collect();
    assert_eq!(reserved.len(), 2);
    let expected_charge = BudgetCharge::new(
        BudgetDimension::CommandSlots,
        None,
        None,
        NonZeroU64::new(1).unwrap(),
    )
    .unwrap();
    for reservation in &reserved {
        assert!(reservation.agent_id().is_none());
        assert!(reservation.invocation_id().is_none());
        assert_eq!(
            reservation.charges(),
            std::slice::from_ref(&expected_charge)
        );
    }
    let rows = fixture
        .store
        .list_coding_budget_reservations(&fixture.job, None, 50)
        .await
        .unwrap();
    assert_eq!(rows.items.len(), 2);
    fixture.close().await;
}

#[tokio::test]
async fn command_settlement_known_releases_slot() {
    let fixture = Fixture::new("command-settle-succeeded").await;
    let ledger = fixture
        .ledger(limits(8, 8, 8, 4, 4, Duration::from_secs(30)))
        .await
        .unwrap();
    let permit = ledger.begin_command().await.unwrap();
    ledger
        .finish_command(permit, EffectOutcome::Succeeded)
        .await
        .unwrap();
    let snapshot = ledger.snapshot().await;
    assert_eq!(snapshot.command_slots_active, 0);
    assert!(!snapshot.poisoned);
    let next = ledger.begin_command().await;
    assert!(next.is_ok());
    fixture.close().await;
}

#[tokio::test]
async fn command_not_started_settlement_releases_slot() {
    let fixture = Fixture::new("command-settle-not-started").await;
    let ledger = fixture
        .ledger(limits(8, 8, 8, 4, 4, Duration::from_secs(30)))
        .await
        .unwrap();
    let permit = ledger.begin_command().await.unwrap();
    ledger
        .finish_command(permit, EffectOutcome::NotStarted)
        .await
        .unwrap();
    let snapshot = ledger.snapshot().await;
    assert_eq!(snapshot.command_slots_active, 0);
    assert!(!snapshot.poisoned);
    let next = ledger.begin_command().await;
    assert!(next.is_ok());
    fixture.close().await;
}

#[tokio::test]
async fn command_settlement_unknown_holds_slot_and_double_settle_poisons() {
    let fixture = Fixture::new("command-settle-unknown").await;
    let ledger = fixture
        .ledger(limits(8, 8, 8, 4, 4, Duration::from_secs(30)))
        .await
        .unwrap();
    let permit = ledger.begin_command().await.unwrap();
    let replay_permit = permit.clone();
    ledger
        .finish_command(permit, EffectOutcome::Unknown)
        .await
        .unwrap();
    let snapshot = ledger.snapshot().await;
    assert_eq!(snapshot.command_slots_active, 1);
    let replay = ledger
        .finish_command(replay_permit, EffectOutcome::Unknown)
        .await;
    assert!(matches!(replay, Err(EffectAdmissionError::OwnerFailure(_))));
    let snapshot = ledger.snapshot().await;
    assert!(snapshot.poisoned);
    let refused = ledger.begin_command().await;
    assert!(matches!(
        refused,
        Err(EffectAdmissionError::OwnerFailure(_))
    ));
    fixture.close().await;
}

#[tokio::test]
async fn resume_folds_released_command_charges() {
    let fixture = Fixture::new("resume-command-charges").await;
    let owner_limits = limits(8, 8, 8, 4, 4, Duration::from_secs(30));
    let ledger = fixture.ledger(owner_limits).await.unwrap();
    let permit = ledger.begin_command().await.unwrap();
    ledger
        .finish_command(permit, EffectOutcome::Succeeded)
        .await
        .unwrap();
    let before = ledger.snapshot().await;
    assert_eq!(before.command_slots_active, 0);
    fixture.close_store().await;
    drop(ledger);

    let store = fixture.reopen();
    let resumed = JobBudgetLedger::resume(store.clone(), fixture.job.clone(), owner_limits)
        .await
        .unwrap();
    let snapshot = resumed.snapshot().await;
    assert_eq!(snapshot.command_slots_active, 0);
    assert!(!snapshot.poisoned);
    fixture.close().await;
}
