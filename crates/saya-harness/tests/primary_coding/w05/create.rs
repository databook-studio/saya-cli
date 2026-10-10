//! W05c2 `create_managed_checkout` acceptance at the public boundary
//! (authored, not executed): the failure-preservation proof, the
//! cancellation posture, pre-existing path adoption and refusal, terminal
//! replay refusal, and idempotent replay, per the fixed W05C2B-TASK API.

use std::{fs, num::NonZeroU64, path::Path};

use saya_agent::CancellationToken;
use saya_harness::coding::git::GitError;
use saya_harness::coding::snapshot::SnapshotError;
use saya_harness::coding::worktree::{CreateCheckoutError, create_managed_checkout};
use saya_types::{
    CheckoutOperationState, CheckoutOperationV1, CodingEventKind, CodingEventV1, ManagedCheckoutId,
    ManagedCheckoutLifecycle, RepositoryMutationAction,
};

use super::create_support::CreateCheckoutFixture;
use super::support::{git_args, run_git};

/// The Failed create outcome for the open operation, recorded through the
/// sequencer's real store outcome path.
fn outcome_event(
    fixture: &CreateCheckoutFixture,
    sequence: u64,
    operation_sequence: NonZeroU64,
    checkout_id: &ManagedCheckoutId,
) -> CodingEventV1 {
    let operation = CheckoutOperationV1::new(
        fixture.job.clone(),
        operation_sequence,
        checkout_id.clone(),
        RepositoryMutationAction::CreateCheckout,
        CheckoutOperationState::Failed,
    )
    .unwrap();
    let kind = CodingEventKind::CheckoutOperationOutcome {
        operation: Box::new(operation),
    };
    CodingEventV1::new(
        NonZeroU64::new(sequence).unwrap(),
        fixture.job.clone(),
        None,
        None,
        kind,
    )
    .unwrap()
}

/// The packet's first failing proof: a corrupted stored object fails the
/// create with the typed snapshot error while the original repository's
/// HEAD, status, tracked working bytes, and raw `.git/index` bytes stay
/// byte-identical, the recoverable intent stays open over the `.git`-only
/// worktree, and the retry with the same checkout id completes exactly
/// once.
#[tokio::test]
async fn checkout_create_failure_preserves_original() {
    let fixture = CreateCheckoutFixture::new("w05-create-preserve").await;
    let checkout = ManagedCheckoutId::parse("checkout-w05-preserve").unwrap();
    let managed = fixture.service.managed_path(&checkout).unwrap();
    let head_before = fixture.git_output(&fixture.repo_root, &["rev-parse", "HEAD"]);
    let status_before = fixture.git_output(&fixture.repo_root, &["status", "--porcelain"]);
    let tracked_before = fs::read(fixture.repo_root.join("tracked.txt")).unwrap();
    let index_path = fixture.repo_root.join(".git").join("index");
    let index_before = fs::read(&index_path).ok();
    let (object, object_bytes) = fixture.corrupt_first_object();
    let error = create_managed_checkout(&fixture.input(&checkout), &CancellationToken::new())
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        CreateCheckoutError::Snapshot(SnapshotError::ObjectDigestMismatch)
    ));
    let head_after = fixture.git_output(&fixture.repo_root, &["rev-parse", "HEAD"]);
    let status_after = fixture.git_output(&fixture.repo_root, &["status", "--porcelain"]);
    assert_eq!(head_after, head_before);
    assert_eq!(status_after, status_before);
    let tracked_after = fs::read(fixture.repo_root.join("tracked.txt")).unwrap();
    assert_eq!(tracked_after, tracked_before);
    assert_eq!(fs::read(&index_path).ok(), index_before);
    let record = fixture.record(&checkout).await.expect("intent row");
    let lifecycle = record.current_lifecycle;
    assert!(matches!(lifecycle, ManagedCheckoutLifecycle::Creating));
    let state = record.latest_operation.state();
    assert!(matches!(state, CheckoutOperationState::Intent));
    let mut entries: Vec<String> = fs::read_dir(Path::new(managed.as_str()))
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    entries.sort();
    assert_eq!(entries, vec![".git"]);
    fs::write(&object, &object_bytes).unwrap();
    let created = create_managed_checkout(&fixture.input(&checkout), &CancellationToken::new())
        .await
        .unwrap();
    assert!(created.receipt().is_some());
    assert_eq!(fs::read(&index_path).ok(), index_before);
    let record = fixture.record(&checkout).await.unwrap();
    let lifecycle = record.current_lifecycle;
    assert!(matches!(lifecycle, ManagedCheckoutLifecycle::Active));
    let listing = fixture.git_output(&fixture.repo_root, &["worktree", "list"]);
    assert_eq!(listing.lines().count(), 2);
    assert_eq!(listing.matches(managed.as_str()).count(), 1);
    for entry in fixture.plan.entries() {
        let relative = entry.path().as_str();
        assert_eq!(
            fs::read(Path::new(managed.as_str()).join(relative)).unwrap(),
            fs::read(fixture.repo_root.join(relative)).unwrap()
        );
    }
    let stored = fixture
        .store
        .get_checkout_plan_bytes(&fixture.job, &checkout)
        .await
        .unwrap()
        .expect("plan bytes");
    let digest = fixture.plan.manifest_sha256().as_str();
    assert_eq!(stored.plan_sha256.as_str(), digest);
    assert_eq!(stored.plan_bytes, fixture.plan.canonical_bytes());
}

/// A pre-cancelled token is observed during the pre-intent identity
/// re-establishment (inventory precedes the sequencer allocation), so the
/// typed cancellation error returns with no checkout row and no mutation.
#[tokio::test]
async fn intent_is_durable_before_mutation() {
    let fixture = CreateCheckoutFixture::new("w05-create-cancel").await;
    let checkout = ManagedCheckoutId::parse("checkout-w05-cancel").unwrap();
    let cancelled = CancellationToken::new();
    cancelled.cancel();
    let error = create_managed_checkout(&fixture.input(&checkout), &cancelled)
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        CreateCheckoutError::Git(GitError::Cancelled)
    ));
    assert!(fixture.record(&checkout).await.is_none());
}

/// A real no-checkout worktree of the same base pre-created at the managed
/// path is adopted without a duplicate registration; a plain directory at
/// the managed path refuses with the record retained (Uncertain outcome).
#[tokio::test]
async fn pre_existing_path_no_duplicate_ref() {
    let fixture = CreateCheckoutFixture::new("w05-create-adopt").await;
    let checkout = ManagedCheckoutId::parse("checkout-w05-adopt").unwrap();
    let managed = fixture.service.managed_path(&checkout).unwrap();
    run_git(
        &fixture.git,
        fixture.temp.path(),
        git_args(
            &fixture.repo_root,
            &[
                "worktree",
                "add",
                "--no-checkout",
                "--detach",
                "--",
                managed.as_str(),
                fixture.base_commit.as_str(),
            ],
        ),
    );
    let created = create_managed_checkout(&fixture.input(&checkout), &CancellationToken::new())
        .await
        .unwrap();
    assert!(created.receipt().is_some());
    let listing = fixture.git_output(&fixture.repo_root, &["worktree", "list"]);
    assert_eq!(listing.matches(managed.as_str()).count(), 1);

    let fixture = CreateCheckoutFixture::new("w05-create-plain").await;
    let checkout = ManagedCheckoutId::parse("checkout-w05-plain").unwrap();
    let managed = fixture.service.managed_path(&checkout).unwrap();
    fs::create_dir_all(Path::new(managed.as_str())).unwrap();
    let error = create_managed_checkout(&fixture.input(&checkout), &CancellationToken::new())
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        CreateCheckoutError::Git(GitError::MutationOutcomeUnknown { .. })
    ));
    let record = fixture.record(&checkout).await.expect("retained");
    let lifecycle = record.current_lifecycle;
    assert!(matches!(lifecycle, ManagedCheckoutLifecycle::Uncertain));
}

/// A recorded Failed outcome settles the checkout id: the retry create with
/// the same id is refused as terminal.
#[tokio::test]
async fn terminal_checkout_id_refuses_replay() {
    let fixture = CreateCheckoutFixture::new("w05-create-terminal").await;
    let checkout = ManagedCheckoutId::parse("checkout-w05-terminal").unwrap();
    let (object, object_bytes) = fixture.corrupt_first_object();
    let error = create_managed_checkout(&fixture.input(&checkout), &CancellationToken::new())
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        CreateCheckoutError::Snapshot(SnapshotError::ObjectDigestMismatch)
    ));
    fs::write(&object, &object_bytes).unwrap();
    let open = fixture.record(&checkout).await.expect("intent row");
    let lifecycle = open.current_lifecycle;
    assert!(matches!(lifecycle, ManagedCheckoutLifecycle::Creating));
    let operation_sequence = open.latest_operation.operation_sequence();
    fixture
        .sequencer
        .append_checkout_outcome(|sequence| {
            outcome_event(&fixture, sequence, operation_sequence, &checkout)
        })
        .await
        .unwrap();
    let settled = fixture.record(&checkout).await.expect("settled");
    let lifecycle = settled.current_lifecycle;
    assert!(matches!(lifecycle, ManagedCheckoutLifecycle::Failed));
    let error = create_managed_checkout(&fixture.input(&checkout), &CancellationToken::new())
        .await
        .unwrap_err();
    assert!(matches!(error, CreateCheckoutError::Terminal));
}

/// Replaying an active checkout is idempotent: the second create verifies
/// the worktree and returns no receipt.
#[tokio::test]
async fn replay_of_active_checkout_is_idempotent() {
    let fixture = CreateCheckoutFixture::new("w05-create-replay").await;
    let checkout = ManagedCheckoutId::parse("checkout-w05-replay").unwrap();
    let first = create_managed_checkout(&fixture.input(&checkout), &CancellationToken::new())
        .await
        .unwrap();
    assert!(first.receipt().is_some());
    let second = create_managed_checkout(&fixture.input(&checkout), &CancellationToken::new())
        .await
        .unwrap();
    assert!(second.receipt().is_none());
    let record = fixture.record(&checkout).await.unwrap();
    let lifecycle = record.current_lifecycle;
    assert!(matches!(lifecycle, ManagedCheckoutLifecycle::Active));
}
