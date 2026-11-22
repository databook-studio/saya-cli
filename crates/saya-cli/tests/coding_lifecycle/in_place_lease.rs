//! Which runs take the folder lease, and which read next to one.
//!
//! A job that states no write scope only reads its folder, so it takes no
//! lease and writes no lease file: its own state stays under the state root.
//! A write-scoped job still holds the one-live-job lease for its lifetime, so a
//! second write-scoped job over the same folder is refused, while a read-only
//! run over that folder opens alongside it.

use super::support::Fixture;
use saya_agent::CancellationToken;
use saya_cli::{CodingAuthorization, CodingError, CodingJobRequest, NativeCodingJob};
use saya_types::JobId;
use std::{fs, path::PathBuf, sync::Arc};

/// Creates one in-place job over the fixture's folder, `writes` stating
/// whether it takes the whole folder as its write scope.
async fn create(
    fixture: &Fixture,
    id: &str,
    writes: bool,
) -> Result<Arc<NativeCodingJob>, CodingError> {
    create_as(fixture, id, writes, None).await
}

/// The same, with the session the job is a job *of* stated: `Some(session)` is
/// the id the folder's lease names, and a job that names it is that session's
/// own child rather than an unrelated job over the same folder.
async fn create_as(
    fixture: &Fixture,
    id: &str,
    writes: bool,
    session: Option<&str>,
) -> Result<Arc<NativeCodingJob>, CodingError> {
    fixture.application.set_session_parent(session);
    let write_paths = if writes {
        vec![".".to_owned()]
    } else {
        Vec::new()
    };
    let request = CodingJobRequest::new(
        JobId::parse(id).unwrap(),
        "read the folder and answer".into(),
        write_paths,
    )
    .unwrap();
    let proposed = fixture.application.propose(&request).await.unwrap();
    let approval = CodingAuthorization::scoped(&proposed).unwrap();
    fixture
        .application
        .create(request, &approval, CancellationToken::new())
        .await
}

/// The state root every fixture job keeps its records under.
fn state_root(fixture: &Fixture) -> PathBuf {
    fixture.root.join("coding")
}

/// The lease files the state root holds. A lease folder no run created reads
/// as none.
fn lease_files(fixture: &Fixture) -> Vec<PathBuf> {
    let directory = state_root(fixture).join("inplace-leases");
    let Ok(entries) = fs::read_dir(&directory) else {
        return Vec::new();
    };
    entries
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "lock")
        })
        .collect()
}

/// The checkouts the job owns: an in-place run creates none.
async fn checkouts(fixture: &Fixture, id: &str) -> usize {
    let job = JobId::parse(id).unwrap();
    fixture
        .store
        .list_managed_checkouts(&job, None, 32)
        .await
        .unwrap()
        .items
        .len()
}

/// A run with no write scope only reads: it takes no lease, leaves no lease
/// file behind, and creates no checkout.
#[tokio::test]
async fn a_run_with_no_write_scope_takes_no_folder_lease() {
    let fixture = Fixture::in_place("lease-read-only").await;
    let job = create(&fixture, "lease-read-only", false)
        .await
        .expect("a job with no write scope is created");

    assert!(
        lease_files(&fixture).is_empty(),
        "a read-only run holds no folder lease: {:?}",
        lease_files(&fixture)
    );
    let records = state_root(&fixture).join(job.job_id().as_str());
    assert!(
        records.join("inplace-root.json").is_file(),
        "the run still records its in-place root under the state root"
    );
    assert_eq!(
        checkouts(&fixture, job.job_id().as_str()).await,
        0,
        "and creates no checkout"
    );
}

/// A write-scoped run still takes the one folder lease, and a second
/// write-scoped run over the same folder is refused.
#[tokio::test]
async fn a_write_scoped_run_holds_the_lease_and_a_second_is_refused() {
    let fixture = Fixture::in_place("lease-write-scope").await;
    let first = create(&fixture, "lease-write-scope-one", true)
        .await
        .expect("a write-scoped job is created");
    assert_eq!(
        lease_files(&fixture).len(),
        1,
        "the write-scoped run holds the folder's one lease"
    );

    let second = create(&fixture, "lease-write-scope-two", true).await;
    let Err(error) = second else {
        panic!("a second live write-scoped job over the same folder must be refused");
    };
    assert!(
        matches!(error, CodingError::InPlaceRootBusy),
        "the second write-scoped job is refused as busy: {error:?}"
    );
    assert_eq!(
        lease_files(&fixture).len(),
        1,
        "the refused job leaves the first run's one lease file"
    );
    drop(first);
}

/// A read-only run and a write-scoped run over the same folder coexist: the
/// read-only one takes no lease, so the write-scoped one keeps its own.
#[tokio::test]
async fn a_read_only_run_reads_alongside_a_write_scoped_one() {
    let fixture = Fixture::in_place("lease-coexist").await;
    let writer = create(&fixture, "lease-coexist-writer", true)
        .await
        .expect("the write-scoped job is created");
    let reader = create(&fixture, "lease-coexist-reader", false)
        .await
        .expect("a read-only job opens over the folder another run leases");

    assert_eq!(
        lease_files(&fixture).len(),
        1,
        "only the write-scoped run holds a lease file"
    );
    let records = state_root(&fixture).join(reader.job_id().as_str());
    assert!(
        records.join("inplace-root.json").is_file(),
        "the read-only run records its root under the state root, not in the folder"
    );
    assert_eq!(
        checkouts(&fixture, reader.job_id().as_str()).await,
        0,
        "and creates no checkout"
    );
    assert_eq!(
        fs::read(fixture.repository.join("tracked.txt")).unwrap(),
        b"original\n",
        "neither run wrote the folder"
    );
    drop((writer, reader));
}

/// A job a session opens over the session's own folder runs alongside the
/// session's run: the folder's lease names that session, and a job naming it is
/// admitted as its dependent — while a job that names no session is the
/// unrelated one and is refused exactly as before.
#[tokio::test]
async fn a_sessions_own_job_runs_alongside_its_run_in_the_same_folder() {
    let fixture = Fixture::in_place("lease-shared").await;
    let session = create_as(&fixture, "lease-shared", true, Some("lease-shared"))
        .await
        .expect("the session's run opens over its own folder");
    assert_eq!(
        lease_files(&fixture).len(),
        1,
        "the session's run holds the folder's one lease"
    );

    let child = create_as(&fixture, "lease-shared-child", true, Some("lease-shared"))
        .await
        .expect("a job the session opens over its own folder is admitted alongside its run");
    assert_eq!(
        lease_files(&fixture).len(),
        1,
        "the admitted job takes no second lease file: it is the session's dependent"
    );
    assert!(
        state_root(&fixture)
            .join(child.job_id().as_str())
            .join("inplace-root.json")
            .is_file(),
        "and works in the session's own folder, not in a checkout"
    );
    assert_eq!(
        checkouts(&fixture, child.job_id().as_str()).await,
        0,
        "a shared job creates no checkout"
    );

    let stranger = create_as(&fixture, "lease-shared-stranger", true, None).await;
    let Err(error) = stranger else {
        panic!("an unrelated job over a leased folder must still be refused");
    };
    assert!(
        matches!(error, CodingError::InPlaceRootBusy),
        "the unrelated job is refused as busy: {error:?}"
    );
    drop((session, child));
}
