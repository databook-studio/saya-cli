use super::*;
use saya_types::{
    CheckoutOperationState, CheckoutOperationV1, CodingEventKind, GitObjectId, ManagedCheckoutId,
    ManagedCheckoutLifecycle, ManagedCheckoutPath, ManagedCheckoutRecordData,
    ManagedCheckoutRecordV1, RepositoryMutationAction, Sha256Digest,
};

pub(in crate::checkout_upgrade) async fn create_completed_checkout(
    seed: &mut CodingSeed,
) -> (
    ManagedCheckoutRecordV1,
    CheckoutOperationV1,
    CodingEventV1,
    CodingEventV1,
) {
    let checkout_id = ManagedCheckoutId::parse("checkout-v13-preserved").unwrap();
    let create_sequence = NonZeroU64::new(4).unwrap();
    let ownership = ManagedCheckoutRecordV1::new(ManagedCheckoutRecordData {
        job_id: seed.job_id.clone(),
        agent_id: Some(seed.child.agent_id.clone()),
        managed_checkout_id: checkout_id.clone(),
        repository_id: seed.repository_id.clone(),
        common_dir_sha256: Sha256Digest::parse(&"a".repeat(64)).unwrap(),
        managed_path: ManagedCheckoutPath::new("/tmp/saya-r13b-preserved-checkout").unwrap(),
        base_commit: GitObjectId::parse(&"b".repeat(40)).unwrap(),
        snapshot_id: seed.snapshot_id.clone(),
        snapshot_sha256: Sha256Digest::parse(&"c".repeat(64)).unwrap(),
        created_by_sequence: create_sequence,
        lifecycle: ManagedCheckoutLifecycle::Creating,
    })
    .unwrap();
    let create_operation = CheckoutOperationV1::new(
        seed.job_id.clone(),
        create_sequence,
        checkout_id.clone(),
        RepositoryMutationAction::CreateCheckout,
        CheckoutOperationState::Intent,
    )
    .unwrap();
    let create_event = event(
        4,
        &seed.job_id,
        Some((seed.child.agent_id.clone(), None)),
        CodingEventKind::CheckoutCreateIntent {
            checkout: Box::new(ownership.clone()),
            operation: Box::new(create_operation),
        },
    );
    seed.store
        .record_checkout_create_intent(create_event.clone())
        .await
        .unwrap();

    let completed_operation = CheckoutOperationV1::new(
        seed.job_id.clone(),
        create_sequence,
        checkout_id,
        RepositoryMutationAction::CreateCheckout,
        CheckoutOperationState::Completed,
    )
    .unwrap();
    let outcome_event = event(
        5,
        &seed.job_id,
        Some((seed.child.agent_id.clone(), None)),
        CodingEventKind::CheckoutOperationOutcome {
            operation: Box::new(completed_operation.clone()),
        },
    );
    seed.store
        .record_checkout_operation_outcome(outcome_event.clone())
        .await
        .unwrap();
    (ownership, completed_operation, create_event, outcome_event)
}
