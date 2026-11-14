use std::time::Duration;

use saya_agent::{
    CancellationReason, EffectAdmission, EffectAdmissionError, EffectCallKind, EffectOutcome,
    EffectPermit, EffectSettlement, PhysicalAttempt,
};
use saya_harness::coding::budget::{ChildBudgetView, ChildCountLimits, JobBudgetLedger};

use crate::scheduler::fixture::{Fixture, limits};

struct PhysicalOwner {
    fixture: Fixture,
    ledger: JobBudgetLedger,
    child: ChildBudgetView,
    model: EffectPermit,
}

async fn physical_owner(label: &str) -> PhysicalOwner {
    let fixture = Fixture::new(label).await;
    let authority = fixture
        .add_child("agent-physical", "invocation-physical")
        .await;
    let ledger = fixture
        .ledger(limits(2, 2, 1, 1, 2, Duration::from_secs(5)))
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
    let child = session.view().clone();
    let model = child.admit_model_request().await.unwrap();
    PhysicalOwner {
        fixture,
        ledger,
        child,
        model,
    }
}

async fn settle_attempt_not_started(
    child: &ChildBudgetView,
    send: EffectPermit,
    attempt: PhysicalAttempt,
) {
    child
        .settle(
            send,
            EffectSettlement::PhysicalAttempt {
                attempt,
                outcome: EffectOutcome::NotStarted,
                reported_usage: None,
                usage_complete: false,
            },
        )
        .await
        .unwrap();
}

async fn assert_owner_refused(ledger: &JobBudgetLedger) {
    assert_eq!(
        ledger.cancellation_token().reason(),
        Some(CancellationReason::OwnerFailure)
    );
    assert!(ledger.snapshot().await.poisoned);
}

#[tokio::test]
async fn physical_attempt_must_match_live_model_parent_actor_kind_and_ordinal() {
    let owner = physical_owner("physical-owner-binding").await;
    let attempt = PhysicalAttempt::new(owner.model, EffectCallKind::Answer, 0);
    let send = owner.child.admit_physical_attempt(attempt).await.unwrap();
    settle_attempt_not_started(&owner.child, send, attempt).await;
    assert!(matches!(
        owner.child.admit_physical_attempt(attempt).await,
        Err(EffectAdmissionError::OwnerFailure(_))
    ));
    assert_owner_refused(&owner.ledger).await;
    owner.fixture.close().await;

    let owner = physical_owner("physical-owner-actor").await;
    let other = owner
        .fixture
        .add_child("agent-other", "invocation-other")
        .await;
    let other_session = owner
        .ledger
        .begin_child(
            other.agent_id(),
            other.invocation_id(),
            ChildCountLimits {
                logical_requests: 1,
                tool_calls: 1,
            },
        )
        .await
        .unwrap();
    let other_child = other_session.view().clone();
    let attempt = PhysicalAttempt::new(owner.model, EffectCallKind::Answer, 0);
    assert!(matches!(
        other_child.admit_physical_attempt(attempt).await,
        Err(EffectAdmissionError::OwnerFailure(_))
    ));
    assert_owner_refused(&owner.ledger).await;
    owner.fixture.close().await;

    let owner = physical_owner("physical-owner-kind").await;
    let attempt = PhysicalAttempt::new(owner.model, EffectCallKind::Answer, 0);
    let send = owner.child.admit_physical_attempt(attempt).await.unwrap();
    settle_attempt_not_started(&owner.child, send, attempt).await;
    let switched = PhysicalAttempt::new(owner.model, EffectCallKind::Continuation, 1);
    assert!(matches!(
        owner.child.admit_physical_attempt(switched).await,
        Err(EffectAdmissionError::OwnerFailure(_))
    ));
    assert_owner_refused(&owner.ledger).await;
    owner.fixture.close().await;

    let owner = physical_owner("physical-owner-parent").await;
    let attempt = PhysicalAttempt::new(owner.model, EffectCallKind::Answer, 0);
    let send = owner.child.admit_physical_attempt(attempt).await.unwrap();
    settle_attempt_not_started(&owner.child, send, attempt).await;
    owner
        .child
        .settle(
            owner.model,
            EffectSettlement::ModelRequest {
                outcome: EffectOutcome::NotStarted,
                reported_usage: None,
                usage_complete: false,
            },
        )
        .await
        .unwrap();
    let orphan = PhysicalAttempt::new(owner.model, EffectCallKind::Answer, 1);
    assert!(matches!(
        owner.child.admit_physical_attempt(orphan).await,
        Err(EffectAdmissionError::OwnerFailure(_))
    ));
    assert_owner_refused(&owner.ledger).await;
    owner.fixture.close().await;
}
