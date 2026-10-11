#![cfg(unix)]
#[path = "cleanup_support.rs"]
mod cleanup_support;
#[allow(dead_code)]
#[path = "w05/create_support.rs"]
mod create_support;
#[allow(dead_code)]
#[path = "w02_support.rs"]
mod support;
use saya_agent::CancellationToken;
use saya_harness::coding::{
    budget::JobBudgetLedger,
    worktree::{
        CheckoutReconciliation, CleanupInput, create_managed_checkout, reconcile_managed_checkout,
    },
};
use saya_types::{
    CheckoutOperationState, CheckoutOperationV1, CodingEventKind, CodingEventV1, ManagedCheckoutId,
    ManagedCheckoutLifecycle, RepositoryMutationAction,
};
use std::num::NonZeroU64;

#[tokio::test]
async fn reconcile_remove_after_git_effect_repairs_event_without_replaying_effect() {
    let fixture = cleanup_support::clean_fixture("w13-reconcile").await;
    let token = CancellationToken::new();
    let id = ManagedCheckoutId::parse("checkout").unwrap();
    create_managed_checkout(&fixture.input(&id), &token)
        .await
        .unwrap();
    fixture
        .sequencer
        .append_checkout_operation_intent(|number| {
            let sequence = NonZeroU64::new(number).unwrap();
            let operation = CheckoutOperationV1::new(
                fixture.job.clone(),
                sequence,
                id.clone(),
                RepositoryMutationAction::RemoveCheckout,
                CheckoutOperationState::Intent,
            )
            .unwrap();
            CodingEventV1::new(
                sequence,
                fixture.job.clone(),
                None,
                None,
                CodingEventKind::CheckoutOperationIntent {
                    operation: Box::new(operation),
                },
            )
            .unwrap()
        })
        .await
        .unwrap();
    let worktree = fixture
        .service
        .adopt_worktree(&fixture.repository, &fixture.base_commit, &id, &token)
        .await
        .unwrap();
    fixture
        .service
        .remove_worktree(&worktree, &token)
        .await
        .unwrap();
    assert_eq!(
        fixture.record(&id).await.unwrap().current_lifecycle,
        ManagedCheckoutLifecycle::Removing
    );
    let ledger = JobBudgetLedger::new(
        fixture.store.clone(),
        fixture.job.clone(),
        create_support::limits(),
    )
    .await
    .unwrap();
    let quiescence = ledger.try_quiescent().await.unwrap();
    let sequencer = ledger.sequencer();
    let input = CleanupInput {
        ownership: cleanup_support::ownership(&fixture, &id),
        sequencer: &sequencer,
        quiescence: &quiescence,
    };
    assert_eq!(
        reconcile_managed_checkout(&input, &token).await.unwrap(),
        CheckoutReconciliation::RemoveCompleted
    );
    assert_eq!(
        fixture.record(&id).await.unwrap().current_lifecycle,
        ManagedCheckoutLifecycle::Removed
    );
    assert_eq!(
        reconcile_managed_checkout(&input, &token).await.unwrap(),
        CheckoutReconciliation::Unchanged
    );
    assert!(fixture.repo_root.join("tracked.txt").exists());
}
