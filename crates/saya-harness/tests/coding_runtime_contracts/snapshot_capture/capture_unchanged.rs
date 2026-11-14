//! The Git capture wrapper still stores byte-identical manifest bytes: the
//! shared orchestrator must not reorder fields or add a local-mode marker to
//! a Git snapshot. The expected bytes are built here from the fixed field
//! order, never from the producer's own serializer.

use std::fs;

use saya_agent::CancellationToken;
use saya_harness::coding::{
    git::GitService,
    snapshot::{CapturedEntry, SnapshotArtifacts, SnapshotOmissionPolicy, SnapshotSelection},
};
use saya_harness::workspace::Workspace;
use saya_types::{RepositoryId, Sha256Digest, SnapshotId};
use serde::Serialize;

use super::super::support::{TempRoot, git_executable, repository, state_root};
use super::fixtures::artifacts_root;

#[derive(Serialize)]
struct ExpectedManifest<'a> {
    schema_version: u32,
    snapshot_id: &'a SnapshotId,
    repository_id: &'a RepositoryId,
    plan_manifest_sha256: &'a Sha256Digest,
    entries: &'a [CapturedEntry],
}

#[tokio::test]
async fn git_capture_stores_the_same_manifest_bytes() {
    let temp = TempRoot::new("w04-git-capture-unchanged");
    let git = git_executable();
    let root = repository(temp.path(), &git);
    fs::write(root.join("tracked.txt"), b"unchanged capture bytes\n").unwrap();
    let service = GitService::new(&git, state_root(temp.path())).unwrap();
    let cancellation = CancellationToken::new();
    let opened = service.open_repository(&root, &cancellation).await.unwrap();
    let policy = SnapshotOmissionPolicy::new(&opened, Vec::new()).unwrap();
    let manifest = service
        .plan_snapshot(
            &opened,
            SnapshotId::parse("w04-capture-unchanged").unwrap(),
            SnapshotSelection::CurrentWorkingContent,
            policy,
            &cancellation,
        )
        .await
        .unwrap();
    let artifacts = SnapshotArtifacts::open(&artifacts_root(&temp)).unwrap();
    let workspace = Workspace::open(opened.root()).unwrap();
    let captured = service
        .capture_snapshot(&opened, &workspace, &manifest, &artifacts, &cancellation)
        .await
        .unwrap();
    let expected = serde_json::to_vec(&ExpectedManifest {
        schema_version: 1,
        snapshot_id: manifest.snapshot_id(),
        repository_id: manifest.repository_id(),
        plan_manifest_sha256: manifest.manifest_sha256(),
        entries: captured.entries(),
    })
    .unwrap();
    let stored = fs::read(
        artifacts_root(&temp)
            .join(manifest.snapshot_id().as_str())
            .join("manifest.json"),
    )
    .unwrap();
    assert_eq!(stored, expected);
}
