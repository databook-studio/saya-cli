//! W05c2 `create_managed_checkout` edge acceptance at the public boundary
//! (authored, not executed): privatization of an adopted permissive managed
//! directory and the job-binding preflight refusal that leaves no checkout
//! row for the submitting job with a usable job-bound sequencer, per the
//! W05C2-FIX3 repairs, plus the W05c3 cross-job checkout-id ownership
//! refusal.

use std::{
    fs,
    num::NonZeroU64,
    os::unix::fs::{MetadataExt, PermissionsExt},
    path::Path,
};

use saya_agent::CancellationToken;
use saya_harness::coding::budget::JobBudgetLedger;
use saya_harness::coding::worktree::{
    CreateCheckoutError, CreateCheckoutInput, create_managed_checkout,
};
use saya_store::NewCodingJob;
use saya_types::{
    CodingEventKind, CodingEventV1, JobId, JobState, ManagedCheckoutId, ManagedCheckoutLifecycle,
    SnapshotId,
};

use super::create_support::{CreateCheckoutFixture, limits};
use super::support::{git_args, run_git};

/// Git's `worktree add` creates the managed directory with the umask mode,
/// so the privatization repair is proved on the adoption path: a
/// pre-existing no-checkout worktree at the managed path is forced to the
/// permissive 0755 mode before the create, and after the create the managed
/// path is a real directory with mode 0700 owned by the same user that owns
/// the fixture's temp root (the process user), verified through
/// unauthenticated `symlink_metadata`.
#[tokio::test]
async fn git_created_directory_is_privatized() {
    let fixture = CreateCheckoutFixture::new("w05-create-private").await;
    let checkout = ManagedCheckoutId::parse("checkout-w05-private").unwrap();
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
    fs::set_permissions(
        Path::new(managed.as_str()),
        fs::Permissions::from_mode(0o755),
    )
    .unwrap();
    let before = fs::symlink_metadata(Path::new(managed.as_str())).unwrap();
    assert_eq!(before.permissions().mode() & 0o777, 0o755);
    let created = create_managed_checkout(&fixture.input(&checkout), &CancellationToken::new())
        .await
        .unwrap();
    assert!(created.receipt().is_some());
    let metadata = fs::symlink_metadata(Path::new(managed.as_str())).unwrap();
    assert_eq!(metadata.permissions().mode() & 0o777, 0o700);
    let root_uid = fs::symlink_metadata(fixture.temp.path()).unwrap().uid();
    assert_eq!(metadata.uid(), root_uid);
}

/// A job bound to a different snapshot than the plan this create writes is
/// refused by the binding preflight before the ledger is touched: the typed
/// mismatch error returns with no checkout row for the submitting job, and
/// a sequencer bound to that job still accepts a later append (not
/// poisoned).
#[tokio::test]
async fn job_binding_mismatch_is_refused_without_poisoning() {
    let fixture = CreateCheckoutFixture::new("w05-create-binding").await;
    let checkout = ManagedCheckoutId::parse("checkout-w05-binding").unwrap();
    let other_job = JobId::parse("job-b").unwrap();
    let identity = fixture
        .service
        .inventory_repository(&fixture.repository, &CancellationToken::new())
        .await
        .unwrap();
    fixture
        .store
        .create_coding_job(NewCodingJob {
            job_id: other_job.clone(),
            repository_id: identity.identity().repository_id().clone(),
            snapshot_id: SnapshotId::parse("w05-create-binding-other").unwrap(),
        })
        .await
        .unwrap();
    let kind = CodingEventKind::StateChanged {
        state: JobState::Running,
    };
    fixture
        .store
        .append_coding_event(
            CodingEventV1::new(
                NonZeroU64::new(1).unwrap(),
                other_job.clone(),
                None,
                None,
                kind,
            )
            .unwrap(),
        )
        .await
        .unwrap();
    let input = CreateCheckoutInput {
        job_id: &other_job,
        ..fixture.input(&checkout)
    };
    let error = create_managed_checkout(&input, &CancellationToken::new())
        .await
        .unwrap_err();
    assert!(matches!(error, CreateCheckoutError::JobBindingMismatch));
    let record = fixture
        .store
        .get_managed_checkout(&other_job, &checkout)
        .await
        .unwrap();
    assert!(record.is_none());
    let sequencer = JobBudgetLedger::new(fixture.store.clone(), other_job.clone(), limits())
        .await
        .unwrap()
        .sequencer();
    sequencer
        .append_event(|sequence| {
            let kind = CodingEventKind::StateChanged {
                state: JobState::Paused,
            };
            CodingEventV1::new(
                NonZeroU64::new(sequence).unwrap(),
                other_job.clone(),
                None,
                None,
                kind,
            )
            .unwrap()
        })
        .await
        .unwrap();
}

/// A checkout id durably owned by another job is refused before anything is
/// read or sequenced: a second job in the same store calls create with the
/// same id, repository and snapshot and gets `CheckoutIdInUse`; the
/// submitting job's ledger still accepts a later append (not poisoned),
/// while the owning job's checkout stays `Active` with its managed
/// directory untouched.
#[tokio::test]
async fn checkout_id_owned_by_another_job_is_refused_without_poisoning() {
    let fixture = CreateCheckoutFixture::new("w05-create-owner").await;
    let checkout = ManagedCheckoutId::parse("checkout-w05-owner").unwrap();
    let created = create_managed_checkout(&fixture.input(&checkout), &CancellationToken::new())
        .await
        .unwrap();
    assert!(created.receipt().is_some());
    let managed = fixture.service.managed_path(&checkout).unwrap();
    let other_job = JobId::parse("job-b").unwrap();
    let identity = fixture
        .service
        .inventory_repository(&fixture.repository, &CancellationToken::new())
        .await
        .unwrap();
    fixture
        .store
        .create_coding_job(NewCodingJob {
            job_id: other_job.clone(),
            repository_id: identity.identity().repository_id().clone(),
            snapshot_id: fixture.plan.snapshot_id().clone(),
        })
        .await
        .unwrap();
    let kind = CodingEventKind::StateChanged {
        state: JobState::Running,
    };
    fixture
        .store
        .append_coding_event(
            CodingEventV1::new(
                NonZeroU64::new(1).unwrap(),
                other_job.clone(),
                None,
                None,
                kind,
            )
            .unwrap(),
        )
        .await
        .unwrap();
    let input = CreateCheckoutInput {
        job_id: &other_job,
        ..fixture.input(&checkout)
    };
    let error = create_managed_checkout(&input, &CancellationToken::new())
        .await
        .unwrap_err();
    assert!(matches!(error, CreateCheckoutError::CheckoutIdInUse));
    let record = fixture
        .store
        .get_managed_checkout(&fixture.job, &checkout)
        .await
        .unwrap();
    assert!(matches!(
        record.unwrap().current_lifecycle,
        ManagedCheckoutLifecycle::Active
    ));
    assert!(Path::new(managed.as_str()).is_dir());
    let sequencer = JobBudgetLedger::new(fixture.store.clone(), other_job.clone(), limits())
        .await
        .unwrap()
        .sequencer();
    sequencer
        .append_event(|sequence| {
            let kind = CodingEventKind::StateChanged {
                state: JobState::Paused,
            };
            CodingEventV1::new(
                NonZeroU64::new(sequence).unwrap(),
                other_job.clone(),
                None,
                None,
                kind,
            )
            .unwrap()
        })
        .await
        .unwrap();
}
