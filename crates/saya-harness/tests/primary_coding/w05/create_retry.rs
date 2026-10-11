//! W05c2 `create_managed_checkout` retry and concurrency acceptance at the
//! public boundary (authored, not executed): completing a retry over a
//! partially materialized target, refusing a retry that changes the base
//! commit, and concurrent same-id creation completing exactly once without
//! poisoning the budget ledger, per the W05C2-FIX review repairs.

use std::{fs, path::Path};

use saya_agent::CancellationToken;
use saya_harness::coding::snapshot::SnapshotError;
use saya_harness::coding::worktree::{CreateCheckoutError, create_managed_checkout};
use saya_types::{
    CheckoutOperationState, GitObjectId, ManagedCheckoutId, ManagedCheckoutLifecycle,
};

use super::create_support::CreateCheckoutFixture;
use super::support::{git_args, run_git};

/// The sorted top-level entry names of the managed directory.
fn managed_entries(managed: &str) -> Vec<String> {
    let mut entries: Vec<String> = fs::read_dir(Path::new(managed))
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    entries.sort();
    entries
}

/// The exact sorted top-level listing of a fully materialized worktree: the
/// `.git` anchor plus one top-level component per plan entry.
fn materialized_listing(fixture: &CreateCheckoutFixture) -> Vec<String> {
    let mut expected: Vec<String> = vec![".git".to_owned()];
    for entry in fixture.plan.entries() {
        let top = entry.path().as_str().split('/').next().unwrap().to_owned();
        if !expected.contains(&top) {
            expected.push(top);
        }
    }
    expected.sort();
    expected
}

/// A retry after a partial materialization completes: the corrupted object
/// fails the first create with the intent open over the `.git`-only
/// worktree; with a stray file planted in the managed directory and the
/// object restored, the same-id create reconciles the populated target back
/// to `.git`-only and completes, leaving exactly `.git` plus the snapshot
/// files with the checkout Active.
#[tokio::test]
async fn retry_after_partial_materialization_completes() {
    let fixture = CreateCheckoutFixture::new("w05-retry-materialize").await;
    let checkout = ManagedCheckoutId::parse("checkout-w05-retry-materialize").unwrap();
    let managed = fixture.service.managed_path(&checkout).unwrap();
    let (object, object_bytes) = fixture.corrupt_first_object();
    let error = create_managed_checkout(&fixture.input(&checkout), &CancellationToken::new())
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        CreateCheckoutError::Snapshot(SnapshotError::ObjectDigestMismatch)
    ));
    let record = fixture.record(&checkout).await.expect("intent row");
    assert!(matches!(
        record.current_lifecycle,
        ManagedCheckoutLifecycle::Creating
    ));
    let state = record.latest_operation.state();
    assert!(matches!(state, CheckoutOperationState::Intent));
    let stray = Path::new(managed.as_str()).join("stray-leftover.txt");
    fs::write(&stray, b"stray bytes\n").unwrap();
    fs::write(&object, &object_bytes).unwrap();
    let created = create_managed_checkout(&fixture.input(&checkout), &CancellationToken::new())
        .await
        .unwrap();
    assert!(created.receipt().is_some());
    let record = fixture.record(&checkout).await.unwrap();
    assert!(matches!(
        record.current_lifecycle,
        ManagedCheckoutLifecycle::Active
    ));
    assert_eq!(
        managed_entries(managed.as_str()),
        materialized_listing(&fixture)
    );
    for entry in fixture.plan.entries() {
        assert_eq!(
            fs::read(Path::new(managed.as_str()).join(entry.path().as_str())).unwrap(),
            fs::read(fixture.repo_root.join(entry.path().as_str())).unwrap()
        );
    }
}

/// A retry that changes the base commit is refused as an input mismatch:
/// the second create validates the caller's input against the recorded
/// ownership and refuses before touching the ledger, the worktree, or the
/// durable record, leaving the managed directory `.git`-only and the intent
/// open exactly as the first attempt left it.
#[tokio::test]
async fn retry_with_different_base_commit_is_refused() {
    let fixture = CreateCheckoutFixture::new("w05-retry-base").await;
    let checkout = ManagedCheckoutId::parse("checkout-w05-retry-base").unwrap();
    let managed = fixture.service.managed_path(&checkout).unwrap();
    let (object, object_bytes) = fixture.corrupt_first_object();
    let error = create_managed_checkout(&fixture.input(&checkout), &CancellationToken::new())
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        CreateCheckoutError::Snapshot(SnapshotError::ObjectDigestMismatch)
    ));
    fs::write(&object, &object_bytes).unwrap();
    let record = fixture.record(&checkout).await.expect("intent row");
    assert!(matches!(
        record.current_lifecycle,
        ManagedCheckoutLifecycle::Creating
    ));
    fs::write(
        fixture.repo_root.join("second.txt"),
        b"second commit bytes\n",
    )
    .unwrap();
    run_git(
        &fixture.git,
        fixture.temp.path(),
        git_args(&fixture.repo_root, &["add", "second.txt"]),
    );
    run_git(
        &fixture.git,
        fixture.temp.path(),
        git_args(
            &fixture.repo_root,
            &[
                "-c",
                "user.name=w05-retry",
                "-c",
                "user.email=w05-retry@example.test",
                "commit",
                "-m",
                "second commit for base mismatch",
            ],
        ),
    );
    let second_commit = GitObjectId::parse(
        fixture
            .git_output(&fixture.repo_root, &["rev-parse", "HEAD"])
            .trim(),
    )
    .unwrap();
    assert_ne!(second_commit, fixture.base_commit);
    // Capture after the base-changing commit: the commit moves the original
    // worktree's HEAD, which is part of `git worktree list` output, while the
    // refusal contract covers only what the refused create itself does.
    let worktrees_before = fixture.git_output(&fixture.repo_root, &["worktree", "list"]);
    let error = create_managed_checkout(
        &fixture.input_with_base(&checkout, &second_commit),
        &CancellationToken::new(),
    )
    .await
    .unwrap_err();
    assert!(matches!(error, CreateCheckoutError::InputMismatch));
    assert_eq!(managed_entries(managed.as_str()), vec![".git".to_owned()]);
    let record = fixture.record(&checkout).await.unwrap();
    assert!(matches!(
        record.current_lifecycle,
        ManagedCheckoutLifecycle::Creating
    ));
    let state = record.latest_operation.state();
    assert!(matches!(state, CheckoutOperationState::Intent));
    assert_eq!(
        fixture.git_output(&fixture.repo_root, &["worktree", "list"]),
        worktrees_before
    );
}

/// Two concurrent creates for one checkout id both succeed: one carries the
/// receipt and the other observes the completed checkout, exactly one
/// managed worktree is registered with the checkout Active, and the budget
/// ledger is not poisoned — a later create for a different id still
/// appends through the same sequencer.
#[tokio::test]
async fn concurrent_creates_for_one_id_both_succeed_once() {
    let fixture = CreateCheckoutFixture::new("w05-retry-concurrent").await;
    let checkout = ManagedCheckoutId::parse("checkout-w05-concurrent").unwrap();
    let managed = fixture.service.managed_path(&checkout).unwrap();
    let input_a = fixture.input(&checkout);
    let input_b = fixture.input(&checkout);
    let cancellation = CancellationToken::new();
    let (first, second) = tokio::join!(
        create_managed_checkout(&input_a, &cancellation),
        create_managed_checkout(&input_b, &cancellation),
    );
    let first = first.unwrap();
    let second = second.unwrap();
    assert!(first.receipt().is_some() || second.receipt().is_some());
    let listing = fixture.git_output(&fixture.repo_root, &["worktree", "list"]);
    assert_eq!(listing.matches(managed.as_str()).count(), 1);
    let record = fixture.record(&checkout).await.unwrap();
    assert!(matches!(
        record.current_lifecycle,
        ManagedCheckoutLifecycle::Active
    ));
    let later = ManagedCheckoutId::parse("checkout-w05-concurrent-later").unwrap();
    let later_created = create_managed_checkout(&fixture.input(&later), &CancellationToken::new())
        .await
        .unwrap();
    assert!(later_created.receipt().is_some());
    let later_record = fixture.record(&later).await.expect("later row");
    assert!(matches!(
        later_record.current_lifecycle,
        ManagedCheckoutLifecycle::Active
    ));
}
