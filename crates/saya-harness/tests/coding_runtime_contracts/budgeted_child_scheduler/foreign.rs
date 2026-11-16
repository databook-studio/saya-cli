use std::time::Duration;

use saya_agent::{CancellationReason, EffectAdmissionError, EffectOutcome};
use saya_harness::coding::budget::ChildCountLimits;
use saya_store::SqliteStateStore;

use super::fixture::{Fixture, limits};

#[tokio::test]
async fn foreign_child_session_cannot_settle_another_ledger() {
    let owner = Fixture::new("foreign-session-owner").await;
    let owner_authority = owner.add_child("agent-foreign", "invocation-foreign").await;
    let owner_ledger = owner
        .ledger(limits(4, 4, 4, 2, 2, Duration::from_secs(30)))
        .await
        .unwrap();
    let session = owner_ledger
        .begin_child(
            owner_authority.agent_id(),
            owner_authority.invocation_id(),
            ChildCountLimits {
                logical_requests: 2,
                tool_calls: 1,
            },
        )
        .await
        .unwrap();
    let foreign = Fixture::new("foreign-session-foreign").await;
    let foreign_authority = foreign
        .add_child("agent-foreign", "invocation-foreign")
        .await;
    let foreign_ledger = foreign
        .ledger(limits(4, 4, 4, 2, 2, Duration::from_secs(30)))
        .await
        .unwrap();
    let _foreign_session = foreign_ledger
        .begin_child(
            foreign_authority.agent_id(),
            foreign_authority.invocation_id(),
            ChildCountLimits {
                logical_requests: 2,
                tool_calls: 1,
            },
        )
        .await
        .unwrap();
    let before = foreign_ledger.snapshot().await;
    let result = foreign_ledger
        .finish_child(session, EffectOutcome::Succeeded)
        .await;
    // This typed-refusal assertion fails if the Arc::ptr_eq guard is removed:
    // the foreign ledger's local map holds the same actor identity as the
    // session, and both children are their ledger's first child-slot
    // admission, so the foreign ledger's own Child permit shares the session's
    // permit id and settlement would succeed, settling the foreign ledger's
    // own child reservation.
    assert!(matches!(result, Err(EffectAdmissionError::OwnerFailure(_))));
    let after = foreign_ledger.snapshot().await;
    assert_eq!(before.logical_requests, after.logical_requests);
    assert_eq!(before.transport_attempts, after.transport_attempts);
    assert_eq!(before.tool_calls, after.tool_calls);
    assert_eq!(before.model_slots_active, after.model_slots_active);
    assert_eq!(before.child_slots_active, after.child_slots_active);
    assert!(after.poisoned);
    assert_eq!(
        foreign_ledger.cancellation_token().reason(),
        Some(CancellationReason::OwnerFailure)
    );
    let reservations = foreign
        .store
        .list_coding_budget_reservations(&foreign.job, None, 50)
        .await
        .unwrap();
    assert_eq!(reservations.items.len(), 1);
    assert!(reservations.items[0].settlement.is_none());
    owner.close().await;
    foreign.close().await;
}

#[tokio::test]
async fn failed_durable_settlement_write_poisons_and_keeps_the_unknown_charge() {
    let fixture = Fixture::new("settlement-write-failure").await;
    let authority = fixture
        .add_child("agent-settle-down", "invocation-settle-down")
        .await;
    let ledger = fixture
        .ledger(limits(4, 4, 4, 2, 2, Duration::from_secs(30)))
        .await
        .unwrap();
    let session = ledger
        .begin_child(
            authority.agent_id(),
            authority.invocation_id(),
            ChildCountLimits {
                logical_requests: 2,
                tool_calls: 1,
            },
        )
        .await
        .unwrap();
    let reservations = fixture
        .store
        .list_coding_budget_reservations(&fixture.job, None, 50)
        .await
        .unwrap();
    assert_eq!(reservations.items.len(), 1);
    fixture.store.close().await;
    let result = ledger.finish_child(session, EffectOutcome::Succeeded).await;
    assert!(matches!(result, Err(EffectAdmissionError::OwnerFailure(_))));
    let snapshot = ledger.snapshot().await;
    assert!(snapshot.poisoned);
    assert_eq!(snapshot.child_slots_active, 1);
    assert_eq!(snapshot.logical_requests, 0);
    assert_eq!(
        ledger.cancellation_token().reason(),
        Some(CancellationReason::OwnerFailure)
    );
    let reopened = SqliteStateStore::new(fixture.database.clone());
    let durable = reopened
        .list_coding_budget_reservations(&fixture.job, None, 50)
        .await
        .unwrap();
    assert_eq!(durable.items.len(), 1);
    assert!(durable.items[0].settlement.is_none());
    reopened.close().await;
    fixture.close().await;
}
