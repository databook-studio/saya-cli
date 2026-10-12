//! Budgeted command sessions: a dropped (never joined) session keeps its
//! command slot charged and its process group is still killed and reaped.

use std::time::Duration;

use saya_harness::coding::budget::start_budgeted;
use saya_types::CodingEventKind;

use super::commands_session::{c01a_support, command_timeout, hold_args, owner};
use super::support::{Fixture, limits};
use c01a_support::HELPER;

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
