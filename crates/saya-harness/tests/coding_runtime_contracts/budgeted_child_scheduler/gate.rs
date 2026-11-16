use std::time::Duration;

use saya_agent::{BudgetDimension, CancellationReason, EffectAdmission, EffectAdmissionError};
use saya_harness::coding::budget::ChildCountLimits;

use super::fixture::{Fixture, limits};

#[tokio::test]
async fn cancelled_ledger_token_denies_admissions_without_new_reservations() {
    let fixture = Fixture::new("cancelled-gate").await;
    let authority = fixture.add_child("agent-gate", "invocation-gate").await;
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
    let before = fixture
        .store
        .list_coding_budget_reservations(&fixture.job, None, 50)
        .await
        .unwrap();
    ledger
        .cancellation_token()
        .cancel_with_reason(CancellationReason::User);
    let root = ledger.root_view();
    assert!(matches!(
        root.admit_model_request().await,
        Err(EffectAdmissionError::Denied)
    ));
    let child = session.view();
    assert!(matches!(
        child.admit_model_request().await,
        Err(EffectAdmissionError::Denied)
    ));
    let second_authority = fixture
        .add_child("agent-gate-two", "invocation-gate-two")
        .await;
    assert!(matches!(
        ledger
            .begin_child(
                second_authority.agent_id(),
                second_authority.invocation_id(),
                ChildCountLimits {
                    logical_requests: 1,
                    tool_calls: 1,
                },
            )
            .await,
        Err(EffectAdmissionError::Denied)
    ));
    let after = fixture
        .store
        .list_coding_budget_reservations(&fixture.job, None, 50)
        .await
        .unwrap();
    assert_eq!(after.items.len(), before.items.len());
    assert!(after.items.iter().all(|record| record.settlement.is_none()));
    assert_eq!(
        ledger.cancellation_token().reason(),
        Some(CancellationReason::User)
    );
    assert!(!ledger.snapshot().await.poisoned);
    fixture.close().await;
}

#[tokio::test]
async fn expired_deadline_latches_the_ledger_reason_on_refused_admission() {
    let fixture = Fixture::new("deadline-latch").await;
    let ledger = fixture
        .ledger(limits(4, 4, 4, 2, 1, Duration::from_millis(50)))
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(120)).await;
    let root = ledger.root_view();
    assert!(matches!(
        root.admit_model_request().await,
        Err(EffectAdmissionError::Denied)
    ));
    assert_eq!(
        ledger.cancellation_token().reason(),
        Some(CancellationReason::Budget(BudgetDimension::ElapsedTime))
    );
    assert!(!ledger.snapshot().await.poisoned);
    fixture.close().await;
}
