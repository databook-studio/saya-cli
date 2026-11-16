//! In-place jobs: a plain folder is captured as a local baseline, its durable
//! identity is the folder's local id, and no managed checkout is ever created.

use super::support::Fixture;
use saya_agent::CancellationToken;
use saya_cli::{
    CodingAuthorization, CodingCommand, CodingJobRequest, NativeCodingJob, dispatch_coding,
};
use saya_store::NewCodingJob;
use saya_types::{JobId, ManagedCheckoutId, RepositoryId, SnapshotId};
use serde_json::json;
use std::{fs, path::Path};

async fn create(fixture: &Fixture, id: &str) -> std::sync::Arc<NativeCodingJob> {
    let job = JobId::parse(id).unwrap();
    let request = CodingJobRequest::new(job, "inspect only".into(), vec![]).unwrap();
    let approval =
        CodingAuthorization::scoped(&fixture.application.propose(&request).await.unwrap()).unwrap();
    fixture
        .application
        .create(request, &approval, CancellationToken::new())
        .await
        .unwrap()
}

fn records(fixture: &Fixture, job: &JobId) -> std::path::PathBuf {
    fixture.root.join("coding").join(job.as_str())
}

#[tokio::test]
async fn in_place_create_captures_baseline() {
    let fixture = Fixture::in_place("in-place-baseline").await;
    let job = create(&fixture, "job-in-place-baseline").await;
    let directory = records(&fixture, job.job_id());
    let baseline = fs::read(directory.join("baseline.json")).unwrap();
    let root: serde_json::Value =
        serde_json::from_slice(&fs::read(directory.join("inplace-root.json")).unwrap()).unwrap();
    let record = fixture
        .store
        .get_coding_job(job.job_id())
        .await
        .unwrap()
        .expect("the job is durable");
    assert!(record.repository_id.as_str().starts_with("loc-"));
    assert_eq!(root["version"], json!(2));
    assert_eq!(root["root_id"], json!(record.repository_id.as_str()));
    assert_eq!(root["snapshot_id"], json!(record.snapshot_id.as_str()));
    assert_eq!(
        root["root_path"],
        json!(fixture.repository.canonicalize().unwrap().to_str().unwrap())
    );
    assert_eq!(root["git"], json!(false));
    assert_eq!(root["start_head"], json!(null));
    let baseline: serde_json::Value = serde_json::from_slice(&baseline).unwrap();
    assert_eq!(baseline["mode"], "in_place", "{baseline}");
    assert_eq!(baseline["snapshot_id"], json!(record.snapshot_id.as_str()));
    assert!(
        baseline.get("entries").is_none() && baseline.get("files").is_none(),
        "an edit-time marker holds no inventory: {baseline}"
    );
    assert!(!directory.join("plan.json").exists());
}

#[tokio::test]
async fn in_place_create_writes_no_worktree() {
    let fixture = Fixture::in_place("in-place-no-worktree").await;
    let job = create(&fixture, "job-in-place-no-worktree").await;
    assert!(!fixture.repository.join(".git").exists());
    let checkouts = fixture
        .store
        .list_managed_checkouts(job.job_id(), None, 32)
        .await
        .unwrap();
    assert!(checkouts.items.is_empty());
    assert!(only_expected_entries(&fixture.repository, &["tracked.txt"]));
}

#[tokio::test]
async fn read_only_in_place_creates_no_checkout() {
    let fixture = Fixture::in_place("in-place-read-only").await;
    let job = create(&fixture, "job-in-place-read-only").await;
    let checkouts = fixture
        .store
        .list_managed_checkouts(job.job_id(), None, 32)
        .await
        .unwrap();
    assert!(checkouts.items.is_empty());
    // No Git plan exists, so even an explicit checkout request is refused
    // rather than materialising the folder somewhere else.
    assert!(
        job.create_checkout(&ManagedCheckoutId::parse("work-foreign").unwrap())
            .await
            .is_err()
    );
    let checkouts = fixture
        .store
        .list_managed_checkouts(job.job_id(), None, 32)
        .await
        .unwrap();
    assert!(checkouts.items.is_empty());
}

#[tokio::test]
async fn owned_job_matches_local_id() {
    let fixture = Fixture::in_place("in-place-owned").await;
    let job = create(&fixture, "job-in-place-owned").await;
    let status = dispatch_coding(
        &fixture.application,
        CodingCommand::Show {
            id: job.job_id().as_str().into(),
        },
    )
    .await
    .unwrap();
    assert_eq!(status["job_id"], json!(job.job_id().as_str()));
    let foreign = JobId::parse("job-not-owned-here").unwrap();
    fixture
        .store
        .create_coding_job(NewCodingJob {
            job_id: foreign.clone(),
            repository_id: RepositoryId::parse("git-elsewhere").unwrap(),
            snapshot_id: SnapshotId::parse("snapshot-elsewhere").unwrap(),
        })
        .await
        .unwrap();
    assert!(fixture.application.status(&foreign).await.is_err());
}

#[tokio::test]
async fn list_filters_local_jobs() {
    let fixture = Fixture::in_place("in-place-list").await;
    let job = create(&fixture, "job-in-place-list").await;
    let foreign = JobId::parse("job-foreign-list").unwrap();
    fixture
        .store
        .create_coding_job(NewCodingJob {
            job_id: foreign.clone(),
            repository_id: RepositoryId::parse("git-elsewhere").unwrap(),
            snapshot_id: SnapshotId::parse("snapshot-elsewhere").unwrap(),
        })
        .await
        .unwrap();
    let listed = dispatch_coding(&fixture.application, CodingCommand::List { after: None })
        .await
        .unwrap();
    assert_eq!(
        listed["jobs"],
        json!([{"job_id": job.job_id().as_str(), "state": "running"}])
    );
}

fn only_expected_entries(root: &Path, expected: &[&str]) -> bool {
    let mut names = fs::read_dir(root)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect::<Vec<_>>();
    names.sort();
    let mut expected = expected
        .iter()
        .map(|name| name.to_string())
        .collect::<Vec<_>>();
    expected.sort();
    names == expected
}
