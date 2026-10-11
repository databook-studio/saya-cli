//! Budgeted command sessions: real-spawn start under charge, denial before
//! any spawn, owner-refusal release, runner-error unknown retention, and
//! drop-without-join retention.

use std::{num::NonZeroU64, path::Path, time::Duration};

use saya_agent::EffectAdmissionError;
use saya_harness::coding::budget::{BudgetedCommandStartError, start_budgeted};
use saya_harness::coding::commands::{CommandOwner, CommandSessionError, CommandStartError};
use saya_harness::runner::CommandProcessState;
use saya_types::{
    BudgetCharge, BudgetDimension, BudgetEffectState, BudgetObservationValue, BudgetReservationV1,
    BudgetSettlementV1, CodingEventKind,
};

use super::support::{Fixture, limits};

#[allow(dead_code)] // shared test harness: this file uses only part of it
#[path = "../c01a/support.rs"]
mod c01a_support;

use c01a_support::HELPER;

fn owner() -> CommandOwner {
    CommandOwner::new(2).unwrap()
}

fn command_timeout() -> NonZeroU64 {
    NonZeroU64::new(30).unwrap()
}

fn command_charge() -> BudgetCharge {
    BudgetCharge::new(
        BudgetDimension::CommandSlots,
        None,
        None,
        NonZeroU64::new(1).unwrap(),
    )
    .unwrap()
}

fn hold_args(marker: &Path) -> Vec<String> {
    vec![
        "hold".to_owned(),
        marker.to_string_lossy().into_owned(),
        "30".to_owned(),
    ]
}

#[tokio::test]
async fn budgeted_start_spawns_real_process_under_charge() {
    let Some(live) = c01a_support::fixture() else {
        return;
    };
    let fixture = Fixture::new("budgeted-start-spawns").await;
    let ledger = fixture
        .ledger(limits(8, 8, 8, 4, 4, Duration::from_secs(30)))
        .await
        .unwrap();
    let owner = owner();
    let marker = live.marker("budgeted-start-exit");
    let session = start_budgeted(
        &ledger,
        &owner,
        live.run_program(),
        HELPER.to_owned(),
        vec![
            "exit".to_owned(),
            marker.to_string_lossy().into_owned(),
            "0".to_owned(),
        ],
        command_timeout(),
    )
    .await
    .unwrap();
    assert_eq!(ledger.snapshot().await.command_slots_active, 1);
    assert!(!matches!(
        session.session().process_state(),
        CommandProcessState::NotSpawned
    ));
    c01a_support::wait_for_file(&marker).await;
    let joined = session.join().await;
    assert!(joined.command.is_ok());
    assert!(joined.settlement.is_ok());
    assert_eq!(ledger.snapshot().await.command_slots_active, 0);
    fixture.close().await;
}

#[tokio::test]
async fn budgeted_start_denied_before_any_spawn() {
    let Some(live) = c01a_support::fixture() else {
        return;
    };
    let fixture = Fixture::new("budgeted-start-denied").await;
    let ledger = fixture
        .ledger(limits(8, 8, 8, 4, 4, Duration::from_secs(30)))
        .await
        .unwrap();
    let owner = owner();
    let first_marker = live.marker("budgeted-denied-first");
    let second_marker = live.marker("budgeted-denied-second");
    let first = start_budgeted(
        &ledger,
        &owner,
        live.run_program(),
        HELPER.to_owned(),
        hold_args(&first_marker),
        command_timeout(),
    )
    .await
    .unwrap();
    let second = start_budgeted(
        &ledger,
        &owner,
        live.run_program(),
        HELPER.to_owned(),
        hold_args(&second_marker),
        command_timeout(),
    )
    .await
    .unwrap();
    c01a_support::wait_for_file(&first_marker).await;
    c01a_support::wait_for_file(&second_marker).await;
    let denied = start_budgeted(
        &ledger,
        &owner,
        live.run_program(),
        HELPER.to_owned(),
        hold_args(&live.marker("budgeted-denied-third")),
        command_timeout(),
    )
    .await;
    assert!(matches!(
        denied,
        Err(BudgetedCommandStartError::Budget(
            EffectAdmissionError::Denied
        ))
    ));
    assert_eq!(ledger.snapshot().await.command_slots_active, 2);
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
    for reservation in &reserved {
        assert!(reservation.agent_id().is_none());
        assert!(reservation.invocation_id().is_none());
        assert_eq!(
            reservation.charges(),
            std::slice::from_ref(&command_charge())
        );
    }
    for session in [first, second] {
        let joined = session.stop_and_join().await;
        assert!(joined.command.is_ok());
        assert!(joined.settlement.is_ok());
    }
    assert_eq!(ledger.snapshot().await.command_slots_active, 0);
    fixture.close().await;
}

#[tokio::test]
async fn budgeted_owner_stopped_releases_reserved_slot() {
    let Some(live) = c01a_support::fixture() else {
        return;
    };
    let fixture = Fixture::new("budgeted-owner-stopped").await;
    let ledger = fixture
        .ledger(limits(8, 8, 8, 4, 4, Duration::from_secs(30)))
        .await
        .unwrap();
    let owner = owner();
    owner.cancel_all();
    let refused = start_budgeted(
        &ledger,
        &owner,
        live.run_program(),
        HELPER.to_owned(),
        hold_args(&live.marker("budgeted-owner-stopped")),
        command_timeout(),
    )
    .await;
    assert!(matches!(
        refused,
        Err(BudgetedCommandStartError::Start(
            CommandStartError::OwnerStopped
        ))
    ));
    assert_eq!(ledger.snapshot().await.command_slots_active, 0);
    let page = fixture
        .store
        .list_coding_events(&fixture.job, None, 100)
        .await
        .unwrap();
    let reserved = page
        .events
        .iter()
        .filter(|event| matches!(event.event(), CodingEventKind::BudgetReserved { .. }))
        .count();
    assert_eq!(reserved, 1);
    let settled: Vec<&BudgetSettlementV1> = page
        .events
        .iter()
        .filter_map(|event| match event.event() {
            CodingEventKind::BudgetSettled { settlement } => Some(settlement.as_ref()),
            _ => None,
        })
        .collect();
    assert_eq!(settled.len(), 1);
    assert_eq!(settled[0].effect_state(), BudgetEffectState::Settled);
    assert_eq!(settled[0].observations().len(), 1);
    assert_eq!(
        settled[0].observations()[0].dimension(),
        BudgetDimension::CommandSlots
    );
    assert_eq!(
        settled[0].observations()[0].value(),
        BudgetObservationValue::Measured(0)
    );
    fixture.close().await;
}

#[tokio::test]
async fn budgeted_runner_error_holds_slot_as_unknown() {
    let Some(live) = c01a_support::fixture() else {
        return;
    };
    let fixture = Fixture::new("budgeted-runner-error").await;
    let ledger = fixture
        .ledger(limits(8, 8, 8, 4, 4, Duration::from_secs(30)))
        .await
        .unwrap();
    let owner = owner();
    let session = start_budgeted(
        &ledger,
        &owner,
        live.run_program(),
        "blocked-helper".to_owned(),
        Vec::new(),
        command_timeout(),
    )
    .await
    .unwrap();
    assert_eq!(ledger.snapshot().await.command_slots_active, 1);
    let joined = session.join().await;
    assert!(matches!(
        joined.command,
        Err(CommandSessionError::Runner(_))
    ));
    assert!(joined.settlement.is_ok());
    assert_eq!(ledger.snapshot().await.command_slots_active, 1);
    fixture.close().await;
}

#[tokio::test]
async fn budgeted_owner_stop_settles_known_for_unspawned_and_running_commands() {
    let Some(live) = c01a_support::fixture() else {
        return;
    };
    let fixture = Fixture::new("budgeted-owner-stop-known").await;
    let ledger = fixture
        .ledger(limits(8, 8, 8, 4, 4, Duration::from_secs(30)))
        .await
        .unwrap();
    let owner = owner();
    // An allowlist-refused start fails the supervision task before any process
    // existed (the same pre-spawn terminal state as a missing program file).
    // The owner then deliberately stops the session: the joined terminal state
    // is KNOWN not-started, never Unknown, and the reservation resolves.
    let unspawned = start_budgeted(
        &ledger,
        &owner,
        live.run_program(),
        "blocked-helper".to_owned(),
        Vec::new(),
        command_timeout(),
    )
    .await
    .unwrap();
    assert_eq!(ledger.snapshot().await.command_slots_active, 1);
    // Let the pre-spawn failure land before the stop: the deliberate stop must
    // observe the joined terminal state, not race the task's first poll.
    tokio::time::timeout(Duration::from_secs(5), async {
        while !unspawned.session().is_finished() {
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("the pre-spawn runner failure settles the supervision task");
    let joined = unspawned.stop_and_join().await;
    assert!(matches!(
        joined.command,
        Err(CommandSessionError::StoppedBeforeSpawn { .. })
    ));
    assert!(joined.settlement.is_ok());
    assert_eq!(ledger.snapshot().await.command_slots_active, 0);
    let page = fixture
        .store
        .list_coding_events(&fixture.job, None, 100)
        .await
        .unwrap();
    let settled: Vec<&BudgetSettlementV1> = page
        .events
        .iter()
        .filter_map(|event| match event.event() {
            CodingEventKind::BudgetSettled { settlement } => Some(settlement.as_ref()),
            _ => None,
        })
        .collect();
    assert_eq!(settled.len(), 1);
    assert_eq!(settled[0].effect_state(), BudgetEffectState::Settled);
    assert_eq!(
        settled[0].observations()[0].value(),
        BudgetObservationValue::Measured(0)
    );

    // A running command stopped by its owner is reaped with a known cancelled
    // outcome and leaves no unresolved reservation either.
    let marker = live.marker("budgeted-owner-stop-running");
    let running = start_budgeted(
        &ledger,
        &owner,
        live.run_program(),
        HELPER.to_owned(),
        hold_args(&marker),
        command_timeout(),
    )
    .await
    .unwrap();
    let pid = c01a_support::wait_for_file(&marker).await;
    let joined = running.stop_and_join().await;
    let outcome = joined.command.unwrap();
    assert!(outcome.cancelled);
    assert_eq!(outcome.pid, pid);
    assert!(joined.settlement.is_ok());
    assert_eq!(ledger.snapshot().await.command_slots_active, 0);
    assert!(
        c01a_support::wait_group_gone(pid).await,
        "the stopped command's process group is reaped"
    );
    fixture.close().await;
}

#[tokio::test]
async fn budgeted_dropped_session_stays_charged() {
    let Some(live) = c01a_support::fixture() else {
        return;
    };
    let fixture = Fixture::new("budgeted-drop-held").await;
    let ledger = fixture
        .ledger(limits(8, 8, 8, 4, 4, Duration::from_secs(30)))
        .await
        .unwrap();
    let owner = owner();
    let marker = live.marker("budgeted-drop-held");
    let session = start_budgeted(
        &ledger,
        &owner,
        live.run_program(),
        HELPER.to_owned(),
        hold_args(&marker),
        command_timeout(),
    )
    .await
    .unwrap();
    let helper_pid = c01a_support::wait_for_file(&marker).await;
    session.session().stop();
    drop(session);
    assert!(
        c01a_support::wait_group_gone(helper_pid).await,
        "the supervision task must kill and reap the dropped session's process group"
    );
    assert_eq!(ledger.snapshot().await.command_slots_active, 1);
    let page = fixture
        .store
        .list_coding_events(&fixture.job, None, 100)
        .await
        .unwrap();
    let reserved = page
        .events
        .iter()
        .filter(|event| matches!(event.event(), CodingEventKind::BudgetReserved { .. }))
        .count();
    assert_eq!(reserved, 1);
    let settled = page
        .events
        .iter()
        .filter(|event| matches!(event.event(), CodingEventKind::BudgetSettled { .. }))
        .count();
    assert_eq!(settled, 0);
    fixture.close().await;
}
