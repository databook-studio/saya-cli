//! W05d `verify_owned_checkout` acceptance (authored, not executed): the
//! Active acceptance proof, the typed refusals, and the read-only invariant.

use std::{fs, path::Path};

use saya_agent::CancellationToken;
use saya_harness::coding::snapshot::{
    SnapshotArtifacts, SnapshotOmissionPolicy, SnapshotSelection,
};
use saya_harness::coding::worktree::{
    OwnershipError, OwnershipInput, create_managed_checkout, verify_owned_checkout,
};
use saya_harness::workspace::Workspace;
use saya_types::{JobId, ManagedCheckoutId, ManagedCheckoutLifecycle};

use super::create_support::CreateCheckoutFixture;
use super::support::{git_args, run_git};

/// One ownership input over the fixture for the given asking job.
fn ownership_input<'a>(
    fixture: &'a CreateCheckoutFixture,
    job_id: &'a JobId,
    checkout_id: &'a ManagedCheckoutId,
) -> OwnershipInput<'a> {
    OwnershipInput {
        store: &fixture.store,
        service: &fixture.service,
        repository: &fixture.repository,
        artifacts: &fixture.artifacts,
        job_id,
        checkout_id,
    }
}

/// Sorted file names directly under `root`.
fn managed_listing(root: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(root)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

/// An Active checkout verifies: the returned values equal the created path
/// and the record, and the plan digest equals the published plan digest.
#[tokio::test]
async fn verify_accepts_an_active_checkout() {
    let fixture = CreateCheckoutFixture::new("w05-own-accept").await;
    let checkout = ManagedCheckoutId::parse("checkout-w05-accept").unwrap();
    let token = CancellationToken::new();
    create_managed_checkout(&fixture.input(&checkout), &token)
        .await
        .unwrap();
    let record = fixture.record(&checkout).await.unwrap();
    let managed = fixture.service.managed_path(&checkout).unwrap();
    let input = ownership_input(&fixture, &fixture.job, &checkout);
    let verified = verify_owned_checkout(&input, &token).await.unwrap();
    assert_eq!(verified.path().as_str(), managed.as_str());
    assert_eq!(
        verified.path().as_str(),
        record.ownership.managed_path().as_str()
    );
    assert_eq!(
        verified.base_commit().as_str(),
        fixture.base_commit.as_str()
    );
    assert_eq!(
        verified.base_commit().as_str(),
        record.ownership.base_commit().as_str()
    );
    assert_eq!(
        verified.snapshot_id().as_str(),
        fixture.plan.snapshot_id().as_str()
    );
    assert_eq!(
        verified.snapshot_sha256().as_str(),
        record.ownership.snapshot_sha256().as_str()
    );
    assert_eq!(
        verified.plan_sha256().as_str(),
        fixture.plan.manifest_sha256().as_str()
    );
    assert!(verified.agent_id().is_none());
}

/// A checkout whose create failed transiently stays `Creating`, so the
/// verify refuses with `NotActive` before any identity step runs.
#[tokio::test]
async fn verify_refuses_a_creating_checkout() {
    let fixture = CreateCheckoutFixture::new("w05-own-creating").await;
    let checkout = ManagedCheckoutId::parse("checkout-w05-creating").unwrap();
    let (object, object_bytes) = fixture.corrupt_first_object();
    create_managed_checkout(&fixture.input(&checkout), &CancellationToken::new())
        .await
        .unwrap_err();
    let record = fixture.record(&checkout).await.expect("intent row");
    assert!(matches!(
        record.current_lifecycle,
        ManagedCheckoutLifecycle::Creating
    ));
    fs::write(&object, &object_bytes).unwrap();
    let input = ownership_input(&fixture, &fixture.job, &checkout);
    let error = verify_owned_checkout(&input, &CancellationToken::new())
        .await
        .unwrap_err();
    assert!(matches!(error, OwnershipError::NotActive));
}

/// A never-created checkout id refuses with `NotFound`.
#[tokio::test]
async fn verify_refuses_an_unknown_checkout_id() {
    let fixture = CreateCheckoutFixture::new("w05-own-unknown").await;
    let checkout = ManagedCheckoutId::parse("checkout-w05-unknown").unwrap();
    let input = ownership_input(&fixture, &fixture.job, &checkout);
    let error = verify_owned_checkout(&input, &CancellationToken::new())
        .await
        .unwrap_err();
    assert!(matches!(error, OwnershipError::NotFound));
}

/// Job B asking for a checkout id job A owns refuses with `WrongOwner`.
#[tokio::test]
async fn verify_refuses_another_jobs_checkout() {
    let fixture = CreateCheckoutFixture::new("w05-own-other").await;
    let checkout = ManagedCheckoutId::parse("checkout-w05-other").unwrap();
    create_managed_checkout(&fixture.input(&checkout), &CancellationToken::new())
        .await
        .unwrap();
    let job_b = JobId::parse("job-b").unwrap();
    let input = ownership_input(&fixture, &job_b, &checkout);
    let error = verify_owned_checkout(&input, &CancellationToken::new())
        .await
        .unwrap_err();
    assert!(matches!(error, OwnershipError::WrongOwner));
}

/// A managed worktree whose HEAD moved off the recorded base fails the live
/// adopt identity check, so the verify refuses with `Git(..)`.
#[tokio::test]
async fn verify_refuses_a_moved_head() {
    let fixture = CreateCheckoutFixture::new("w05-own-head").await;
    let checkout = ManagedCheckoutId::parse("checkout-w05-head").unwrap();
    let token = CancellationToken::new();
    create_managed_checkout(&fixture.input(&checkout), &token)
        .await
        .unwrap();
    let managed = fixture.service.managed_path(&checkout).unwrap();
    let root = Path::new(managed.as_str());
    run_git(
        &fixture.git,
        fixture.temp.path(),
        git_args(root, &["add", "-A"]),
    );
    run_git(
        &fixture.git,
        fixture.temp.path(),
        git_args(
            root,
            &[
                "-c",
                "user.name=saya",
                "-c",
                "user.email=saya@example.com",
                "commit",
                "-m",
                "moved",
            ],
        ),
    );
    let head = fixture.git_output(root, &["rev-parse", "HEAD"]);
    assert_ne!(head, fixture.base_commit.as_str());
    let input = ownership_input(&fixture, &fixture.job, &checkout);
    let error = verify_owned_checkout(&input, &token).await.unwrap_err();
    assert!(matches!(error, OwnershipError::Git(_)));
}

/// The published snapshot replaced by a different but internally consistent
/// capture is accepted by the strict reader, so the refusal is
/// `SnapshotMismatch`, not `Snapshot(..)` (the reader's own refusal).
#[tokio::test]
async fn verify_refuses_a_tampered_snapshot_artifact() {
    let fixture = CreateCheckoutFixture::new("w05-own-tamper").await;
    let checkout = ManagedCheckoutId::parse("checkout-w05-tamper").unwrap();
    let token = CancellationToken::new();
    create_managed_checkout(&fixture.input(&checkout), &token)
        .await
        .unwrap();
    let input = ownership_input(&fixture, &fixture.job, &checkout);
    assert!(verify_owned_checkout(&input, &token).await.is_ok());

    fs::write(fixture.repo_root.join("extra.txt"), b"extra bytes\n").unwrap();
    let policy = SnapshotOmissionPolicy::new(&fixture.repository, Vec::new()).unwrap();
    let tampered_plan = fixture
        .service
        .plan_snapshot(
            &fixture.repository,
            fixture.plan.snapshot_id().clone(),
            SnapshotSelection::CurrentWorkingContent,
            policy,
            &token,
        )
        .await
        .unwrap();
    let tampered_root = fixture.temp.path().join("artifacts-tampered");
    let tampered = SnapshotArtifacts::open(&tampered_root).unwrap();
    let workspace = Workspace::open(fixture.repository.root()).unwrap();
    fixture
        .service
        .capture_snapshot(
            &fixture.repository,
            &workspace,
            &tampered_plan,
            &tampered,
            &token,
        )
        .await
        .unwrap();
    let published = fixture.temp.path().join("artifacts");
    let snapshot_name = fixture.plan.snapshot_id().as_str();
    fs::remove_dir_all(published.join(snapshot_name)).unwrap();
    fs::rename(
        tampered_root.join(snapshot_name),
        published.join(snapshot_name),
    )
    .unwrap();

    let error = verify_owned_checkout(&input, &token).await.unwrap_err();
    assert!(matches!(error, OwnershipError::SnapshotMismatch));
}

/// A successful verify writes nothing: the managed directory listing and
/// the managed worktree's `git status --porcelain` are unchanged.
#[tokio::test]
async fn verify_leaves_the_checkout_untouched() {
    let fixture = CreateCheckoutFixture::new("w05-own-clean").await;
    let checkout = ManagedCheckoutId::parse("checkout-w05-clean").unwrap();
    let token = CancellationToken::new();
    create_managed_checkout(&fixture.input(&checkout), &token)
        .await
        .unwrap();
    let managed = fixture.service.managed_path(&checkout).unwrap();
    let root = Path::new(managed.as_str());
    let listing_before = managed_listing(root);
    let status_before = fixture.git_output(root, &["status", "--porcelain"]);
    let input = ownership_input(&fixture, &fixture.job, &checkout);
    verify_owned_checkout(&input, &token).await.unwrap();
    assert_eq!(managed_listing(root), listing_before);
    assert_eq!(
        fixture.git_output(root, &["status", "--porcelain"]),
        status_before
    );
}
