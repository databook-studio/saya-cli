use std::time::Duration;

use saya_agent::{
    CancellationReason, EffectAdmission, EffectAdmissionError, EffectCallKind, EffectOutcome,
    EffectPermit, EffectSettlement, LocalStateEffect, PhysicalAttempt, ToolEffect,
};
use saya_harness::coding::budget::{ChildBudgetView, ChildCountLimits, JobBudgetLedger};
use saya_types::{BudgetDimension, BudgetEffectState, BudgetObservationValue};

use super::fixture::{Fixture, limits};

#[tokio::test]
async fn root_and_child_share_one_serialized_logical_count_and_model_slot() {
    let fixture = Fixture::new("shared-root-child").await;
    let authority = fixture.add_child("agent-shared", "invocation-shared").await;
    let ledger = fixture
        .ledger(limits(1, 4, 4, 1, 1, Duration::from_secs(5)))
        .await
        .unwrap();
    let root = ledger.root_view();
    let session = ledger
        .begin_child(
            authority.agent_id(),
            authority.invocation_id(),
            ChildCountLimits {
                logical_requests: 1,
                tool_calls: 4,
            },
        )
        .await
        .unwrap();
    let child = session.view();
    let (root_result, child_result) =
        tokio::join!(root.admit_model_request(), child.admit_model_request(),);
    assert_eq!(
        (root_result.is_ok() as usize) + (child_result.is_ok() as usize),
        1
    );
    assert_eq!(
        (matches!(&root_result, Err(EffectAdmissionError::Denied)) as usize)
            + (matches!(&child_result, Err(EffectAdmissionError::Denied)) as usize),
        1
    );

    if let Ok(permit) = root_result {
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
    if let Ok(permit) = child_result {
        child
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
    }
    ledger
        .finish_child(session, EffectOutcome::NotStarted)
        .await
        .unwrap();
    let snapshot = ledger.snapshot().await;
    assert_eq!(snapshot.logical_requests, 1);
    assert_eq!(snapshot.model_slots_active, 0);
    assert_eq!(snapshot.child_slots_active, 0);
    assert_eq!(
        fixture
            .store
            .list_coding_budget_reservations(&fixture.job, None, 10)
            .await
            .unwrap()
            .items
            .len(),
        2
    );
    fixture.close().await;
}

#[tokio::test]
async fn one_tool_batch_has_one_durable_amount_and_dropped_member_stays_unknown() {
    let fixture = Fixture::new("tool-batch-group").await;
    let authority = fixture.add_child("agent-batch", "invocation-batch").await;
    let ledger = fixture
        .ledger(limits(4, 4, 2, 1, 1, Duration::from_secs(5)))
        .await
        .unwrap();
    let session = ledger
        .begin_child(
            authority.agent_id(),
            authority.invocation_id(),
            ChildCountLimits {
                logical_requests: 4,
                tool_calls: 2,
            },
        )
        .await
        .unwrap();
    let child = session.view();
    let effect = ToolEffect {
        database_data: false,
        external_side_effect: false,
        requires_approval: false,
        local_state: LocalStateEffect::None,
    };
    let permits = child.admit_tool_batch(&[effect, effect]).await.unwrap();
    assert_eq!(permits.len(), 2);
    let reservations = fixture
        .store
        .list_coding_budget_reservations(&fixture.job, None, 10)
        .await
        .unwrap();
    let tool_rows = reservations
        .items
        .iter()
        .filter(|row| {
            row.reservation
                .charges()
                .iter()
                .any(|charge| charge.dimension() == BudgetDimension::ToolCalls)
        })
        .collect::<Vec<_>>();
    assert_eq!(tool_rows.len(), 1);
    assert_eq!(tool_rows[0].reservation.charges()[0].reserved().get(), 2);
    child
        .settle(
            permits[0],
            EffectSettlement::Tool {
                outcome: EffectOutcome::Succeeded,
            },
        )
        .await
        .unwrap();
    drop(permits[1]);
    let before_finish = ledger.snapshot().await;
    assert_eq!(before_finish.tool_calls, 2);
    assert_eq!(before_finish.tool_permits_pending, 1);
    ledger
        .finish_child(session, EffectOutcome::Unknown)
        .await
        .unwrap();

    let after_finish = ledger.snapshot().await;
    assert_eq!(after_finish.tool_calls, 2);
    assert_eq!(after_finish.unknown_tool_calls, 2);
    let tool_row = fixture
        .store
        .get_coding_budget_reservation(
            &fixture.job,
            tool_rows[0].reservation.reservation_sequence(),
        )
        .await
        .unwrap()
        .unwrap();
    let settlement = tool_row.settlement.unwrap();
    assert_eq!(settlement.effect_state(), BudgetEffectState::Uncertain);
    assert_eq!(
        settlement.observations()[0].value(),
        BudgetObservationValue::Unknown
    );
    assert_eq!(after_finish.measured_tool_calls, 1);
    fixture.close().await;
}

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
