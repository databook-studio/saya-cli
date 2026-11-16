use std::time::Duration;

use saya_agent::{EffectAdmission, EffectAdmissionError, EffectOutcome, EffectSettlement};
use saya_harness::coding::budget::{BudgetLedgerError, EffectiveBudgetLimits, JobBudgetLedger};
use saya_types::CodingBudgetPolicy;

use super::support::{Fixture, limits};

#[tokio::test]
async fn uncapped_policy_keeps_finite_slots_and_restores_from_its_v2_record() {
    let fixture = Fixture::new("uncapped-policy-resume").await;
    let ledger = JobBudgetLedger::new_uncapped(fixture.store.clone(), fixture.job.clone(), 2, 2)
        .await
        .unwrap();
    assert_eq!(
        ledger.limits().await,
        EffectiveBudgetLimits {
            logical_requests: None,
            transport_attempts: None,
            tool_calls: None,
            model_slots: 2,
            child_slots: 2,
            deadline: None,
        }
    );
    assert_eq!(ledger.remaining().await, None);
    assert!(matches!(
        fixture
            .store
            .get_coding_budget_policy(&fixture.job)
            .await
            .unwrap()
            .unwrap()
            .policy,
        CodingBudgetPolicy::UncappedV2(_)
    ));

    let root = ledger.root_view();
    let first = root.admit_model_request().await.unwrap();
    let second = root.admit_model_request().await.unwrap();
    assert!(matches!(
        root.admit_model_request().await,
        Err(EffectAdmissionError::Denied)
    ));
    for permit in [first, second] {
        root.settle(
            permit,
            EffectSettlement::ModelRequest {
                outcome: EffectOutcome::NotStarted,
                reported_usage: None,
                usage_complete: false,
            },
        )
        .await
        .unwrap();
    }
    for _ in 0..3 {
        let permit = root.admit_model_request().await.unwrap();
        root.settle(
            permit,
            EffectSettlement::ModelRequest {
                outcome: EffectOutcome::NotStarted,
                reported_usage: None,
                usage_complete: false,
            },
        )
        .await
        .unwrap();
    }
    assert_eq!(ledger.snapshot().await.logical_requests, 5);
    drop(root);
    drop(ledger);
    fixture.close_store().await;

    let store = fixture.reopen();
    let bounded = JobBudgetLedger::resume(
        store.clone(),
        fixture.job.clone(),
        limits(1, 1, 1, 2, 2, Duration::from_secs(30)),
    )
    .await;
    assert!(matches!(bounded, Err(BudgetLedgerError::PolicyMismatch)));
    let resumed = JobBudgetLedger::resume_uncapped(store.clone(), fixture.job.clone())
        .await
        .unwrap();
    assert_eq!(
        resumed.limits().await,
        EffectiveBudgetLimits {
            logical_requests: None,
            transport_attempts: None,
            tool_calls: None,
            model_slots: 2,
            child_slots: 2,
            deadline: None,
        }
    );
    assert_eq!(resumed.snapshot().await.logical_requests, 5);
    let permit = resumed.root_view().admit_model_request().await.unwrap();
    resumed
        .root_view()
        .settle(
            permit,
            EffectSettlement::ModelRequest {
                outcome: EffectOutcome::NotStarted,
                reported_usage: None,
                usage_complete: false,
            },
        )
        .await
        .unwrap();
    assert_eq!(resumed.snapshot().await.logical_requests, 6);
    store.close().await;
    fixture.close().await;
}

#[tokio::test]
async fn bounded_v1_policy_cannot_resume_as_uncapped() {
    let fixture = Fixture::new("bounded-policy-not-uncapped").await;
    let configured = limits(3, 4, 5, 2, 2, Duration::from_secs(30));
    drop(fixture.ledger(configured).await.unwrap());
    fixture.close_store().await;
    let store = fixture.reopen();
    assert!(matches!(
        JobBudgetLedger::resume_uncapped(store.clone(), fixture.job.clone()).await,
        Err(BudgetLedgerError::PolicyMismatch)
    ));
    assert!(matches!(
        store
            .get_coding_budget_policy(&fixture.job)
            .await
            .unwrap()
            .unwrap()
            .policy,
        CodingBudgetPolicy::BoundedV1(_)
    ));
    store.close().await;
    fixture.close().await;
}

#[tokio::test]
async fn uncapped_policy_keeps_the_recorded_child_slot_ceiling() {
    let fixture = Fixture::new("uncapped-child-slots").await;
    let ledger = JobBudgetLedger::new_uncapped(fixture.store.clone(), fixture.job.clone(), 3, 2)
        .await
        .unwrap();
    let first_authority = fixture.add_child("agent-first", "invocation-first").await;
    let second_authority = fixture.add_child("agent-second", "invocation-second").await;
    let third_authority = fixture.add_child("agent-third", "invocation-third").await;
    let first = ledger
        .begin_child_uncapped(first_authority.agent_id(), first_authority.invocation_id())
        .await
        .unwrap();
    let second = ledger
        .begin_child_uncapped(
            second_authority.agent_id(),
            second_authority.invocation_id(),
        )
        .await
        .unwrap();
    assert!(matches!(
        ledger
            .begin_child_uncapped(third_authority.agent_id(), third_authority.invocation_id(),)
            .await,
        Err(EffectAdmissionError::Denied)
    ));
    assert_eq!(ledger.snapshot().await.child_slots_active, 2);
    ledger
        .finish_child(first, EffectOutcome::NotStarted)
        .await
        .unwrap();
    ledger
        .finish_child(second, EffectOutcome::NotStarted)
        .await
        .unwrap();
    assert_eq!(ledger.snapshot().await.child_slots_active, 0);
    fixture.close().await;
}
