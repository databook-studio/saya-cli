//! Owner-bound command permits: a permit admitted by one ledger cannot
//! settle another ledger's command reservation.

use std::time::Duration;

use saya_agent::{CancellationReason, EffectAdmissionError, EffectOutcome};

use super::support::{Fixture, limits};

#[tokio::test]
async fn foreign_command_permit_cannot_settle_another_ledger() {
    let owner = Fixture::new("foreign-command-owner").await;
    let owner_ledger = owner
        .ledger(limits(8, 8, 8, 4, 4, Duration::from_secs(30)))
        .await
        .unwrap();
    let permit = owner_ledger.begin_command().await.unwrap();
    let foreign = Fixture::new("foreign-command-foreign").await;
    let foreign_ledger = foreign
        .ledger(limits(8, 8, 8, 4, 4, Duration::from_secs(30)))
        .await
        .unwrap();
    let _foreign_permit = foreign_ledger.begin_command().await.unwrap();
    let before = foreign_ledger.snapshot().await;
    let result = foreign_ledger
        .finish_command(permit, EffectOutcome::Succeeded)
        .await;
    // This typed-refusal assertion fails if the Arc::ptr_eq guard is removed:
    // each ledger's first command-slot admission shares the same permit id,
    // so the foreign ledger would settle its own command reservation with
    // the owner ledger's permit.
    assert!(matches!(result, Err(EffectAdmissionError::OwnerFailure(_))));
    let after = foreign_ledger.snapshot().await;
    assert_eq!(before.command_slots_active, after.command_slots_active);
    assert!(after.poisoned);
    assert_eq!(
        foreign_ledger.cancellation_token().reason(),
        Some(CancellationReason::OwnerFailure)
    );
    let refused = foreign_ledger.begin_command().await;
    assert!(matches!(
        refused,
        Err(EffectAdmissionError::OwnerFailure(_))
    ));
    let owner_snapshot = owner_ledger.snapshot().await;
    assert_eq!(owner_snapshot.command_slots_active, 1);
    owner.close().await;
    foreign.close().await;
}
