//! `/start --checkout` — the opt-in isolation. The job works in a managed copy
//! of the repository, the session's own folder stays untouched until the job's
//! own apply, and a folder Git does not root says so in one sentence.

use super::support::Fixture;
use saya_agent::CancellationToken;
use saya_cli::{
    CodingApplication, CodingAuthorization, CodingError, CodingJobRequest, NativeCodingJob,
    RootMode,
};
use saya_types::{JobId, ManagedCheckoutId};
use std::{fs, path::Path, path::PathBuf, sync::Arc};

fn state_root(fixture: &Fixture) -> PathBuf {
    fixture.root.join("coding")
}

fn records(fixture: &Fixture, id: &str) -> PathBuf {
    state_root(fixture).join(id)
}

/// The application a `--checkout` start opens over: the checkout mode, stated as
/// the launch's own so a folder without Git is refused rather than served in
/// place.
async fn open(
    fixture: &Fixture,
    folder: &Path,
    checkout: bool,
) -> Result<Arc<CodingApplication>, CodingError> {
    CodingApplication::open_for_start(
        fixture.runtime.clone(),
        fixture.store.clone(),
        folder,
        &state_root(fixture),
        &fixture.git,
        &fixture.program_dir,
        RootMode::Managed,
        checkout,
        &CancellationToken::new(),
    )
    .await
    .map(|(application, _)| application)
}

/// One job over the whole folder, approved for exactly that scope.
async fn create(application: &Arc<CodingApplication>, id: &str) -> Arc<NativeCodingJob> {
    let request = CodingJobRequest::new(
        JobId::parse(id).unwrap(),
        "edit the folder".into(),
        vec![".".to_owned()],
    )
    .unwrap();
    let proposed = application.propose(&request).await.unwrap();
    let approval = CodingAuthorization::scoped(&proposed).unwrap();
    application
        .create(request, &approval, CancellationToken::new())
        .await
        .unwrap()
}

/// The job works in a managed checkout of the repository: its edits land there,
/// the session's folder keeps its bytes until the job's own apply, and the job
/// records no in-place root.
#[tokio::test]
async fn a_checkout_start_works_in_a_checkout_until_the_jobs_own_apply() {
    let fixture = Fixture::new("start-checkout").await;
    let application = open(&fixture, &fixture.repository, true)
        .await
        .expect("a Git working tree can be checked out");
    let job = create(&application, "start-checkout").await;
    assert!(
        !records(&fixture, "start-checkout")
            .join("inplace-root.json")
            .exists(),
        "a checkout start records no in-place root: it owns no folder"
    );

    let candidate = ManagedCheckoutId::parse("start-checkout-work").unwrap();
    job.create_checkout(&candidate)
        .await
        .expect("the job cuts a managed checkout of the repository");
    let checkouts = fixture
        .store
        .list_managed_checkouts(job.job_id(), None, 32)
        .await
        .unwrap();
    assert_eq!(
        checkouts.items.len(),
        1,
        "the job owns exactly one managed checkout"
    );
    let record = fixture
        .store
        .get_managed_checkout(job.job_id(), &candidate)
        .await
        .unwrap()
        .expect("the checkout is recorded");
    let managed = Path::new(record.ownership.managed_path().as_str());
    fs::write(managed.join("tracked.txt"), b"checked out\n").unwrap();
    assert_eq!(
        fs::read(fixture.repository.join("tracked.txt")).unwrap(),
        b"original\n",
        "the run's edit reaches only the checkout"
    );

    let review = job.review_existing(&candidate).await.unwrap();
    job.apply_review(&review, true).await.unwrap();
    assert_eq!(
        fs::read(fixture.repository.join("tracked.txt")).unwrap(),
        b"checked out\n",
        "the job's own apply is what reaches the folder"
    );
}

/// A folder Git does not root has no checkout to cut, and `--checkout` says so
/// in one sentence. The same folder with no flag is still served in place.
#[tokio::test]
async fn checkout_over_a_folder_without_git_says_so_in_one_sentence() {
    let fixture = Fixture::in_place("start-checkout-plain").await;
    let Err(error) = open(&fixture, &fixture.repository, true).await else {
        panic!("a plain folder has no checkout to cut");
    };
    assert!(
        matches!(error, CodingError::CheckoutNeedsRepository),
        "the plain folder is refused as one: {error:?}"
    );
    let sentence = error.to_string();
    assert!(
        sentence.contains("--checkout") && sentence.contains("Git"),
        "the sentence names the flag and the reason: {sentence}"
    );
    assert_eq!(
        sentence.lines().count(),
        1,
        "one sentence, not a paragraph: {sentence}"
    );

    let application = open(&fixture, &fixture.repository, false)
        .await
        .expect("the configured default still serves a plain folder in place");
    assert!(
        application.is_in_place(),
        "and it is the folder itself, not a checkout"
    );
}
