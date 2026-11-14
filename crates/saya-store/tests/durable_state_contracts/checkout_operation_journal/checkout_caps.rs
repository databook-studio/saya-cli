use super::support::Fixture;
use saya_store::StoreError;
use saya_types::{CheckoutOperationState, ManagedCheckoutId, RepositoryMutationAction};
use std::num::NonZeroU64;

#[tokio::test]
async fn checkout_and_operation_limits_refuse_the_first_excess_record() {
    let checkouts = Fixture::new("checkout-cap").await;
    for index in 0..256_u64 {
        checkouts
            .create(2 + index, &format!("checkout-r14-{index:03}"))
            .await;
    }
    let excess_checkout = checkouts.create_event(
        258,
        "checkout-r14-excess",
        checkouts.repository.clone(),
        checkouts.snapshot.clone(),
        None,
    );
    assert_eq!(
        checkouts
            .store
            .record_checkout_create_intent(excess_checkout)
            .await
            .unwrap_err(),
        StoreError::LimitExceeded,
    );
    assert_eq!(
        checkouts
            .store
            .get_managed_checkout(
                &checkouts.job,
                &ManagedCheckoutId::parse("checkout-r14-excess").unwrap(),
            )
            .await
            .unwrap(),
        None,
    );
    checkouts.close().await;

    let operations = Fixture::new("operation-cap").await;
    operations.create(2, "checkout-r14-operation-cap").await;
    operations
        .complete_create(3, 2, "checkout-r14-operation-cap")
        .await;
    for index in 0..9_999_u64 {
        let intent_sequence = 4 + 2 * index;
        let intent = operations.operation_event(
            intent_sequence,
            intent_sequence,
            "checkout-r14-operation-cap",
            RepositoryMutationAction::IntegrateCandidate,
            CheckoutOperationState::Intent,
            None,
        );
        operations
            .store
            .record_checkout_operation_intent(intent)
            .await
            .unwrap();
        let outcome = operations.operation_event(
            intent_sequence + 1,
            intent_sequence,
            "checkout-r14-operation-cap",
            RepositoryMutationAction::IntegrateCandidate,
            CheckoutOperationState::Completed,
            None,
        );
        operations
            .store
            .record_checkout_operation_outcome(outcome)
            .await
            .unwrap();
    }
    let excess_sequence = 20_002;
    let excess = operations.operation_event(
        excess_sequence,
        excess_sequence,
        "checkout-r14-operation-cap",
        RepositoryMutationAction::IntegrateCandidate,
        CheckoutOperationState::Intent,
        None,
    );
    assert_eq!(
        operations
            .store
            .record_checkout_operation_intent(excess)
            .await
            .unwrap_err(),
        StoreError::LimitExceeded,
    );
    assert_eq!(
        operations
            .store
            .get_checkout_operation(
                &operations.job,
                &ManagedCheckoutId::parse("checkout-r14-operation-cap").unwrap(),
                NonZeroU64::new(excess_sequence).unwrap(),
            )
            .await
            .unwrap(),
        None,
    );
    operations.close().await;
}
