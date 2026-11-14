//! One job baseline, two modes: the Git snapshot manifest and the in-place
//! local baseline both round-trip through `JobBaseline::from_verified_bytes`,
//! and wrapping a Git plan changes none of its bytes.

use std::{fs, path::Path};

use saya_agent::CancellationToken;
use saya_harness::coding::{
    git::GitService,
    local::{JobBaseline, LocalBaseline, LocalRoot},
    snapshot::{SnapshotManifest, SnapshotOmissionPolicy, SnapshotSelection},
};
use saya_types::SnapshotId;

use crate::support::{TempRoot, git_executable, repository, state_root};

async fn git_manifest(temp: &TempRoot) -> SnapshotManifest {
    let git = git_executable();
    let root = repository(temp.path(), &git);
    fs::write(root.join("tracked.txt"), b"baseline bytes\n").unwrap();
    let service = GitService::new(&git, state_root(temp.path())).unwrap();
    let cancellation = CancellationToken::new();
    let opened = service.open_repository(&root, &cancellation).await.unwrap();
    let policy = SnapshotOmissionPolicy::new(&opened, Vec::new()).unwrap();
    service
        .plan_snapshot(
            &opened,
            SnapshotId::parse("job-git-baseline").unwrap(),
            SnapshotSelection::CurrentWorkingContent,
            policy,
            &cancellation,
        )
        .await
        .unwrap()
}

fn local_baseline(temp: &TempRoot) -> saya_harness::coding::local::LocalBaseline {
    let root_path = temp.path().join("local-root");
    fs::create_dir(&root_path).unwrap();
    fs::write(root_path.join("notes.txt"), b"local bytes\n").unwrap();
    let state = temp.path().join("local-state");
    fs::create_dir(&state).unwrap();
    let root = LocalRoot::open(&root_path, &state).unwrap();
    LocalBaseline::edit_time(&root, SnapshotId::parse("job-local-baseline").unwrap()).unwrap()
}

fn round_trip(baseline: JobBaseline) -> JobBaseline {
    let bytes = baseline.canonical_bytes().to_vec();
    let digest = baseline.manifest_sha256().clone();
    let parsed = JobBaseline::from_verified_bytes(&bytes, &digest).unwrap();
    assert_eq!(parsed.canonical_bytes(), bytes);
    assert_eq!(parsed.manifest_sha256(), &digest);
    assert_eq!(parsed.snapshot_id(), baseline.snapshot_id());
    assert_eq!(parsed.repository_id(), baseline.repository_id());
    parsed
}

#[tokio::test]
async fn job_baseline_round_trips_both_modes() {
    let temp = TempRoot::new("w04-job-baseline-both");
    let git = round_trip(JobBaseline::Git(Box::new(git_manifest(&temp).await)));
    assert!(git.git().is_some());
    assert!(git.in_place().is_none());

    let in_place = round_trip(JobBaseline::InPlace(local_baseline(&temp)));
    assert!(in_place.in_place().is_some());
    assert!(in_place.git().is_none());
    assert!(Path::new(in_place.in_place().unwrap().root_path()).starts_with(temp.path()));
}

#[tokio::test]
async fn git_plan_bytes_unchanged() {
    let temp = TempRoot::new("w04-git-plan-bytes");
    let manifest = git_manifest(&temp).await;
    let bytes = manifest.canonical_bytes().to_vec();
    let digest = manifest.manifest_sha256().clone();
    let baseline = JobBaseline::Git(Box::new(manifest));
    assert_eq!(baseline.canonical_bytes(), bytes.as_slice());
    assert_eq!(baseline.manifest_sha256(), &digest);
    let parsed = JobBaseline::from_verified_bytes(&bytes, &digest).unwrap();
    assert_eq!(parsed.canonical_bytes(), bytes.as_slice());
    assert!(parsed.git().is_some());
}
