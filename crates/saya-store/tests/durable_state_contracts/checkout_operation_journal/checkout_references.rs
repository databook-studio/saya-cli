use super::support::Fixture;
use saya_store::StoreError;
use saya_types::{CheckoutOperationState, RepositoryMutationAction};

#[tokio::test]
async fn operation_intent_and_outcome_require_referenced_typed_records() {
    let fixture = Fixture::new("operation-refusal").await;
    let create = fixture.create(2, "checkout-r14-reference").await;
    assert_eq!(
        fixture
            .store
            .record_checkout_operation_intent(create)
            .await
            .unwrap_err(),
        StoreError::Invalid
    );
    let forged = fixture.operation_event(
        3,
        1,
        "checkout-r14-reference",
        RepositoryMutationAction::CreateCheckout,
        CheckoutOperationState::Completed,
        None,
    );
    assert_eq!(
        fixture
            .store
            .record_checkout_operation_outcome(forged)
            .await
            .unwrap_err(),
        StoreError::Invalid
    );
    let wrong_action = fixture.operation_event(
        3,
        2,
        "checkout-r14-reference",
        RepositoryMutationAction::RemoveCheckout,
        CheckoutOperationState::Completed,
        None,
    );
    assert_eq!(
        fixture
            .store
            .record_checkout_operation_outcome(wrong_action)
            .await
            .unwrap_err(),
        StoreError::Conflict
    );
    let correct_outcome = fixture.operation_event(
        3,
        2,
        "checkout-r14-reference",
        RepositoryMutationAction::CreateCheckout,
        CheckoutOperationState::Completed,
        None,
    );
    fixture
        .store
        .record_checkout_operation_outcome(correct_outcome)
        .await
        .unwrap();
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
