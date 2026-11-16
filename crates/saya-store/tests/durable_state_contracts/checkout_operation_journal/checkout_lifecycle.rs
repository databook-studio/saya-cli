use super::support::{Fixture, assert_lifecycle};
use saya_store::CodingEventAppend;
use saya_types::{CheckoutOperationState, ManagedCheckoutLifecycle, RepositoryMutationAction};
use std::num::NonZeroU64;

#[tokio::test]
async fn typed_operations_derive_conservative_checkout_lifecycle() {
    let fixture = Fixture::new("lifecycle").await;
    fixture.create(2, "checkout-r14-lifecycle").await;
    fixture
        .complete_create(3, 2, "checkout-r14-lifecycle")
        .await;
    assert_lifecycle(
        &fixture,
        "checkout-r14-lifecycle",
        ManagedCheckoutLifecycle::Active,
    )
    .await;

    let integrate = fixture.operation_event(
        4,
        4,
        "checkout-r14-lifecycle",
        RepositoryMutationAction::IntegrateCandidate,
        CheckoutOperationState::Intent,
        None,
    );
    assert_eq!(
        fixture
            .store
            .record_checkout_operation_intent(integrate)
            .await
            .unwrap(),
        CodingEventAppend::Appended(NonZeroU64::new(4).unwrap())
    );
    assert_lifecycle(
        &fixture,
        "checkout-r14-lifecycle",
        ManagedCheckoutLifecycle::Retained,
    )
    .await;
    let competing = fixture.operation_event(
        5,
        5,
        "checkout-r14-lifecycle",
        RepositoryMutationAction::ApplyBack,
        CheckoutOperationState::Intent,
        None,
    );
    assert_eq!(
        fixture
            .store
            .record_checkout_operation_intent(competing)
            .await
            .unwrap_err(),
        saya_store::StoreError::Conflict,
    );
    let integrated = fixture.operation_event(
        5,
        4,
        "checkout-r14-lifecycle",
        RepositoryMutationAction::IntegrateCandidate,
        CheckoutOperationState::Completed,
        None,
    );
    fixture
        .store
        .record_checkout_operation_outcome(integrated)
        .await
        .unwrap();
    assert_lifecycle(
        &fixture,
        "checkout-r14-lifecycle",
        ManagedCheckoutLifecycle::Retained,
    )
    .await;

    let apply = fixture.operation_event(
        6,
        6,
        "checkout-r14-lifecycle",
        RepositoryMutationAction::ApplyBack,
        CheckoutOperationState::Intent,
        None,
    );
    fixture
        .store
        .record_checkout_operation_intent(apply)
        .await
        .unwrap();
    let applied = fixture.operation_event(
        7,
        6,
        "checkout-r14-lifecycle",
        RepositoryMutationAction::ApplyBack,
        CheckoutOperationState::Failed,
        None,
    );
    fixture
        .store
        .record_checkout_operation_outcome(applied)
        .await
        .unwrap();
    assert_lifecycle(
        &fixture,
        "checkout-r14-lifecycle",
        ManagedCheckoutLifecycle::Retained,
    )
    .await;

    let remove = fixture.operation_event(
        8,
        8,
        "checkout-r14-lifecycle",
        RepositoryMutationAction::RemoveCheckout,
        CheckoutOperationState::Intent,
        None,
    );
    fixture
        .store
        .record_checkout_operation_intent(remove)
        .await
        .unwrap();
    assert_lifecycle(
        &fixture,
        "checkout-r14-lifecycle",
        ManagedCheckoutLifecycle::Removing,
    )
    .await;
    let failed_remove = fixture.operation_event(
        9,
        8,
        "checkout-r14-lifecycle",
        RepositoryMutationAction::RemoveCheckout,
        CheckoutOperationState::Failed,
        None,
    );
    fixture
        .store
        .record_checkout_operation_outcome(failed_remove)
        .await
        .unwrap();
    assert_lifecycle(
        &fixture,
        "checkout-r14-lifecycle",
        ManagedCheckoutLifecycle::Retained,
    )
    .await;
    let remove_again = fixture.operation_event(
        10,
        10,
        "checkout-r14-lifecycle",
        RepositoryMutationAction::RemoveCheckout,
        CheckoutOperationState::Intent,
        None,
    );
    fixture
        .store
        .record_checkout_operation_intent(remove_again)
        .await
        .unwrap();
    let removed = fixture.operation_event(
        11,
        10,
        "checkout-r14-lifecycle",
        RepositoryMutationAction::RemoveCheckout,
        CheckoutOperationState::Completed,
        None,
    );
    fixture
        .store
        .record_checkout_operation_outcome(removed)
        .await
        .unwrap();
    assert_lifecycle(
        &fixture,
        "checkout-r14-lifecycle",
        ManagedCheckoutLifecycle::Removed,
    )
    .await;
    fixture.close().await;
}
