use std::time::Duration;

use saya_harness::coding::budget::{BudgetedCommandStartError, start_budgeted};
use saya_harness::coding::commands::{CommandSessionError, CommandStartError};
use saya_types::{
    BudgetDimension, BudgetEffectState, BudgetObservationValue, BudgetSettlementV1, CodingEventKind,
};

use super::{c01a_support, command_timeout, hold_args, owner};
use crate::r03b::support::{Fixture, limits};

use c01a_support::HELPER;

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
