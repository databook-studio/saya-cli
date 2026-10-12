use std::time::Duration;

use saya_agent::EffectAdmissionError;
use saya_harness::coding::budget::{BudgetedCommandStartError, start_budgeted};
use saya_harness::runner::CommandProcessState;
use saya_types::{BudgetReservationV1, CodingEventKind};

use super::{c01a_support, command_charge, command_timeout, hold_args, owner};
use crate::r03b::support::{Fixture, limits};

use c01a_support::HELPER;

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
