use super::support::{Fixture, assert_lifecycle};
use saya_store::StoreError;
use saya_types::{
    CheckoutOperationState, CodingEventKind, CodingEventV1, JobState, ManagedCheckoutLifecycle,
    RepositoryMutationAction,
};
use std::num::NonZeroU64;

#[tokio::test]
async fn failed_create_outcome_remains_failed_metadata() {
    let fixture = Fixture::new("failed-create").await;
    fixture.create(2, "checkout-r14-failed").await;
    let failed = fixture.operation_event(
        3,
        2,
        "checkout-r14-failed",
        RepositoryMutationAction::CreateCheckout,
        CheckoutOperationState::Failed,
        None,
    );
    fixture
        .store
        .record_checkout_operation_outcome(failed)
        .await
        .unwrap();
    assert_lifecycle(
        &fixture,
        "checkout-r14-failed",
        ManagedCheckoutLifecycle::Failed,
    )
    .await;
    fixture.close().await;
}

#[tokio::test]
async fn uncertain_outcome_blocks_a_later_checkout_intent() {
    let fixture = Fixture::new("uncertain").await;
    fixture.create(2, "checkout-r14-uncertain").await;
    let uncertain = fixture.operation_event(
        3,
        2,
        "checkout-r14-uncertain",
        RepositoryMutationAction::CreateCheckout,
        CheckoutOperationState::Uncertain,
        None,
    );
    fixture
        .store
        .record_checkout_operation_outcome(uncertain)
        .await
        .unwrap();
    assert_lifecycle(
        &fixture,
        "checkout-r14-uncertain",
        ManagedCheckoutLifecycle::Uncertain,
    )
    .await;

    let remove = fixture.operation_event(
        4,
        4,
        "checkout-r14-uncertain",
        RepositoryMutationAction::RemoveCheckout,
        CheckoutOperationState::Intent,
        None,
    );
    assert_eq!(
        fixture
            .store
            .record_checkout_operation_intent(remove)
            .await
            .unwrap_err(),
        StoreError::Conflict
    );
    assert_eq!(
        fixture
            .store
            .list_coding_events(&fixture.job, None, 100)
            .await
            .unwrap()
            .events
            .len(),
        3
    );
    fixture.close().await;
}

#[tokio::test]
async fn removal_and_its_late_outcome_do_not_reopen_terminal_job() {
    let fixture = Fixture::new("terminal-remove").await;
    fixture.create(2, "checkout-r14-terminal").await;
    fixture.complete_create(3, 2, "checkout-r14-terminal").await;
    let terminal = CodingEventV1::new(
        NonZeroU64::new(4).unwrap(),
        fixture.job.clone(),
        None,
        None,
        CodingEventKind::StateChanged {
            state: JobState::Completed,
        },
    )
    .unwrap();
    fixture.store.append_coding_event(terminal).await.unwrap();
    let remove = fixture.operation_event(
        5,
        5,
        "checkout-r14-terminal",
        RepositoryMutationAction::RemoveCheckout,
        CheckoutOperationState::Intent,
        None,
    );
    fixture
        .store
        .record_checkout_operation_intent(remove)
        .await
        .unwrap();
    let removed = fixture.operation_event(
        6,
        5,
        "checkout-r14-terminal",
        RepositoryMutationAction::RemoveCheckout,
        CheckoutOperationState::Completed,
        None,
    );
    fixture
        .store
        .record_checkout_operation_outcome(removed)
        .await
        .unwrap();

    assert_eq!(
        fixture
            .store
            .get_coding_job(&fixture.job)
            .await
            .unwrap()
            .unwrap()
            .state,
        JobState::Completed
    );
    assert_lifecycle(
        &fixture,
        "checkout-r14-terminal",
        ManagedCheckoutLifecycle::Removed,
    )
    .await;
    fixture.close().await;
}
