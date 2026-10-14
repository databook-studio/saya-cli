//! Budgeted command sessions: real-spawn start under charge, denial before
//! any spawn, owner-refusal release, and runner-error unknown retention.
//! Drop-without-join retention lives in `commands_session_drop.rs`.

use std::{num::NonZeroU64, path::Path, time::Duration};

use saya_harness::coding::budget::start_budgeted;
use saya_harness::coding::commands::{CommandOwner, CommandSessionError};
use saya_types::{BudgetCharge, BudgetDimension};

use super::support::{Fixture, limits};

#[allow(dead_code)] // shared test harness: this file uses only part of it
#[path = "../command_owner_lifecycle/support.rs"]
pub(super) mod owner_lifecycle_support;

mod release;
mod start;

pub(super) fn owner() -> CommandOwner {
    CommandOwner::new(2).unwrap()
}

pub(super) fn command_timeout() -> NonZeroU64 {
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

pub(super) fn hold_args(marker: &Path) -> Vec<String> {
    vec![
        "hold".to_owned(),
        marker.to_string_lossy().into_owned(),
        "30".to_owned(),
    ]
}

#[tokio::test]
async fn budgeted_runner_error_holds_slot_as_unknown() {
    let Some(live) = owner_lifecycle_support::fixture() else {
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
