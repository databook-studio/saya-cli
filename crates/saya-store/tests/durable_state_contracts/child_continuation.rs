#[path = "child_continuation/support.rs"]
mod support;
use super::token_charge_reservations::reservation;
use saya_types::{
    ApprovalNetworkDestination, ApprovalNetworkScheme, ApprovalRequestContext, ApprovalRequestV1,
    ApprovalScope, BudgetDimension, BudgetEffectState, BudgetObservation, BudgetObservationValue,
    BudgetSettlementV1, CheckoutOperationState, CodingEventKind, RepositoryMutationAction,
};
use support::{Fixture, digest, nz};

#[tokio::test]
async fn known_completion_rotates_only_the_current_invocation() {
    let mut f = Fixture::new("follow-known").await;
    let completion = f.complete().await;
    let event = f.prepared(completion);
    f.base
        .store
        .record_coding_child_invocation(event.clone(), &f.authority)
        .await
        .unwrap();
    let child = f
        .base
        .store
        .list_coding_children(&f.base.job, None, 100)
        .await
        .unwrap()
        .items
        .pop()
        .unwrap();
    assert_eq!(
        child.current_invocation_id,
        event.invocation_id().unwrap().clone()
    );
    assert_eq!(child.state, saya_types::JobState::Queued);
    assert!(
        f.base
            .store
            .get_coding_completion(&f.base.job, completion)
            .await
            .unwrap()
            .is_some()
    );
    f.base.close().await;
}
#[tokio::test]
async fn settled_unknown_observation_refuses_rotation() {
    let mut f = Fixture::new("follow-unknown").await;
    let reservation = reservation(&f.base.job, f.next, Some(&f.child));
    let reserved = f.event(CodingEventKind::BudgetReserved {
        reservation: Box::new(reservation),
    });
    let sequence = reserved.sequence();
    f.base
        .store
        .reserve_coding_budget_fenced(reserved, Some(&f.authority))
        .await
        .unwrap();
    let settled = BudgetSettlementV1::new(
        f.base.job.clone(),
        sequence,
        nz(f.next),
        BudgetEffectState::Settled,
        vec![
            BudgetObservation::new(
                BudgetDimension::ToolCalls,
                None,
                None,
                BudgetObservationValue::Unknown,
            )
            .unwrap(),
        ],
    )
    .unwrap();
    let event = super::event_projection_storage::event(
        f.next,
        &f.base.job,
        None,
        CodingEventKind::BudgetSettled {
            settlement: Box::new(settled),
        },
    );
    f.next += 1;
    f.base.store.settle_coding_budget(event).await.unwrap();
    let completion = f.complete().await;
    let prepared = f.prepared(completion);
    f.assert_refused(prepared).await;
    f.base.close().await;
}
#[tokio::test]
async fn recorded_uncertain_checkout_outcome_refuses_rotation() {
    let mut f = Fixture::new("follow-checkout").await;
    let intent = f.base.operation_event(
        f.next,
        f.next,
        "checkout-child",
        RepositoryMutationAction::IntegrateCandidate,
        CheckoutOperationState::Intent,
        Some(f.child.agent_id.clone()),
    );
    let operation = f.next;
    f.next += 1;
    f.base
        .store
        .record_checkout_operation_intent(intent)
        .await
        .unwrap();
    let outcome = f.base.operation_event(
        f.next,
        operation,
        "checkout-child",
        RepositoryMutationAction::IntegrateCandidate,
        CheckoutOperationState::Uncertain,
        Some(f.child.agent_id.clone()),
    );
    f.next += 1;
    f.base
        .store
        .record_checkout_operation_outcome(outcome)
        .await
        .unwrap();
    let completion = f.complete().await;
    let prepared = f.prepared(completion);
    f.assert_refused(prepared).await;
    f.base.close().await;
}
#[tokio::test]
async fn pending_predecessor_approval_refuses_rotation() {
    let mut f = Fixture::new("follow-approval").await;
    let request = ApprovalRequestV1::new(
        f.base.job.clone(),
        f.child.agent_id.clone(),
        f.child.current_invocation_id.clone(),
        nz(f.next),
        ApprovalRequestContext::new(digest("a"), 1, 1, 2).unwrap(),
        ApprovalScope::network_access(
            ApprovalNetworkDestination::new(ApprovalNetworkScheme::Https, "example.com").unwrap(),
        ),
    )
    .unwrap();
    let event = f.event(CodingEventKind::ApprovalRequested {
        request: Box::new(request),
    });
    f.base
        .store
        .record_coding_approval_request(event)
        .await
        .unwrap();
    let completion = f.complete().await;
    let prepared = f.prepared(completion);
    f.assert_refused(prepared).await;
    f.base.close().await;
}
#[path = "child_continuation/adoption.rs"]
mod adoption;
#[path = "child_continuation/lifecycle.rs"]
mod lifecycle;
#[path = "child_continuation/usage_unknown.rs"]
mod usage_unknown;
