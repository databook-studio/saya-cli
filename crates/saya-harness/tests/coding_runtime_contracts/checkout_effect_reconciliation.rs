#![cfg(unix)]
#[allow(clippy::duplicate_mod)] // intentional: this test group links its own private copy of the shared fixture (own statics)
#[path = "cleanup_support.rs"]
mod cleanup_support;
#[allow(clippy::duplicate_mod)] // intentional: this test group links its own private copy of the shared fixture (own statics)
#[allow(dead_code)]
#[path = "managed_worktree_creation/create_support.rs"]
mod create_support;
#[allow(clippy::duplicate_mod)] // intentional: this test group links its own private copy of the shared fixture (own statics)
#[allow(dead_code)]
#[path = "worktree_inventory_support.rs"]
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

#[tokio::test]
async fn reconcile_create_after_real_git_effect_before_outcome_preserves_original() {
    use saya_harness::coding::{snapshot::materialize_snapshot, worktree::CreateCheckoutError};
    let fixture = create_support::CreateCheckoutFixture::new("w13-create-effect").await;
    let token = CancellationToken::new();
    let id = ManagedCheckoutId::parse("interrupted-create").unwrap();
    let original = std::fs::read(fixture.repo_root.join("tracked.txt")).unwrap();
    let (object, bytes) = fixture.corrupt_first_object();
    let error = create_managed_checkout(&fixture.input(&id), &token)
        .await
        .unwrap_err();
    assert!(matches!(error, CreateCheckoutError::Snapshot(_)));
    let record = fixture.record(&id).await.unwrap();
    assert_eq!(record.current_lifecycle, ManagedCheckoutLifecycle::Creating);
    let path = std::path::PathBuf::from(fixture.service.managed_path(&id).unwrap().as_str());
    assert!(path.join(".git").is_file());
    std::fs::write(object, bytes).unwrap();
    materialize_snapshot(
        &fixture.artifacts,
        fixture.plan.snapshot_id(),
        &path,
        &token,
    )
    .unwrap();
    fixture
        .store
        .record_checkout_plan_bytes(
            &fixture.job,
            &id,
            fixture.plan.manifest_sha256(),
            fixture.plan.canonical_bytes(),
        )
        .await
        .unwrap();
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
        CheckoutReconciliation::CreateCompleted
    );
    assert_eq!(
        fixture.record(&id).await.unwrap().current_lifecycle,
        ManagedCheckoutLifecycle::Active
    );
    let inventory = fixture
        .service
        .inventory_repository(&fixture.repository, &token)
        .await
        .unwrap();
    assert_eq!(
        inventory
            .worktrees()
            .iter()
            .filter(|worktree| worktree.path() == path)
            .count(),
        1
    );
    assert_eq!(
        std::fs::read(fixture.repo_root.join("tracked.txt")).unwrap(),
        original
    );
}

#[tokio::test]
async fn reconcile_refuses_when_managed_path_is_not_a_directory() {
    use saya_harness::coding::worktree::CreateCheckoutError;
    let fixture = create_support::CreateCheckoutFixture::new("w13-probe-file-path").await;
    let token = CancellationToken::new();
    let id = ManagedCheckoutId::parse("interrupted-create").unwrap();
    let original = std::fs::read(fixture.repo_root.join("tracked.txt")).unwrap();
    let (object, bytes) = fixture.corrupt_first_object();
    let error = create_managed_checkout(&fixture.input(&id), &token)
        .await
        .unwrap_err();
    assert!(matches!(error, CreateCheckoutError::Snapshot(_)));
    let record = fixture.record(&id).await.unwrap();
    assert_eq!(record.current_lifecycle, ManagedCheckoutLifecycle::Creating);
    let path = std::path::PathBuf::from(fixture.service.managed_path(&id).unwrap().as_str());
    assert!(path.is_dir());
    std::fs::write(object, bytes).unwrap();
    std::fs::remove_dir_all(&path).unwrap();
    std::fs::write(&path, "not a checkout directory").unwrap();
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
        CheckoutReconciliation::RequireOperator
    );
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "not a checkout directory"
    );
    assert_eq!(
        fixture.record(&id).await.unwrap().current_lifecycle,
        ManagedCheckoutLifecycle::Creating
    );
    assert_eq!(
        std::fs::read(fixture.repo_root.join("tracked.txt")).unwrap(),
        original
    );
}

#[tokio::test]
async fn reconcile_requires_operator_for_an_uncertain_checkout() {
    use saya_harness::coding::{git::GitError, worktree::CreateCheckoutError};
    let fixture = create_support::CreateCheckoutFixture::new("w13-uncertain").await;
    let token = CancellationToken::new();
    let id = ManagedCheckoutId::parse("uncertain-create").unwrap();
    let original = std::fs::read(fixture.repo_root.join("tracked.txt")).unwrap();
    let path = std::path::PathBuf::from(fixture.service.managed_path(&id).unwrap().as_str());
    std::fs::create_dir_all(&path).unwrap();
    std::fs::write(path.join("marker.txt"), "operator evidence").unwrap();
    let error = create_managed_checkout(&fixture.input(&id), &token)
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        CreateCheckoutError::Git(GitError::MutationOutcomeUnknown { .. })
    ));
    let record = fixture.record(&id).await.unwrap();
    assert_eq!(
        record.current_lifecycle,
        ManagedCheckoutLifecycle::Uncertain
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
        CheckoutReconciliation::RequireOperator
    );
    assert_eq!(
        fixture.record(&id).await.unwrap().current_lifecycle,
        ManagedCheckoutLifecycle::Uncertain
    );
    assert_eq!(
        std::fs::read_to_string(path.join("marker.txt")).unwrap(),
        "operator evidence"
    );
    assert_eq!(
        std::fs::read(fixture.repo_root.join("tracked.txt")).unwrap(),
        original
    );
}
